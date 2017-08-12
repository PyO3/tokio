#![allow(unused_variables)]

use std::net::SocketAddr;
use std::os::unix::io::RawFd;

use pyo3::*;

use addrinfo::AddrInfo;
use utils::Classes;


#[py::class(weakref, freelist=50)]
pub struct Socket {
    fd: Option<RawFd>,
    family: i32,
    socktype: i32,
    proto: i32,
    sockaddr: SocketAddr,
    peername: Option<SocketAddr>,
    socket: Option<PyObject>,
    token: PyToken,
}


impl Socket {
    pub fn new(py: Python, addr: &AddrInfo) -> PyResult<Py<Socket>> {
        py.init(
            |token| Socket{
                fd: None,
                family: addr.family.to_int() as i32,
                socktype: addr.socktype.to_int() as i32,
                proto: addr.protocol.to_int() as i32,
                sockaddr: addr.sockaddr.clone(),
                peername: None,
                socket: None,
                token: token})
    }

    pub fn new_peer(py: Python, addr: &AddrInfo,
                    peer: SocketAddr, fd: Option<RawFd>) -> PyResult<Py<Socket>> {
        py.init(
            |token| Socket{
                fd: fd,
                family: addr.family.to_int() as i32,
                socktype: addr.socktype.to_int() as i32,
                proto: addr.protocol.to_int() as i32,
                sockaddr: addr.sockaddr.clone(),
                peername: Some(peer),
                socket: None,
                token: token})
    }
}


#[py::methods]
impl Socket {

    #[getter]
    fn get_family(&self) -> PyResult<i32> {
        Ok(self.family)
    }

    #[getter]
    fn get_type(&self) -> PyResult<i32> {
        Ok(self.socktype)
    }

    #[getter]
    fn get_proto(&self) -> PyResult<i32> {
        Ok(self.proto)
    }

    fn accept(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("accept method is not supported."))
    }

    fn bind(&self, py: Python, address: PyObject) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("bind method is not supported."))
    }

    fn close(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("close method is not supported."))
    }

    fn connect(&self, py: Python, address: PyObject) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("connect method is not supported."))
    }

    fn connect_ex(&self, py: Python, address: PyObject) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("connect_ex method is not supported."))
    }

    fn detach(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("detach method is not supported."))
    }

    fn dup(&mut self, py: Python) -> PyResult<PyObject> {
        if let Some(ref sock) = self.socket {
            return sock.call_method0(py, "dup")
        }

        if let Some(fd) = self.fd {
            let sock = Classes.Socket.as_ref(py).call1(
                "socket", (self.family, self.socktype, self.proto, fd))?;

            let res = sock.call_method0("dup");
            self.socket = Some(sock.into());
            Ok(res?.into())
        } else {
            Err(PyErr::new::<exc::RuntimeError, _>("dup method is not supported."))
        }
    }

    fn fileno(&self, py: Python) -> PyResult<i32> {
        Ok(-1)
    }

    fn get_inheritable(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("Method is not supported."))
    }

    pub fn getpeername(&self, py: Python) -> PyResult<PyObject> {
        match self.peername {
            None => Err(PyErr::new::<exc::OSError, _>("Socket is not connected")),
            Some(ref addr) => match addr {
                &SocketAddr::V4(addr) => {
                    Ok((format!("{}", addr.ip()), addr.port()).into_object(py))
                }
                &SocketAddr::V6(addr) => {
                    Ok((format!("{}", addr.ip()),
                        addr.port(), addr.flowinfo(), addr.scope_id(),).into_object(py))
                },
            }
        }
    }

    pub fn getsockname(&self, py: Python) -> PyResult<PyObject> {
        match self.sockaddr {
            SocketAddr::V4(ref addr) => {
                Ok((format!("{}", addr.ip()), addr.port()).into_object(py))
            }
            SocketAddr::V6(ref addr) => {
                Ok((format!("{}", addr.ip()),
                    addr.port(), addr.flowinfo(), addr.scope_id(),).into_object(py))
            },
        }
    }

    fn getsockopt(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("getsockopt method is not supported."))
    }

    fn gettimeout(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("gettimeout method is not supported."))
    }

    fn ioctl(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("ioctl method is not supported."))
    }

    fn listen(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("listen method is not supported."))
    }

    fn makefile(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("makefile method is not supported."))
    }

    fn recv(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("recv method is not supported."))
    }

    fn recvfrom(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("recvfrom method is not supported."))
    }

    fn recvmsg(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("recvmsg method is not supported."))
    }

    fn recvmsg_into(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("Method is not supported."))
    }

    fn recvfrom_into(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("Method is not supported."))
    }

    fn recv_into(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("Method is not supported."))
    }

    fn send(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("send method is not supported."))
    }

    fn sendall(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("sendall method is not supported."))
    }

    fn sendto(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("sendto method is not supported."))
    }

    fn sendmsg(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("sendmsg method is not supported."))
    }

    fn sendmsg_afalg(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("sendmsg_afalg method is not supported."))
    }

    // we need to implement this
    fn sendfile(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("sendfile method is not supported."))
    }

    fn set_inheritable(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("Method is not supported."))
    }

    fn setblocking(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("setblocking method is not supported."))
    }

    fn settimeout(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("settimeout method is not supported."))
    }

    #[args(args="*", kwargs="**")]
    fn setsockopt(&self, args: &PyTuple, kwargs: Option<&PyDict>) -> PyResult<()> {
        // Err(PyErr::new::<exc::RuntimeError, _>(py, "setsockopt method is not supported."))
        Ok(())
    }

    fn shutdown(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("shutdown method is not supported."))
    }

    fn share(&self, py: Python) -> PyResult<()> {
        Err(PyErr::new::<exc::RuntimeError, _>("share method is not supported."))
    }
}
