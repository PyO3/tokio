use std::cell;
use std::net::SocketAddr;
use cpython::*;
use futures::future::*;
use tokio_core::net::TcpStream;

use utils::{Classes, Handle};

pub fn accept_connection(handle: Handle, protocol: PyObject,
                         socket: TcpStream, peer: SocketAddr) {
    println!("Protocol created: {:?}", protocol);
}


py_class!(pub class TokioTcpTrasnport |py| {

    def test(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

});
