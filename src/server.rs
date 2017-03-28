use std::io;
use std::cell::RefCell;
use std::net::SocketAddr;
use cpython::*;
use futures::sync::oneshot;
use futures::{Async, Stream, Future, Poll};
use net2::TcpBuilder;
use net2::unix::UnixTcpBuilderExt;
use tokio_core::net::{TcpListener, TcpStream, Incoming};

use addrinfo;
use future;
use server;
use utils;
use transport;


pub fn create_server(py: Python, factory: PyObject, handle: utils::Handle,
                     host: Option<String>, port: Option<u16>,
                     family: i32, flags: i32, sock: Option<PyObject>,
                     backlog: i32, ssl: Option<PyObject>,
                     reuse_address: bool, reuse_port: bool) -> PyResult<TokioServer> {

    let lookup = match addrinfo::lookup_addrinfo(
            &host.unwrap(), port.unwrap_or(0), family, flags, addrinfo::SocketType::Stream) {
        Ok(lookup) => lookup,
        Err(_) => return Err(PyErr::new_lazy_init(
            utils::Classes.OSError.clone_ref(py), None))
    };

    // configure sockets
    let mut listeners = Vec::new();
    for info in lookup {
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
            return Err(utils::os_error(py, err));
        }

        match builder.listen(backlog) {
            Ok(listener) => {
                match TcpListener::from_listener(listener, &info.sockaddr, &handle.h) {
                    Ok(lst) => {
                        info!("Started listening on {:?}", info.sockaddr);
                        listeners.push(lst);
                    },
                    Err(err) => return Err(utils::os_error(py, err)),
                }
            }
            Err(err) => return Err(utils::os_error(py, err)),
        }
    }

    // create tokio listeners
    let mut handles = Vec::new();
    for listener in listeners {
        let (tx, rx) = oneshot::channel::<()>();
        handles.push(tx);
        Server::serve(handle.clone(), listener.incoming(), factory.clone_ref(py), rx);
    }

    TokioServer::create_instance(py, handle, RefCell::new(Some(handles)))
}


py_class!(pub class TokioServer |py| {
    data handle: utils::Handle;
    data stop_handles: RefCell<Option<Vec<oneshot::Sender<()>>>>;

    def close(&self) -> PyResult<PyObject> {
        let handles = self.stop_handles(py).borrow_mut().take();
        if let Some(handles) = handles {
            for h in handles {
                let _ = h.send(());
            }
        }
        Ok(py.None())
    }

    def wait_closed(&self) -> PyResult<future::TokioFuture> {
        let fut = future::create_future(py, self.handle(py).clone())?;
        fut.set_result(py, true.to_py_object(py).into_object())?;
        Ok(fut)
    }

});


struct Server {
    stream: Incoming,
    stop: oneshot::Receiver<()>,
    factory: PyObject,
    handle: utils::Handle,
}

impl Server {

    //
    // Start accepting incoming connections
    //
    fn serve(handle: utils::Handle, stream: Incoming,
             factory: PyObject, stop: oneshot::Receiver<()>) {

        let h = handle.clone();
        let srv = Server { stop: stop, stream: stream, factory: factory, handle: handle };

        srv.handle.clone().spawn(
            srv.for_each(move |(socket, peer, proto)| {
                transport::accept_connection(h.clone(), proto, socket, peer);

                Ok(())
            }).map_err(|e| {
                error!("Server error: {}", e);
            })
        );
    }
}


impl Stream for Server
{
    type Item = (TcpStream, SocketAddr, PyObject);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.stop.poll() {
            Ok(Async::Ready(_)) | Err(_) => {
                // We are done
                Ok(Async::Ready(None))
            },
            Ok(Async::NotReady) => {
                let option = self.stream.poll()?;
                match option {
                    Async::Ready(Some((socket, peer))) => {
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        let proto = match self.factory.call(py, NoArgs, None) {
                            Ok(proto) => proto,
                            Err(_) => {
                                // TODO: log exception to loop logging facility
                                return Err(
                                    io::Error::new(
                                        io::ErrorKind::Other, "Protocol factory failure"));
                            }
                        };

                        Ok(Async::Ready(Some((socket, peer, proto))))
                    },
                    Async::Ready(None) => Ok(Async::Ready(None)),
                    Async::NotReady => Ok(Async::NotReady),
                }
            },
        }
    }
}
