#![allow(unused_variables)]
#![allow(dead_code)]

use std::io;
use std::net::SocketAddr;
use std::convert::Into;
use cpython::*;
use futures::{unsync, Async, AsyncSink, Stream, Future, Poll, Sink};
use bytes::BytesMut;
use tokio_io::{AsyncRead};
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_core::net::TcpStream;

use http;
use http::pyreq;
use http::pytransport::{PyHttpTransport, PyHttpTransportMessage};
use pyunsafe::{GIL, Handle, Sender};
use utils::{PyLogger, with_py};


pub fn http_transport_factory(handle: Handle, factory: &PyObject,
                              socket: TcpStream, _peer: SocketAddr) -> Result<(), io::Error> {
    let gil = Python::acquire_gil();
    let py = gil.python();

    let (tx, rx) = unsync::mpsc::unbounded();
    let tr = PyHttpTransport::new(py, handle.clone(), Sender::new(tx), factory)?;
    let tr2 = tr.clone_ref(py);
    let tr3 = tr.clone_ref(py);

    // create internal wire transport
    let transport = HttpTransport::new(handle.clone(), socket, rx, tr,
                                       tr3.get_data_received(py).clone_ref(py));

    // start connection processing
    handle.spawn(
        transport.map(move |_| {
            tr2.connection_lost()
        }).map_err(move |err| {
            tr3.connection_error(err)
        })
    );
    Ok(())
}


struct HttpTransport {
    handle: Handle,
    framed: Framed<TcpStream, HttpTransportCodec>,
    intake: unsync::mpsc::UnboundedReceiver<PyHttpTransportMessage>,
    transport: PyHttpTransport,
    data_received: PyObject,

    buf: Option<PyBytes>,
    flushed: bool,
    closing: bool,
    req: Option<pyreq::PyRequest>,
}

impl HttpTransport {

    fn new(handle: Handle, socket: TcpStream,
           intake: unsync::mpsc::UnboundedReceiver<PyHttpTransportMessage>,
           transport: PyHttpTransport, data_received: PyObject) -> HttpTransport {

        HttpTransport {
            handle: handle,
            framed: socket.framed(HttpTransportCodec::new()),
            intake: intake,
            transport: transport,
            data_received: data_received,

            buf: None,
            flushed: false,
            closing: false,
            req: None,
        }
    }

    fn read_from_socket(&mut self) -> Poll<(), io::Error> {
        // poll for incoming data
        match self.framed.poll() {
            Ok(Async::Ready(Some(msg))) => {
                match msg {
                    http::RequestMessage::Message(msg) => {
                        with_py(|py| match pyreq::PyRequest::new(py, msg, self.handle.clone()) {
                            Err(err) => {
                                error!("{:?}", err);
                                err.clone_ref(py).print(py);
                            },
                            Ok(req) => {
                                self.req = Some(req.clone_ref(py));
                                self.data_received.call(
                                    py, PyTuple::new(py, &[req.into_object()]), None)
                                    .into_log(py, "data_received error");
                            }
                        });
                    },
                    http::RequestMessage::Body(chunk) => {
                    },
                    http::RequestMessage::Completed => {
                        if let Some(ref req) = self.req {
                            req.content().feed_eof(GIL::python());
                        }
                        self.req = None;
                    }
                }
                //self.transport.data_received(msg)?;
                self.read_from_socket()
            },
            Ok(Async::Ready(None)) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err.into()),
        }
    }
}


impl Future for HttpTransport
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.closing {
            return self.framed.close()
        }

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
                            PyHttpTransportMessage::Bytes(bytes) =>
                                Some(bytes),
                            PyHttpTransportMessage::Close(err) => {
                                trace!("Start transport closing procesdure");
                                self.closing = true;
                                return self.framed.close()
                            }
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


struct HttpTransportCodec {
    decoder: http::RequestDecoder,
}

impl HttpTransportCodec {
    fn new() -> HttpTransportCodec {
        HttpTransportCodec {
            decoder: http::RequestDecoder::new(),
        }
    }
}

impl Decoder for HttpTransportCodec {
    type Item = http::RequestMessage;
    type Error = http::Error;

    #[inline]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
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
