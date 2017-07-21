// Copyright (c) 2017-present PyO3 Project and Contributors

use std::io;
use std::net::SocketAddr;
use std::collections::HashMap;
use std::os::unix::io::AsRawFd;

use pyo3::*;
use futures::unsync::mpsc;
use futures::{unsync, Async, AsyncSink, Stream, Future, Poll, Sink};
use bytes::{Bytes, BytesMut, BufMut};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_core::net::TcpStream;

use {PyFuture, TokioEventLoop};
use utils::{Classes, PyLogger};
use addrinfo::AddrInfo;
use pybytes;
use pyunsafe::{GIL, Sender};
use socket::Socket;

#[derive(Debug)]
pub struct InitializedTransport {
    pub transport: PyObject,
    pub protocol: PyObject,
}

impl InitializedTransport {
    pub fn new(transport: PyObject, protocol: PyObject) -> InitializedTransport {
        InitializedTransport {
            transport: transport,
            protocol: protocol,
        }
    }
}

impl IntoPyTuple for InitializedTransport {
    fn into_tuple(self, py: Python) -> Py<PyTuple> {
        (self.transport.clone_ref(py), self.protocol.clone_ref(py)).into_tuple(py)
    }
}


// Transport factory
pub type TransportFactory = fn(
    Py<TokioEventLoop>, bool, &PyObject, &Option<PyObject>, Option<PyObject>,
    TcpStream, Option<&AddrInfo>, Option<SocketAddr>,
    Option<Py<PyFuture>>) -> io::Result<InitializedTransport>;

pub struct BytesMsg {
    pub buf: buffer::PyBuffer,
    pub len: usize,
}

pub enum TcpTransportMessage {
    Bytes(BytesMsg),
    Pause,
    Resume,
    Close,
    Shutdown,
}


pub fn tcp_transport_factory<T>(
    evloop: Py<TokioEventLoop>, server: bool,
    factory: &PyObject, ssl: &Option<PyObject>, server_hostname: Option<PyObject>,
    socket: T, addr: Option<&AddrInfo>,
    peer: Option<SocketAddr>, waiter: Option<Py<PyFuture>>) -> io::Result<InitializedTransport>

    where T: AsyncRead + AsyncWrite + AsRawFd + 'static
{
    let gil = Python::acquire_gil();
    let py = gil.python();

    let ev = evloop.as_ref(py);
    let mut info: HashMap<&'static str, PyObject> = HashMap::new();

    if let (Some(ref addr), Some(peer)) = (addr, peer) {
        let sock = Socket::new_peer(py, addr, peer, Some(socket.as_raw_fd()))?;
        let sock_ref = sock.as_ref(py);
        info.insert("sockname", sock_ref.getsockname(py)?.into());
        info.insert("peername", sock_ref.getpeername(py)?.into());
        info.insert("socket", sock.clone_ref(py).into());
    }

    // create protocol
    let proto = factory.as_ref(py).call(NoArgs, None)
        .log_error(py, "Protocol factory failure")?;

    // create py transport
    let (tx, rx) = mpsc::unbounded();

    let (tr, wrp_tr): (_, PyObject) = if let Some(ref ssl) = *ssl {
        // create SSLProtocol and wrpped transport
        let kwargs = PyDict::new(py);
        info.insert("sslcontext", ssl.clone_ref(py));
        let _ = kwargs.set_item("server_side", server);
        if let Some(hostname) = server_hostname {
            let _ = kwargs.set_item("server_hostname", hostname);
        }
        let ssl_proto = Classes.SSLProto.as_ref(py).call(
            (evloop.clone_ref(py), proto, ssl.clone_ref(py), waiter),
            Some(&kwargs))?;

        let tr = PyTcpTransportPtr::new(py, ev, Sender::new(tx), &ssl_proto, info)?;
        let wrp_tr = ssl_proto.getattr("_app_transport")?;
        (tr, wrp_tr.into())
    } else {
        // normal transport
        if let Some(waiter) = waiter {
            waiter.as_mut(py).set(py, Ok(py.None()));
        }
        let tr = PyTcpTransportPtr::new(py, ev, Sender::new(tx), proto, info)?;
        let wrp_tr = tr.0.clone_ref(py).into();
        (tr, wrp_tr)
    };

    // create transport and then call connection_made on protocol
    let transport = TcpTransport::new(socket, rx, tr.clone_ref(py));

    // handle connection lost
    let conn_err = tr.clone_ref(py);
    let conn_lost = tr.clone_ref(py);

    ev.href().spawn(
        transport.map(move |_| {
            conn_lost.connection_lost()
        }).map_err(move |err| {
            conn_err.connection_error(err)
        })
    );

    Ok(InitializedTransport::new(wrp_tr.into(), proto.into()))
}


#[py::class(freelist=100)]
pub struct PyTcpTransport {
    evloop: Py<TokioEventLoop>,
    connection_lost: PyObject,
    data_received: PyObject,
    transport: Sender<TcpTransportMessage>,
    drain: Option<Py<PyFuture>>,
    drained: bool,
    closing: bool,
    info: HashMap<&'static str, PyObject>,
    paused: bool,
    token: PyToken,
}

pub struct PyTcpTransportPtr(Py<PyTcpTransport>);


#[py::methods]
impl PyTcpTransport {

    fn is_closing(&self) -> PyResult<bool> {
        Ok(self.closing)
    }

    fn get_extra_info(&self, py: Python, name: &str, default: Option<PyObject>)
                      -> PyResult<PyObject> {
        if self.closing {
            return match default {
                Some(val) => Ok(val),
                None => Ok(py.None())
            };
        }

        if let Some(val) = self.info.get(name) {
            Ok(val.clone_ref(py))
        } else {
            match default {
                Some(val) => Ok(val),
                None => Ok(py.None())
            }
        }
    }

    ///
    /// write bytes to transport
    ///
    fn write(&mut self, py: Python, data: &PyObjectRef) -> PyResult<()> {
        let data = buffer::PyBuffer::get(py, data)?;
        let len = if let Some(slice) = data.as_slice::<u8>(py) {
            slice.len() as usize
        } else {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "data argument must be a bytes-like object"))
        };

        self.drained = false;
        let _ = self.transport.send(
            TcpTransportMessage::Bytes(BytesMsg{buf:data, len:len}));
        Ok(())
    }

    ///
    /// write bytes to transport
    ///
    fn writelines(&mut self, py: Python, data: &PyObjectRef) -> PyResult<()> {
        let iter = data.iter()?;

        for item in iter {
            let _ = self.write(py, item?)?;
        }

        Ok(())
    }

    ///
    /// write eof, close tx part of socket
    ///
    fn write_eof(&self) -> PyResult<()> {
        Ok(())
    }

    ///
    /// write all data to socket
    ///
    fn drain(&mut self, py: Python) -> PyResult<Py<PyFuture>> {
        if self.drained {
            Ok(PyFuture::done_fut(py, self.evloop.clone_ref(py), py.None())?)
        } else {
            if let Some(ref fut) = self.drain {
                Ok(fut.clone_ref(py))
            } else {
                let fut = PyFuture::new(py, self.evloop.clone_ref(py))?;
                self.drain = Some(fut.clone_ref(py));
                Ok(fut)
            }
        }
    }

    fn pause_reading(&mut self) -> PyResult<()> {
        self.paused = true;
        let _ = self.transport.send(TcpTransportMessage::Pause);
        Ok(())
    }

    fn resume_reading(&mut self) -> PyResult<()> {
        self.paused = false;
        let _ = self.transport.send(TcpTransportMessage::Resume);
        Ok(())
    }

    ///
    /// close transport
    ///
    fn close(&mut self) -> PyResult<()> {
        if !self.closing {
            self.closing = true;
            let _ = self.transport.send(TcpTransportMessage::Close);
        }
        Ok(())
    }

    ///
    /// abort transport
    ///
    fn abort(&mut self) -> PyResult<()> {
        self.closing = true;
        let _ = self.transport.send(TcpTransportMessage::Shutdown);
        Ok(())
    }
}

impl PyTcpTransportPtr {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               sender: Sender<TcpTransportMessage>,
               protocol: &PyObjectRef, info: HashMap<&'static str, PyObject>)
               -> PyResult<PyTcpTransportPtr>
    {
        // get protocol callbacks
        let connection_made = protocol.getattr("connection_made")?;
        let connection_lost = protocol.getattr("connection_lost")?;
        let data_received = protocol.getattr("data_received")?;

        let transport = py.init(|token| PyTcpTransport {
            evloop: evloop.into(),
            connection_lost: connection_lost.into(),
            data_received: data_received.into(),
            transport: sender,
            drain: None,
            drained: true,
            closing: false,
            info: info,
            paused: false,
            token: token})?;

        // connection made
        let _ = connection_made.call((transport.clone_ref(py),), None)
            .map_err(|err| {
                transport.as_mut(py).closing = true;
                let _ = transport.as_mut(py).transport.send(TcpTransportMessage::Close);
                evloop.log_error(err, "Protocol.connection_made error")
            });

        Ok(PyTcpTransportPtr(transport))
    }

    pub fn clone_ref(&self, py: Python) -> PyTcpTransportPtr {
        PyTcpTransportPtr(self.0.clone_ref(py))
    }

    pub fn connection_lost(&self) {
        trace!("Protocol.connection_lost(None)");
        self.0.with(|py, transport| {
            transport.evloop.as_ref(py).with(
                "Protocol.connection_made error",
                || transport.connection_lost.call(py, (py.None(),), None))});
    }

    pub fn connection_error(&self, err: io::Error) {
        trace!("Protocol.connection_lost({:?})", err);
        self.0.with_mut(|py, tr| {
            match err.kind() {
                io::ErrorKind::TimedOut => {
                    trace!("socket.timeout");
                    let e = Classes.SocketTimeout.as_ref(py).call(NoArgs, None).unwrap();

                    tr.connection_lost.call(py, (e,), None)
                        .into_log(py, "connection_lost error");
                },
                _ => {
                    trace!("Protocol.connection_lost(err): {:?}", err);
                    let mut e = err.to_pyerr(py);
                    tr.connection_lost.call(py, (e.instance(py),), None)
                        .into_log(py, "connection_lost error");
                }
            }
        });
    }

    pub fn data_received(&self, bytes: Bytes) -> bool {
        self.0.with(|py, tr| {
            tr.evloop.as_ref(py).with(
                "data_received error", || {
                    let bytes = pybytes::PyBytes::new(py, bytes)?;
                    // let bytes = PyBytes::new(py, bytes.as_ref());
                    tr.data_received.call(py, (bytes,), None)
                        .log_error(py, "data_received error")
                });
            !tr.paused
        })
    }

    pub fn drained(&self) {
        self.0.with_mut(|py, tr| {
            tr.drained = true;
            match tr.drain.take() {
                Some(fut) => {
                    let _ = fut.as_mut(py).set(py, Ok(py.None()));
                },
                None => (),
            }
        })
    }
}


#[derive(Copy, Clone, PartialEq, Debug)]
enum TransportState {
    Normal,
    Paused,
    Closing,
    Closed,
}

struct TcpTransport<T> {
    framed: Framed<T, TcpTransportCodec>,
    intake: unsync::mpsc::UnboundedReceiver<TcpTransportMessage>,
    transport: PyTcpTransportPtr,

    buf: Option<BytesMsg>,
    incoming_eof: bool,
    flushed: bool,
    state: TransportState,
}

impl<T> TcpTransport<T>
    where T: AsyncRead + AsyncWrite + AsRawFd
{

    fn new(socket: T,
           intake: mpsc::UnboundedReceiver<TcpTransportMessage>,
           transport: PyTcpTransportPtr) -> TcpTransport<T> {

        TcpTransport {
            framed: socket.framed(TcpTransportCodec),
            intake: intake,
            transport: transport,

            buf: None,
            incoming_eof: false,
            flushed: true,
            state: TransportState::Normal,
        }
    }
}


impl<T> Future for TcpTransport<T>
    where T: AsyncRead + AsyncWrite
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let bytes = if let Some(bytes) = self.buf.take() {
                Some(bytes)
            } else {
                match self.intake.poll() {
                    Ok(Async::Ready(Some(msg))) => {
                        match msg {
                            TcpTransportMessage::Bytes(bytes) => {
                                Some(bytes)
                            },
                            TcpTransportMessage::Pause => {
                                match self.state {
                                    TransportState::Normal => {
                                        self.state = TransportState::Paused;
                                    }
                                    _ => (),
                                }
                                return self.poll()
                            },
                            TcpTransportMessage::Resume => {
                                match self.state {
                                    TransportState::Paused => {
                                        self.state = TransportState::Normal;
                                    }
                                    _ => (),
                                }
                                return self.poll()
                            },
                            TcpTransportMessage::Close => {
                                match self.state {
                                    TransportState::Normal | TransportState::Paused =>
                                        self.state = TransportState::Closing,
                                    _ => (),
                                }
                                None
                            }
                            TcpTransportMessage::Shutdown => {
                                self.state = TransportState::Closed;
                                let _ = self.framed.get_mut().shutdown();
                                return Ok(Async::Ready(()))
                            }
                        }
                    }
                    Ok(_) => None,
                    Err(_) => {
                        return Err(io::Error::new(io::ErrorKind::Other, "Closed"));
                    }
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
            if self.flushed {
                self.transport.drained();
            }
        }

        // poll for incoming data
        if !self.incoming_eof && self.state != TransportState::Paused {
            loop {
                match self.framed.poll() {
                    Ok(Async::Ready(Some(bytes))) => {
                        if ! self.transport.data_received(bytes) {
                            self.state = TransportState::Paused;
                            break
                        }
                        continue
                    },
                    Ok(Async::Ready(None)) => self.incoming_eof = true,
                    Ok(Async::NotReady) => (),
                    Err(err) => return Err(err.into()),
                }
                break
            }
        }

        // close
        if self.state == TransportState::Closing {
            if self.incoming_eof {
                return Ok(Async::Ready(()))
            }
            return self.framed.close();
        }

        if self.flushed && self.incoming_eof {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }
}


struct TcpTransportCodec;

impl Decoder for TcpTransportCodec {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let res = if !src.is_empty() {
            Ok(Some(src.take().freeze()))
        } else {
            Ok(None)
        };
        if src.capacity() <= 1024 {
            src.reserve(32768);
        }
        res
    }
}

impl Encoder for TcpTransportCodec {
    type Item = BytesMsg;
    type Error = io::Error;

    fn encode(&mut self, msg: BytesMsg, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(msg.len);
        {
            let mut slice = unsafe { dst.bytes_mut() };
            msg.buf.copy_to_slice(GIL::python(), &mut slice[..msg.len])?;
        }
        unsafe {
            let new_len = dst.len() + msg.len;
            dst.set_len(new_len);
        }

        Ok(())
    }
}
