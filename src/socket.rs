#![allow(unused_variables)]
use std::net::SocketAddr;

use cpython::*;

use addrinfo::AddrInfo;


py_class!(pub class Socket |py| {
    data family: i32;
    data socktype: i32;
    data proto: i32;
    data sockaddr: SocketAddr;
    data peername: Option<SocketAddr>;
    
    property family {
        get(&slf) -> PyResult<i32> {
            Ok(*slf.family(py))
        }
    }
    property _type {
        get(&slf) -> PyResult<i32> {
            Ok(*slf.socktype(py))
        }
    }
    property proto {
        get(&slf) -> PyResult<i32> {
            Ok(*slf.proto(py))
        }
    }

    def accept(&self) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def bind(&self, address) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def close(&self) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def connect(&self, address) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def connect_ex(&self, address) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def detach(&self) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def dup(&self) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def fileno(&self) -> PyResult<i32> {
        Ok(-1)
    }

    def get_inheritable(&self) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def getpeername(&self) -> PyResult<()> {
        if let None = *self.peername(py) {
            Err(PyErr::new::<exc::OSError, _>(py, "Socket is not connected"))
        } else {
            Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
        }
    }

    def getsockname(&self) -> PyResult<PyTuple> {
        match *self.sockaddr(py) {
            SocketAddr::V4(ref addr) => {
                Ok((format!("{}", addr.ip()), addr.port()).to_py_tuple(py))
            }
            SocketAddr::V6(ref addr) => {
                Ok((format!("{}", addr.ip()),
                    addr.port(), addr.flowinfo(), addr.scope_id(),).to_py_tuple(py))
            },
        }
    }

    def getsockopt(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def gettimeout(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def ioctl(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def listen(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def makefile(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recv(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recvfrom(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recvmsg(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recvmsg_into(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recvfrom_into(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def recv_into(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def send(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def sendall(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def sendto(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def sendmsg(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def sendmsg_afalg(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    // we need to implement this
    def sendfile(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def set_inheritable(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def setblocking(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def settimeout(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def setsockopt(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def shutdown(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

    def share(&self, *args, **kwargs) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>(py, "Mthos is not supported."))
    }

});

impl Socket {
    pub fn new(py: Python, addr: &AddrInfo) -> PyResult<Socket> {
        Socket::create_instance(
            py,
            addr.family.to_int() as i32,
            addr.socktype.to_int() as i32,
            addr.protocol.to_int() as i32,
            addr.sockaddr.clone(), None)
    }

    pub fn new_peer(py: Python, addr: &AddrInfo, peer: SocketAddr) -> PyResult<Socket> {
        Socket::create_instance(
            py,
            addr.family.to_int() as i32,
            addr.socktype.to_int() as i32,
            addr.protocol.to_int() as i32,
            addr.sockaddr.clone(),
            Some(peer))
    }
}
