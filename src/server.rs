use std::io;
use std::net;
use std::os::unix;
use pyo3::*;
use futures::{unsync, Async, Stream, Future, Poll};
use net2::TcpBuilder;
use net2::unix::UnixTcpBuilderExt;
use tokio_core::net::{TcpListener, Incoming};
use tokio_uds;
use tokio_io::IoStream;

use {PyFuture, TokioEventLoop};
use addrinfo;
use pyunsafe;
use socket::Socket;
use transport::{TransportFactory, tcp_transport_factory};


pub fn create_server(py: Python, evloop: &TokioEventLoop,
                     addrs: Vec<addrinfo::AddrInfo>, backlog: i32,
                     ssl: Option<PyObject>, reuse_address: bool, reuse_port: bool,
                     proto_factory: PyObject, transport_factory: TransportFactory)
                     -> PyResult<PyObject> {

    let handle = evloop.get_handle();

    // configure sockets
    let mut listeners = Vec::new();
    let mut sockets = Vec::new();
    for info in addrs {
        let builder = match info.family {
            addrinfo::Family::Inet =>
                if let Ok(b) = TcpBuilder::new_v4() { b } else { continue },

            addrinfo::Family::Inet6 => {
                if let Ok(b) = TcpBuilder::new_v6() {
                    let _ = b.only_v6(true);
                    b
                } else {
                    continue
                }
            },
            _ => continue
        };

        let _ = builder.reuse_address(reuse_address);
        let _ = builder.reuse_port(reuse_port);

        if let Err(err) = builder.bind(info.sockaddr) {
            return Err(err.to_pyerr(py));
        }

        match builder.listen(backlog) {
            Ok(listener) => {
                match TcpListener::from_listener(listener, &info.sockaddr, &handle.h) {
                    Ok(lst) => {
                        info!("Started listening on {:?}", info.sockaddr);
                        let mut addr = info.clone();
                        addr.sockaddr = lst.local_addr().expect("should not fail");
                        let s = Socket::new(py, &addr)?;
                        sockets.push(s);
                        listeners.push((lst, addr));
                    },
                    Err(err) => return Err(err.to_pyerr(py)),
                }
            }
            Err(err) => return Err(err.to_pyerr(py)),
        }
    }

    // create tokio listeners
    let mut handles = Vec::new();
    for (listener, addr) in listeners {

        // copy sslcontext for each server
        let s = if let Some(ref ssl) = ssl {
            Some(ssl.clone_ref(py))
        } else {
            None
        };

        let (tx, rx) = unsync::oneshot::channel::<()>();
        handles.push(pyunsafe::OneshotSender::new(tx));

        Server::serve(evloop, addr, listener.incoming(),
                      transport_factory, proto_factory.clone_ref(py), s, rx);
    }

    py.init(|token| TokioServer{
        evloop: evloop.into(),
        sockets: PyTuple::new(py, &sockets[..]),
        stop_handle: Some(handles),
        token: token}).map(|ptr| ptr.into())
}


pub fn create_sock_server(py: Python, evloop: &TokioEventLoop,
                          listener: net::TcpListener, info: addrinfo::AddrInfo,
                          ssl: Option<PyObject>, proto_factory: PyObject,
                          transport_factory: TransportFactory) -> PyResult<PyObject> {

    match TcpListener::from_listener(listener, &info.sockaddr, evloop.href()) {
        Ok(lst) => {
            info!("Started listening on {:?}", info.sockaddr);
            let mut addr = info.clone();
            addr.sockaddr = lst.local_addr().expect("should not fail");
            let sock = Socket::new(py, &addr)?;

            let (tx, rx) = unsync::oneshot::channel::<()>();
            let handles = vec![pyunsafe::OneshotSender::new(tx)];

            Server::serve(evloop, addr, lst.incoming(),
                          transport_factory, proto_factory, ssl, rx);

            py.init(|token| TokioServer {
                evloop: evloop.into(),
                sockets: PyTuple::new(py, &[sock]),
                stop_handle: Some(handles),
                token: token}).map(|ptr| ptr.into())
        },
        Err(err) => Err(err.to_pyerr(py)),
    }
}


pub fn create_uds_server(py: Python, evloop: &TokioEventLoop,
                         listener: tokio_uds::UnixListener, ssl: Option<PyObject>,
                         proto_factory: PyObject) -> PyResult<PyObject> {
    info!("Started listening on {:?}", listener.local_addr().unwrap());

    let (tx, rx) = unsync::oneshot::channel::<()>();
    let handles = vec![pyunsafe::OneshotSender::new(tx)];

    UdsServer::serve(evloop, listener.incoming(), proto_factory, ssl, rx);

    py.init(|token| TokioServer{
        evloop: evloop.into(),
        sockets: PyTuple::empty(py),
        stop_handle: Some(handles),
        token: token}).map(|ptr| ptr.into())
}


#[py::class]
pub struct TokioServer {
    evloop: Py<TokioEventLoop>,
    sockets: Py<PyTuple>,
    stop_handle: Option<Vec<pyunsafe::OneshotSender<()>>>,
    token: PyToken,
}


#[py::methods]
impl TokioServer {

    #[getter]
    fn sockets(&self) -> PyResult<PyObject> {
        Ok(self.sockets.to_object(self.py()))
    }

    fn close(&mut self, py: Python) -> PyResult<PyObject> {
        if let Some(handles) = self.stop_handle.take() {
            for h in handles {
                let _ = h.send(());
            }
        }
        Ok(py.None())
    }

    fn wait_closed(&self, py: Python) -> PyResult<Py<PyFuture>> {
        PyFuture::done_fut(py, self.evloop.clone_ref(py), true.to_object(py))
    }
}


struct Server {
    evloop: Py<TokioEventLoop>,
    addr: addrinfo::AddrInfo,
    stream: Incoming,
    stop: unsync::oneshot::Receiver<()>,
    transport: TransportFactory,
    factory: PyObject,
    ssl: Option<PyObject>,
}

impl Server {

    //
    // Start accepting incoming connections
    //
    fn serve(evloop: &TokioEventLoop, addr: addrinfo::AddrInfo,
             stream: Incoming, transport: TransportFactory,
             factory: PyObject, ssl: Option<PyObject>, stop: unsync::oneshot::Receiver<()>) {

        let srv = Server { evloop: evloop.into(), addr: addr, stop: stop, stream: stream,
                           transport: transport, factory: factory, ssl: ssl};

        evloop.get_handle().spawn(
            srv.map_err(|e| {
                error!("Server error: {}", e);
            })
        );
    }
}


impl Future for Server
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.stop.poll() {
            // TokioServer is closed remotely
            Ok(Async::Ready(_)) | Err(_) => return Ok(Async::Ready(())),
            Ok(Async::NotReady) => (),
        }

        let option = self.stream.poll()?;
        match option {
            Async::Ready(Some((socket, peer))) => {
                (self.transport)(
                    self.evloop.clone_ref(pyunsafe::GIL::python()),
                    true, &self.factory, &self.ssl,
                    None, socket, Some(&self.addr), Some(peer), None)?;

                // we can not just return Async::NotReady here,
                // because self.stream is not registered within mio anymore
                // next stream.poll() will re-register io object
                self.poll()
            },
            Async::Ready(None) =>
                Ok(Async::Ready(())),
            Async::NotReady =>
                Ok(Async::NotReady),
        }
    }
}


type UdsIncoming = IoStream<(tokio_uds::UnixStream, unix::net::SocketAddr)>;

struct UdsServer {
    evloop: Py<TokioEventLoop>,
    stream: UdsIncoming,
    stop: unsync::oneshot::Receiver<()>,
    factory: PyObject,
    ssl: Option<PyObject>,
}

impl UdsServer {

    //
    // Start accepting incoming connections
    //
    fn serve(evloop: &TokioEventLoop, stream: UdsIncoming,
             factory: PyObject, ssl: Option<PyObject>, stop: unsync::oneshot::Receiver<()>) {

        let srv = UdsServer { evloop: evloop.into(), stop: stop,
                              stream: stream, factory: factory, ssl: ssl};

        evloop.get_handle().spawn(
            srv.map_err(|e| {
                error!("Server error: {}", e);
            })
        );
    }
}


impl Future for UdsServer
{
    type Item = ();
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.stop.poll() {
            // TokioServer is closed remotely
            Ok(Async::Ready(_)) | Err(_) => {
                return Ok(Async::Ready(()))
            }
            Ok(Async::NotReady) => (),
        }

        let option = self.stream.poll()?;
        match option {
            Async::Ready(Some((socket, _peer))) => {
                tcp_transport_factory(
                    self.evloop.clone_ref(pyunsafe::GIL::python()),
                    true, &self.factory, &self.ssl, None, socket, None, None, None)?;

                // we can not just return Async::NotReady here,
                // because self.stream is not registered within mio anymore
                // next stream.poll() will re-register io object
                self.poll()
            },
            Async::Ready(None) =>
                Ok(Async::Ready(())),
            Async::NotReady =>
                Ok(Async::NotReady),
        }
    }
}
