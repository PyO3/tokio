#![allow(unused_variables)]
#![allow(dead_code)]

use std::io;
use std::net::SocketAddr;
use cpython::*;
use futures::{unsync, Async, AsyncSink, Stream, Future, Poll, Sink};
use bytes::BytesMut;
use tokio_io::{AsyncRead};
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_core::net::TcpStream;

use utils::{Classes, PyLogger, ToPyErr, with_py};
use pybytes::{TokioBytes, create_bytes};
use pyunsafe::{GIL, Handle, Sender};


pub enum HttpTransportMessage {
    Bytes(PyBytes),
    Close,
}


py_class!(pub class TokioHttpTransport |py| {
    data handle: Handle;
    data transport: Sender<HttpTransportMessage>;

    def get_extra_info(&self, _name: PyString,
                       default: Option<PyObject> = None ) -> PyResult<PyObject> {
        Ok(
            if let Some(ob) = default {
                ob
            } else {
                py.None()
            }
        )
    }

    //
    // write bytes to transport
    //
    def write(&self, data: PyBytes) -> PyResult<PyObject> {
        //let bytes = Bytes::from(data.data(py));
        let _ = self.transport(py).send(HttpTransportMessage::Bytes(data));
        Ok(py.None())
    }

    //def drain(&self) -> PyResult<TokioFuture> {
    //    let fut = create_future(py, self.handle(py).clone())?;
    //    fut.set_result(py, py.None())?;
    //    Ok(fut)
    //}

    //
    // close transport
    //
    def close(&self) -> PyResult<PyObject> {
        let _ = self.transport(py).send(HttpTransportMessage::Close);
        Ok(py.None())
    }

});


pub fn http_transport_factory(handle: Handle, factory: &PyObject,
                              socket: TcpStream, _peer: SocketAddr) -> Result<(), io::Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();

    // create protocol
    let proto = factory.call(py, NoArgs, None)
        .log_error(py, "Protocol factory error")?;

    // get protocol callbacks
    let connection_made = proto.getattr(py, "connection_made")?;
    let connection_lost = proto.getattr(py, "connection_lost")?;
    let connection_err = connection_lost.clone_ref(py);
    let data_received = proto.getattr(py, "data_received")?;

    // create internal wire transport
    let transport = HttpTransport::new(py, handle.clone(), socket, data_received)
        .log_error(py, "WireTransport error")?;

    // connection made
    connection_made.call(
        py, PyTuple::new(
            py, &[transport.transport.clone_ref(py).into_object()]), None)
        .log_error(py, "Protocol.connection_made error")?;

    // start connection processing
    handle.spawn(
        transport.map(move |_| {
            trace!("Protocol.connection_lost(None)");
            with_py(|py| {
                let _ = connection_lost.call(py, PyTuple::new(py, &[py.None()]), None)
                    .log_error(py, "connection_lost error");
            });
        }).map_err(move |err| {
            match err.kind() {
                io::ErrorKind::TimedOut => {
                    trace!("socket.timeout");
                    with_py(|py| {
                        let e = Classes.SocketTimeout.call(
                            py, NoArgs, None).unwrap();

                        connection_err.call(py, PyTuple::new(py, &[e]), None)
                            .into_log(py, "connection_lost error");
                    });
                },
                _ => {
                    trace!("Protocol.connection_lost(err): {:?}", err);
                    with_py(|py| {
                        let mut e = err.to_pyerr(py);
                        connection_err.call(py, PyTuple::new(py, &[e.instance(py)]), None)
                            .into_log(py, "connection_lost error");
                    });
                }
            }
        })
    );
    Ok(())
}


struct HttpTransport {
    framed: Framed<TcpStream, HttpTransportCodec>,
    intake: unsync::mpsc::UnboundedReceiver<HttpTransportMessage>,

    transport: TokioHttpTransport,
    data_received: PyObject,

    buf: Option<PyBytes>,
    flushed: bool,
    closing: bool,
}

impl HttpTransport {

    fn new(py: Python, handle: Handle, socket: TcpStream, data_received: PyObject) -> PyResult<HttpTransport> {
        let (tx, rx) = unsync::mpsc::unbounded();

        let transport = TokioHttpTransport::create_instance(py, handle, Sender::new(tx))?;

        Ok(HttpTransport {
            framed: socket.framed(HttpTransportCodec),
            intake: rx,

            transport: transport,
            data_received: data_received,

            buf: None,
            flushed: false,
            closing: false,
        })
    }

    fn read_from_socket(&mut self) -> Poll<(), io::Error> {
        // poll for incoming data
        match self.framed.poll() {
            Ok(Async::Ready(Some(bytes))) => {
                with_py(|py| {
                    trace!("Data recv: {}", bytes.__len__(py).unwrap());
                    self.data_received.call(py, PyTuple::new(py, &[bytes.into_object()]),
                                            None)
                        .into_log(py, "data_received error");
                });

                self.read_from_socket()
            },
            Ok(Async::Ready(None)) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err),
        }
    }
}


impl Future for HttpTransport
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(_) = self.read_from_socket()? {
            return Ok(Async::Ready(()))
        }

        loop {
            let bytes = if let Some(bytes) = self.buf.take() {
                Some(bytes)
            } else {
                match self.intake.poll() {
                    Ok(Async::Ready(Some(msg))) => {
                        match msg {
                            HttpTransportMessage::Bytes(bytes) =>
                                Some(bytes),
                            HttpTransportMessage::Close =>
                                return Ok(Async::Ready(())),
                        }
                    }
                    Ok(_) => None,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
                }
            };

            if let Some(bytes) = bytes {
                self.flushed = false;

                match self.framed.start_send(bytes) {
                    Ok(AsyncSink::NotReady(bytes)) => {
                        self.buf = Some(bytes);
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
        if !self.flushed {
            self.flushed = self.framed.poll_complete()?.is_ready();
        }

        Ok(Async::NotReady)
    }
}


struct HttpTransportCodec;

impl Decoder for HttpTransportCodec {
    type Item = TokioBytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !src.is_empty() {
            let bytes = src.take().freeze();

            let gil = Python::acquire_gil();
            let py = gil.python();

            match create_bytes(py, bytes) {
                Ok(bytes) =>
                    Ok(Some(bytes)),
                Err(_) =>
                    Err(io::Error::new(
                        io::ErrorKind::Other, "Can not create TokioBytes instance")),
            }
        } else {
            Ok(None)
        }
    }

}

impl Encoder for HttpTransportCodec {
    type Item = PyBytes;
    type Error = io::Error;

    fn encode(&mut self, msg: PyBytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.extend(msg.data(GIL::python()));
        Ok(())
    }

}
