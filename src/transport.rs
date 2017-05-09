#![allow(unused_variables)]

use std::io;
use std::cell::{Cell, RefCell};
use std::net::SocketAddr;
use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use cpython::*;
use futures::unsync::mpsc;
use futures::{unsync, Async, AsyncSink, Stream, Future, Poll, Sink};
use bytes::{Bytes, BytesMut, BufMut};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::{Encoder, Decoder, Framed};
use tokio_core::net::TcpStream;

use ::TokioEventLoop;
use utils::{Classes, PyLogger, ToPyErr, with_py};
use addrinfo::AddrInfo;
use pybytes;
use pyfuture::PyFuture;
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

impl ToPyTuple for InitializedTransport {
    fn to_py_tuple(&self, py: Python) -> PyTuple {
        (self.transport.clone_ref(py), self.protocol.clone_ref(py)).to_py_tuple(py)
    }
}


// Transport factory
pub type TransportFactory = fn(
    &TokioEventLoop, bool, &PyObject, &Option<PyObject>, Option<PyObject>,
    TcpStream, Option<&AddrInfo>, Option<SocketAddr>,
    Option<PyFuture>) -> io::Result<InitializedTransport>;


pub struct BytesMsg {
    pub buf: buffer::PyBuffer,
    pub len: usize,
}

pub enum TcpTransportMessage {
    Bytes(BytesMsg),
    Close,
}


pub fn tcp_transport_factory<T>(
    evloop: &TokioEventLoop, server: bool,
    factory: &PyObject, ssl: &Option<PyObject>, server_hostname: Option<PyObject>,
    socket: T, addr: Option<&AddrInfo>,
    peer: Option<SocketAddr>, waiter: Option<PyFuture>) -> io::Result<InitializedTransport>

    where T: AsyncRead + AsyncWrite + AsRawFd + 'static
{
    let gil = Python::acquire_gil();
    let py = gil.python();

    let mut info: HashMap<&'static str, PyObject> = HashMap::new();

    if let (Some(ref addr), Some(peer)) = (addr, peer) {
        let sock = Socket::new_peer(py, addr, peer)?;
        info.insert("sockname", sock.getsockname(py)?.into_object());
        info.insert("peername", sock.getpeername(py)?.into_object());
        info.insert("socket", sock.clone_ref(py).into_object());
    }

    // create protocol
    let proto = factory.call(py, NoArgs, None)
        .log_error(py, "Protocol factory failure")?;

    // create py transport
    let (tx, rx) = mpsc::unbounded();

    let (tr, wrp_tr) = if let Some(ref ssl) = *ssl {
        // create SSLProtocol and wrpped transport
        let kwargs = PyDict::new(py);
        info.insert("sslcontext", ssl.clone_ref(py));
        let _ = kwargs.set_item(py, "server_side", server);
        if let Some(hostname) = server_hostname {
            let _ = kwargs.set_item(py, "server_hostname", hostname);
        }
        let ssl_proto = Classes.SSLProto.call(py, (
            evloop.clone_ref(py),
            proto.clone_ref(py), ssl.clone_ref(py), waiter), Some(&kwargs))?;

        let tr = PyTcpTransport::new(py, evloop, Sender::new(tx), &ssl_proto, info)?;
        let wrp_tr = ssl_proto.getattr(py, "_app_transport")?;
        (tr, wrp_tr)
    } else {
        // normal transport
        if let Some(waiter) = waiter {
            waiter.set(py, Ok(py.None()));
        }
        let tr = PyTcpTransport::new(py, evloop, Sender::new(tx), &proto, info)?;
        let wrp_tr = tr.clone_ref(py).into_object();
        (tr, wrp_tr)
    };

    // create transport and then call connection_made on protocol
    let transport = TcpTransport::new(socket, rx, tr.clone_ref(py));

    // handle connection lost
    let conn_err = tr.clone_ref(py);
    let conn_lost = tr.clone_ref(py);

    evloop.href().spawn(
        transport.map(move |_| {
            conn_lost.connection_lost()
        }).map_err(move |err| {
            conn_err.connection_error(err)
        })
    );

    Ok(InitializedTransport::new(wrp_tr.into_object(), proto))
}


py_class!(pub class PyTcpTransport |py| {
    data _loop: TokioEventLoop;
    data _connection_lost: PyObject;
    data _data_received: PyObject;
    data _transport: Sender<TcpTransportMessage>;
    data _drain: RefCell<Option<PyFuture>>;
    data _closing: Cell<bool>;
    data _info: HashMap<&'static str, PyObject>;

    def is_closing(&self) -> PyResult<bool> {
        Ok(self._closing(py).get())
    }

    def get_extra_info(&self, name: PyString,
                       default: Option<PyObject> = None) -> PyResult<PyObject> {
        if let Some(val) = self._info(py).get(name.to_string(py)?.as_ref()) {
            Ok(val.clone_ref(py))
        } else {
            match default {
                Some(val) => Ok(val),
                None => Ok(py.None())
            }
        }
    }

    //
    // write bytes to transport
    //
    def write(&self, data: PyObject) -> PyResult<()> {
        let data = buffer::PyBuffer::get(py, &data)?;
        let len = if let Some(slice) = data.as_slice::<u8>(py) {
            slice.len() as usize
        } else {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "data argument must be a bytes-like object"))
        };

        let _ = self._transport(py).send(
            TcpTransportMessage::Bytes(BytesMsg{buf:data, len:len}));
        Ok(())
    }

    //
    // write bytes to transport
    //
    def writelines(&self, data: PyObject) -> PyResult<()> {
        let iter = data.iter(py)?;

        for item in iter {
            let _ = self.write(py, item?)?;
        }

        Ok(())
    }

    //
    // write eof, close tx part of socket
    //
    def write_eof(&self) -> PyResult<()> {
        Ok(())
    }

    //
    // write all data to socket
    //
    def drain(&self) -> PyResult<PyFuture> {
        if let Some(ref fut) = *self._drain(py).borrow() {
            Ok(fut.clone_ref(py))
        } else {
            let fut = PyFuture::new(py, self._loop(py))?;
            *self._drain(py).borrow_mut() = Some(fut.clone_ref(py));
            Ok(fut)
        }
    }

    def pause_reading(&self) -> PyResult<()> {
        Ok(())
    }

    def resume_reading(&self) -> PyResult<()> {
        Ok(())
    }

    //
    // close transport
    //
    def close(&self) -> PyResult<PyObject> {
        if ! self._closing(py).get() {
            self._closing(py).set(true);
            let _ = self._transport(py).send(TcpTransportMessage::Close);
        }
        Ok(py.None())
    }

});

impl PyTcpTransport {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               sender: Sender<TcpTransportMessage>,
               protocol: &PyObject, info: HashMap<&'static str, PyObject>) -> PyResult<PyTcpTransport> {

        // get protocol callbacks
        let connection_made = protocol.getattr(py, "connection_made")?;
        let connection_lost = protocol.getattr(py, "connection_lost")?;
        let data_received = protocol.getattr(py, "data_received")?;

        let transport = PyTcpTransport::create_instance(
            py, evloop.clone_ref(py),
            connection_lost, data_received, sender,
            RefCell::new(None), Cell::new(false), info)?;

        // connection made
        let _ = connection_made.call(py, (transport.clone_ref(py),), None)
            .map_err(|err| {
                transport._closing(py).set(true);
                let _ = transport._transport(py).send(TcpTransportMessage::Close);
                evloop.log_error(py, err, "Protocol.connection_made error")
            });

        Ok(transport)
    }

    pub fn connection_lost(&self) {
        trace!("Protocol.connection_lost(None)");
        with_py(|py| {
            self._loop(py).with(
                py, "Protocol.connection_made error",
                |py| self._connection_lost(py).call(py, (py.None(),), None))});
    }

    pub fn connection_error(&self, err: io::Error) {
        trace!("Protocol.connection_lost({:?})", err);
        with_py(|py| {
            match err.kind() {
                io::ErrorKind::TimedOut => {
                    trace!("socket.timeout");
                    let e = Classes.SocketTimeout.call(py, NoArgs, None).unwrap();

                    self._connection_lost(py).call(py, (e,), None)
                        .into_log(py, "connection_lost error");
                },
                _ => {
                    trace!("Protocol.connection_lost(err): {:?}", err);
                    let mut e = err.to_pyerr(py);
                    self._connection_lost(py).call(py, (e.instance(py),), None)
                        .into_log(py, "connection_lost error");
                }
            }
        });
    }

    pub fn data_received(&self, bytes: Bytes) {
        with_py(|py| {
            let _ = pybytes::PyBytes::new(py, bytes)
                .map_err(|e| e.into_log(py, "can not create PyBytes"))
                .map(|bytes| self._loop(py).with(
                    py, "data_received error", |py|
                    self._data_received(py).call(py, (bytes,), None)));
        });
    }

    pub fn drained(&self) {
        match self._drain(GIL::python()).borrow_mut().take() {
            Some(fut) => with_py(|py| {let _ = fut.set(py, Ok(py.None()));}),
            None => (),
        }
    }
}


struct TcpTransport<T> {
    framed: Framed<T, TcpTransportCodec>,
    intake: unsync::mpsc::UnboundedReceiver<TcpTransportMessage>,
    transport: PyTcpTransport,

    buf: Option<BytesMsg>,
    incoming_eof: bool,
    flushed: bool,
    closing: bool,
}

impl<T> TcpTransport<T>
    where T: AsyncRead + AsyncWrite
{

    fn new(socket: T,
           intake: mpsc::UnboundedReceiver<TcpTransportMessage>,
           transport: PyTcpTransport) -> TcpTransport<T> {

        TcpTransport {
            framed: socket.framed(TcpTransportCodec),
            intake: intake,
            transport: transport,

            buf: None,
            incoming_eof: false,
            flushed: true,
            closing: false,
        }
    }
}


impl<T> Future for TcpTransport<T>
    where T: AsyncRead + AsyncWrite
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // poll for incoming data
        if !self.incoming_eof {
            loop {
                match self.framed.poll() {
                    Ok(Async::Ready(Some(bytes))) => {
                        self.transport.data_received(bytes);
                        continue
                    },
                    Ok(Async::Ready(None)) => {
                        self.incoming_eof = true;
                    },
                    Ok(Async::NotReady) => (),
                    Err(err) => return Err(err.into()),
                }
                break
            }
        }

        loop {
            let bytes = if let Some(bytes) = self.buf.take() {
                Some(bytes)
            } else {
                match self.intake.poll() {
                    Ok(Async::Ready(Some(msg))) => {
                        match msg {
                            TcpTransportMessage::Bytes(bytes) =>
                                Some(bytes),
                            TcpTransportMessage::Close => {
                                self.closing = true;
                                None
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

        // close
        if self.closing {
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
        if !src.is_empty() {
            Ok(Some(src.take().freeze()))
        } else {
            Ok(None)
        }
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
