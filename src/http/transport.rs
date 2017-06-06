use std::io;
use std::net::SocketAddr;
use std::convert::Into;
use std::collections::{VecDeque, HashMap};
use std::os::unix::io::{AsRawFd, RawFd};
use pyo3::*;
use futures::unsync::mpsc;
use futures::{Async, AsyncSink, Stream, Future, Poll, Sink};
use tokio_io::AsyncRead;
use tokio_io::codec::Framed;
use tokio_core::net::TcpStream;

use {PyFuture, PyFuturePtr, TokioEventLoop, TokioEventLoopPtr};
use addrinfo::AddrInfo;
use http::codec::{HttpTransportCodec, EncoderMessage};
use http::pytransport::{PyHttpTransport, PyHttpTransportMessage};
use socket::Socket;
use utils::PyLogger;
use pyunsafe::Sender;
use transport::InitializedTransport;


pub fn http_transport_factory(
    evloop: TokioEventLoopPtr, _server: bool, factory: &PyObject,
    _ssl: &Option<PyObject>, _server_hostname: Option<PyObject>,
    socket: TcpStream, addr: Option<&AddrInfo>,
    peer: Option<SocketAddr>, _waiter: Option<PyFuturePtr>) -> io::Result<InitializedTransport>
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
    let proto = factory.call(py, NoArgs, None).log_error(py, "Protocol factory failure")?;

    let (tx, rx) = mpsc::unbounded();
    let tr = PyHttpTransport::new(py, ev, Sender::new(tx), &proto, py.None())?;
    let tr2 = tr.clone_ref(py).into();
    let tr3 = tr.clone_ref(py).into();

    // create internal wire transport
    let transport = HttpTransport::new(socket, rx, tr.clone_ref(py));

    // start connection processing
    ev.href().spawn(
        transport.map(move |_| {
            tr2.connection_lost()
        }).map_err(move |err| {
            tr3.connection_error(err)
        })
    );
    Ok(InitializedTransport::new(tr.into(), proto))
}


struct HttpTransport {
    framed: Framed<TcpStream, HttpTransportCodec>,
    intake: mpsc::UnboundedReceiver<PyHttpTransportMessage>,
    transport: PyHttpTransportPtr,

    buf: Option<EncoderMessage>,
    streams: VecDeque<mpsc::UnboundedReceiver<EncoderMessage>>,
    incoming_eof: bool,
    flushed: bool,
    closing: bool,
}

impl HttpTransport {

    fn new(socket: TcpStream,
           intake: mpsc::UnboundedReceiver<PyHttpTransportMessage>,
           transport: PyHttpTransport) -> HttpTransport {

        HttpTransport {
            framed: socket.framed(HttpTransportCodec::new()),
            intake: intake,
            transport: transport,

            buf: None,
            streams: VecDeque::new(),
            incoming_eof: false,
            flushed: false,
            closing: false,
        }
    }
}


impl Future for HttpTransport
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // poll for incoming data
        if !self.incoming_eof {
            loop {
                match self.framed.poll() {
                    Ok(Async::Ready(Some(msg))) => {
                        if let Some(recv) = self.transport.data_received(msg)? {
                            self.streams.push_back(recv);
                        }
                        continue
                    },
                    Ok(Async::Ready(None)) => {
                        // TODO send eof_received to pytransport
                        // self.incoming_eof = true;
                        return Ok(Async::Ready(()))
                    },
                    Ok(Async::NotReady) => (),
                    Err(err) => return Err(err.into()),
                }
                break
            }
        }

        // process outgoing data
        'sink: loop {
            if let Some(msg) = self.buf.take() {
                self.flushed = false;

                let enc_msg = match self.framed.start_send(msg) {
                    Ok(AsyncSink::NotReady(bytes)) => {
                        Some(bytes)
                    },
                    Ok(AsyncSink::Ready) => None,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
                };
                // unprocessed data
                if let Some(msg) = enc_msg {
                    self.buf = Some(msg);
                    break
                }
            }

            // poll streams
            'streams: loop {
                match self.streams.front_mut() {
                    Some(ref mut stream) => {
                        match stream.poll() {
                            Ok(Async::Ready(Some(msg))) => {     // data available, try to send
                                self.buf = Some(msg);
                                continue 'sink
                            },
                            Ok(Async::Ready(None)) => (),        // stream is empty
                            Ok(Async::NotReady) => break 'sink,  // no data available
                            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
                        }
                    }
                    None => break 'sink,
                }
                // this can happen only if stream is empty
                let _ = self.streams.pop_front();
            }
        }

        // commands from transport
        match self.intake.poll() {
            Ok(Async::Ready(Some(msg))) => {
                match msg {
                    PyHttpTransportMessage::Close(_) => {
                        trace!("Start transport closing procesdure");
                        self.closing = true;
                    }
                }
            },
            Ok(_) => (),
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Closed")),
        }

        // close
        if self.closing {
            return self.framed.close()
        }

        // flush sink
        if !self.flushed {
            self.flushed = self.framed.poll_complete()?.is_ready();
        }

        Ok(Async::NotReady)
    }
}
