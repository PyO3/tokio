use std::io;
use std::net;
use cpython::*;
use futures::{future, Async, Future, Poll};
use net2::TcpBuilder;
use native_tls::TlsConnector;
use tokio_core::net::TcpStream;
use tokio_tls::TlsConnectorExt;

use ::PyFuture;
use addrinfo;
use utils::{Classes, ToPyErr, with_py};
use pyunsafe;
use transport::tcp_transport_factory;


pub fn create_connection(py: Python, factory: PyObject,
                         handle: pyunsafe::Handle, fut: PyFuture,
                         addrs: Vec<addrinfo::AddrInfo>,
                         ssl: Option<TlsConnector>, srv_hostname: String) {
    let fut_err = fut.clone_ref(py);

    let conn = Connector {
        handle: handle.clone(),
        addr_no: 0,
        addrs: addrs,
        errors: Vec::new(),
        conn: None,
    }.map_err(move |err| {
        with_py(|py| fut_err.set(py, Err(err)));
    });

    let h = handle.clone();

    match ssl {
        Some(ssl) => {
            // ssl handshake
            let fut_tls_err = fut.clone_ref(py);
            let tls_handshake = conn.and_then(move |socket| {
                ssl.connect_async(srv_hostname.as_str(), socket).map_err(move |e| {
                    with_py(|py| fut_tls_err.set(
                        py, Err(io::Error::new(io::ErrorKind::Other, e).to_pyerr(py))));
                })
            });
            // create transport
            let transport = tls_handshake.and_then(move |socket| {
                let result = tcp_transport_factory(h, &factory, socket, None)
                    .map_err(|e| with_py(|py| e.to_pyerr(py)))
                    .map(|(tr, proto)|
                         with_py(|py| (tr, proto).to_py_object(py).into_object()));
                with_py(|py| fut.set(py, result));
                future::ok(())
            });
            handle.spawn(transport);
        },
        None => {
            // normal transport
            let transport = conn.and_then(move |socket| {
                let result = tcp_transport_factory(h, &factory, socket, None)
                    .map_err(|e| with_py(|py| e.to_pyerr(py)))
                    .map(|(tr, proto)|
                         with_py(|py| (tr, proto).to_py_object(py).into_object()));
                with_py(|py| fut.set(py, result));
                future::ok(())
            });
            handle.spawn(transport);
        }
    }
}


struct Connector {
    handle: pyunsafe::Handle,
    addr_no: usize,
    addrs: Vec<addrinfo::AddrInfo>,
    errors: Vec<PyErr>,
    conn: Option<Box<Future<Item=TcpStream, Error=io::Error>>>,
}


impl Future for Connector {
    type Item = TcpStream;
    type Error = PyErr;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let mut conn = match self.conn.take() {
                Some(conn) => conn,
                None => {  // try to connect to next address
                    if self.addr_no >= self.addrs.len() {
                        // no more addresses
                        return with_py(|py| {
                            Err(PyErr::new_lazy_init(
                                Classes.OSError.clone_ref(py),
                                Some("Multiple exceptions".to_py_object(py).into_object())))
                        })
                    } else {
                        let info = &self.addrs[self.addr_no];
                        self.addr_no += 1;

                        let builder = match info.sockaddr {
                            net::SocketAddr::V4(_) =>
                                if let Ok(b) = TcpBuilder::new_v4() { b } else { continue },

                            net::SocketAddr::V6(_) => {
                                if let Ok(b) = TcpBuilder::new_v6() {
                                    let _ = b.only_v6(true);
                                    b
                                } else {
                                    continue
                                }
                            },
                        };

                        // convert to tokio TcpStream and connect
                        match builder.to_tcp_stream() {
                            Ok(stream) =>
                                TcpStream::connect_stream(
                                    stream, &info.sockaddr, &self.handle),
                            Err(err) => {
                                self.errors.push(with_py(|py| err.to_pyerr(py)));
                                continue
                            }
                        }
                    }
                }
            };

            // check if connected
            match conn.poll() {
                Ok(Async::Ready(stream)) =>
                    return Ok(Async::Ready(stream)),
                Ok(Async::NotReady) => {
                    self.conn = Some(conn);
                    return Ok(Async::NotReady)
                }
                Err(err) => {
                    self.errors.push(with_py(|py| err.to_pyerr(py)));
                    continue
                }
            }
        }
    }
}
