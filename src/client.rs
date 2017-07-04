use std::io;
use std::net;
use pyo3::*;
use futures::{future, Future};
use net2::TcpBuilder;
use tokio_core::net::TcpStream;

use {PyFut, PyFuture, TokioEventLoop};
use addrinfo::AddrInfo;
use fut::{for_each, Until, UntilError};
use pyunsafe::{GIL, Handle};
use transport::{InitializedTransport, tcp_transport_factory};


pub fn create_sock_connection(
    factory: PyObject, evloop: Py<TokioEventLoop>,
    stream: TcpStream, addr: AddrInfo,
    ssl: Option<PyObject>, hostname: Option<PyObject>, waiter: Py<PyFuture>)
    -> Box<Future<Item=InitializedTransport, Error=io::Error>>
{
    let peer = stream.peer_addr().expect("should never happen");

    let result = tcp_transport_factory(
        evloop, false, &factory, &ssl,
        hostname, stream, Some(&addr), Some(peer), Some(waiter.clone_ref(GIL::python())));

    let waiter: PyFut = waiter.into();
    Box::new(
        waiter.then(move |_| match result {
            Ok(transport) => future::ok(transport),
            Err(err) => future::err(err)
        }))
}

pub fn create_connection(
    factory: PyObject, evloop: Py<TokioEventLoop>, addrs: Vec<AddrInfo>,
    ssl: Option<PyObject>, hostname: Option<PyObject>, waiter: Py<PyFuture>)
    -> Box<Future<Item=InitializedTransport, Error=io::Error>>
{
    let handle = evloop.as_ref(GIL::python()).get_handle();
    let conn = connect(addrs, handle.clone());

    let transport = conn.and_then(
        move |(socket, addr)| {
            let peer = socket.peer_addr().expect("should never happen");
            let result = tcp_transport_factory(
                evloop, false, &factory, &ssl, hostname,
                socket, Some(&addr), Some(peer), Some(waiter.clone_ref(GIL::python())));

            let waiter: PyFut = waiter.into();
            waiter.then(move |_| match result {
                Ok(transport) => future::ok(transport),
                Err(err) => future::err(err)
            })
        });
    Box::new(transport)
}

fn connect(addrs: Vec<AddrInfo>, handle: Handle)
           -> Box<Future<Item=(TcpStream, AddrInfo), Error=io::Error>>
{
    let fut = for_each(addrs).until::<_, _, _, ()>(move |info| {
        let builder = match info.sockaddr {
            net::SocketAddr::V4(_) =>
                if let Ok(b) = TcpBuilder::new_v4() {
                    b
                } else {
                    return future::Either::A(future::ok(None))
                },
            net::SocketAddr::V6(_) => {
                if let Ok(b) = TcpBuilder::new_v6() {
                    let _ = b.only_v6(true);
                    b
                } else {
                    return future::Either::A(future::ok(None))
                }
            },
        };

        let info: AddrInfo = info.clone();

        // convert to tokio TcpStream and connect
        match builder.to_tcp_stream() {
            Ok(stream) =>
                future::Either::B(
                    TcpStream::connect_stream(stream, &info.sockaddr, &handle)
                        .then(|res| match res {
                            Ok(conn) => future::ok(Some((conn, info))),
                            Err(_) => future::ok(None)
                        })
                ),
            Err(_err) => {
                //self.errors.push(with_py(|py| err.to_pyerr(py)));
                future::Either::A(future::ok(None))
            }
        }
    }).map_err(|e| {
        match e {
            UntilError::NoResult => io::Error::new(
                io::ErrorKind::ConnectionRefused, "Can not connect to host"),
            _ => unreachable!(),
        }
    });

    Box::new(fut)
}
