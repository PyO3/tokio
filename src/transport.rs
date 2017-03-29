use std::io;
use std::net::SocketAddr;
use cpython::*;
use futures::{unsync, Async, AsyncSink, Stream, Future, Poll, Sink};
use bytes::{Bytes, BytesMut};
use tokio_io::{AsyncRead};
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_core::net::TcpStream;

use utils;
use future::{TokioFuture, create_future};
use unsafepy::{Handle, Sender};


py_class!(pub class TokioTcpTransport |py| {
    data handle: Handle;
    data writer: Sender<Bytes>;

    def get_extra_info(&self, name: PyString,
                       default: Option<PyObject> = None ) -> PyResult<PyObject> {
        Ok(
            if let Some(ob) = default {
                ob
            } else {
                py.None()
            }
        )
    }

    def write(&self, data: PyBytes) -> PyResult<PyObject> {
        let bytes = Bytes::from(data.data(py));
        self.writer(py).send(bytes);
        Ok(py.None())
    }

    //def drain(&self) -> PyResult<TokioFuture> {
    //    let fut = create_future(py, self.handle(py).clone())?;
    //    fut.set_result(py, py.None())?;
    //    Ok(fut)
    //}

});


pub fn accept_connection(handle: Handle, factory: &PyObject,
                         socket: TcpStream, peer: SocketAddr) -> Result<(), io::Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();

    // create protocol
    let proto = match factory.call(py, NoArgs, None) {
        Ok(proto) => proto,
        Err(_) => {
            // TODO: log exception to loop logging facility
            return Err(io::Error::new(
                io::ErrorKind::Other, "Protocol factory failure"));
        }
    };

    // create transport and then call connection_made on protocol
    let transport = match Transport::new(py, handle.clone(), socket, proto) {
        Err(err) =>
            return Err(io::Error::new(
                io::ErrorKind::Other, format!("Python protocol error: {:?}", err))),
        Ok(transport) => {
            let res = transport.connection_made(py);
            transport
        }
    };

    handle.spawn(transport.map_err(|e| {
        println!("Error: {:?}", e);
    }));

    Ok(())
}


struct Transport {
    framed: Framed<TcpStream, TransportCodec>,
    reader: unsync::mpsc::UnboundedReceiver<Bytes>,
    protocol: PyObject,
    transport: TokioTcpTransport,
    connection_made: PyObject,
    connection_lost: PyObject,
    eof_received: PyObject,
    data_received: PyObject,

    bytes: Option<Bytes>,
    is_flushed: bool
}

impl Transport {

    fn new(py: Python, handle: Handle, socket: TcpStream, protocol: PyObject) -> PyResult<Transport> {
        let (tx, rx) = unsync::mpsc::unbounded();

        let transport = TokioTcpTransport::create_instance(py, handle, Sender::new(tx))?;
        let connection_made = protocol.getattr(py, "connection_made")?;
        let connection_lost = protocol.getattr(py, "connection_lost")?;
        let data_received = protocol.getattr(py, "data_received")?;
        let eof_received = protocol.getattr(py, "eof_received")?;

        Ok(Transport {
            framed: socket.framed(TransportCodec),
            reader: rx,
            protocol: protocol,
            transport: transport,
            connection_made: connection_made,
            connection_lost: connection_lost,
            data_received: data_received,
            eof_received: eof_received,
            bytes: None,
            is_flushed: false,
        })
    }

    fn connection_made(&self, py: Python) -> io::Result<()> {
        let res = self.connection_made.call(
            py, PyTuple::new(py, &[self.transport.clone_ref(py).into_object()]), None);

        match res {
            Err(err) => return Err(io::Error::new(
                io::ErrorKind::Other, format!("Protocol.connection_made error: {:?}", err))),
            _ => Ok(())
        }
    }

    fn read_from_socket(&mut self) -> Poll<(), io::Error> {
        // poll for incoming data
        match self.framed.poll() {
            Ok(Async::Ready(Some(bytes))) => {
                let gil = Python::acquire_gil();
                let py = gil.python();

                let b = PyBytes::new(py, &bytes).into_object();
                let res = self.data_received.call(py, PyTuple::new(py, &[b]), None);

                match res {
                    Err(err) => {
                        println!("data_received error {:?}", &err);
                        err.print(py);
                    }
                    _ => (),
                }

                Ok(Async::NotReady)
            },
            Ok(Async::Ready(None)) => {
                // Protocol.connection_lost(None)
                let gil = Python::acquire_gil();
                let py = gil.python();
                let res = self.connection_lost.call(py, PyTuple::new(py, &[py.None()]), None);

                match res {
                    Err(err) => {
                        println!("connection_lost error {:?}", &err);
                        err.print(py);
                    }
                    _ => (),
                }

                Ok(Async::Ready(()))
            },
            Ok(Async::NotReady) => {
                Ok(Async::NotReady)
            },
            Err(err) => {
                // Protocol.connection_lost(exc)
                let gil = Python::acquire_gil();
                let py = gil.python();
                let mut e = utils::os_error(py, &err);
                let res = self.connection_lost.call(py, PyTuple::new(py, &[e.instance(py)]), None);

                match res {
                    Err(err) => {
                        println!("connection_lost error {:?}", &err);
                        err.print(py);
                    }
                    _ => (),
                }

                Err(err)
            }
        }
    }
}


impl Future for Transport
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(_) = self.read_from_socket()? {
            return Ok(Async::Ready(()))
        }

        loop {
            let bytes = if let Some(bytes) = self.bytes.take() {
                Some(bytes)
            } else {
                match self.reader.poll() {
                    Ok(Async::Ready(Some(bytes))) => {
                        Some(bytes)
                    },
                    Ok(_) => None,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
                }
            };

            if let Some(bytes) = bytes {
                self.is_flushed = false;

                match self.framed.start_send(bytes) {
                    Ok(AsyncSink::NotReady(bytes)) => {
                        self.bytes = Some(bytes);
                        break
                    }
                    Ok(AsyncSink::Ready) => continue,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
                }
            } else {
                break
            }
        }

        // flush sink
        if !self.is_flushed {
            self.is_flushed = self.framed.poll_complete()?.is_ready();
        }

        Ok(Async::NotReady)
    }
}


struct TransportCodec;

impl Decoder for TransportCodec {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !src.is_empty() {
            Ok(Some(src.take().freeze()))
        } else {
            Ok(None)
        }
    }

}

impl Encoder for TransportCodec {
    type Item = Bytes;
    type Error = io::Error;

    fn encode(&mut self, msg: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.extend(msg);
        Ok(())
    }

}
