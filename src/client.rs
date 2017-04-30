use std::io;
use std::net;
use cpython::*;
use futures::{future, Future};
use net2::TcpBuilder;
use native_tls::TlsConnector;
use tokio_core::net::TcpStream;
use tokio_tls::TlsConnectorExt;

use addrinfo;
use pyunsafe::Handle;
use transport::{InitializedTransport, tcp_transport_factory};
use fut::{for_each, Until, UntilError};


pub fn create_connection(factory: PyObject, handle: Handle, addrs: Vec<addrinfo::AddrInfo>,
                         ssl: Option<TlsConnector>, hostname: String)
                         -> Box<Future<Item=InitializedTransport, Error=io::Error>> {

    let conn = connect(addrs, handle.clone());

    match ssl {
        Some(ssl) => {
            // ssl handshake
            let transport = conn.and_then(move |socket| {
                ssl.connect_async(hostname.as_str(), socket)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
            }).and_then(move |socket| tcp_transport_factory(handle, &factory, socket, None));

            Box::new(transport)
        },
        None => {
            let transport = conn.and_then(
                move |socket| tcp_transport_factory(handle, &factory, socket, None));
            Box::new(transport)
        }
    }
}


fn connect(addrs: Vec<addrinfo::AddrInfo>, handle: Handle)
           -> Box<Future<Item=TcpStream, Error=io::Error>>
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

        // convert to tokio TcpStream and connect
        match builder.to_tcp_stream() {
            Ok(stream) =>
                future::Either::B(
                    TcpStream::connect_stream(stream, &info.sockaddr, &handle)
                        .then(|res| match res {
                            Ok(conn) => future::ok(Some(conn)),
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
                io::ErrorKind::Other, "Can not connect to host"),
            _ => unreachable!(),
        }
    });

    Box::new(fut)
}
