use std::io;
use std::cell::RefCell;
use cpython::*;
use futures::{unsync, Async, Stream, Future, Poll};
use net2::TcpBuilder;
use net2::unix::UnixTcpBuilderExt;
use tokio_core::net::{TcpListener, Incoming};

use ::{PyFuture, TokioEventLoop};
use addrinfo;
use utils::ToPyErr;
use pyunsafe;
use transport::TransportFactory;


pub fn create_server(py: Python, evloop: TokioEventLoop,
                     addrs: Vec<addrinfo::AddrInfo>, backlog: i32, _ssl: Option<PyObject>,
                     reuse_address: bool, reuse_port: bool,
                     proto_factory: PyObject, transport_factory: TransportFactory)
                     -> PyResult<PyObject> {

    let handle = evloop.get_handle();

    // configure sockets
    let mut listeners = Vec::new();
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
                        listeners.push(lst);
                    },
                    Err(err) => return Err(err.to_pyerr(py)),
                }
            }
            Err(err) => return Err(err.to_pyerr(py)),
        }
    }

    // create tokio listeners
    let mut handles = Vec::new();
    for listener in listeners {
        let (tx, rx) = unsync::oneshot::channel::<()>();
        handles.push(pyunsafe::OneshotSender::new(tx));
        Server::serve(evloop.clone_ref(py), listener.incoming(),
                      transport_factory, proto_factory.clone_ref(py), rx);
    }

    let srv = TokioServer::create_instance(
        py, evloop.clone_ref(py), RefCell::new(Some(handles)))?;

    Ok(srv.into_object())
}


py_class!(pub class TokioServer |py| {
    data _loop: TokioEventLoop;
    data stop_handle: RefCell<Option<Vec<pyunsafe::OneshotSender<()>>>>;

    def close(&self) -> PyResult<PyObject> {
        let handles = self.stop_handle(py).borrow_mut().take();
        if let Some(handles) = handles {
            for h in handles {
                let _ = h.send(());
            }
        }
        Ok(py.None())
    }

    def wait_closed(&self) -> PyResult<PyFuture> {
        let fut = PyFuture::new(py, self._loop(py))?;
        fut.set_result(py, true.to_py_object(py).into_object())?;
        Ok(fut)
    }

});


struct Server {
    evloop: TokioEventLoop,
    stream: Incoming,
    stop: unsync::oneshot::Receiver<()>,
    transport: TransportFactory,
    factory: PyObject,
}

impl Server {

    //
    // Start accepting incoming connections
    //
    fn serve(evloop: TokioEventLoop, stream: Incoming, transport: TransportFactory,
             factory: PyObject, stop: unsync::oneshot::Receiver<()>) {

        let srv = Server { evloop: evloop, stop: stop, stream: stream,
                           transport: transport, factory: factory };

        let handle = srv.evloop.get_handle();
        handle.spawn(
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
                (self.transport)(&self.evloop, &self.factory, socket, Some(peer))?;

                // we can not just return Async::NotReady here,
                // because self.stream is not registered within mio anymore
                // next stream.poll() will re-arm future
                self.poll()
            },
            Async::Ready(None) => Ok(Async::Ready(())),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
