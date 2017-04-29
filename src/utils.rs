#![allow(non_upper_case_globals)]

use cpython::*;
use std::io;
use std::os::raw::c_long;
use std::time::Duration;
use std::error::Error;

use pyfuture::PyFuture;
use addrinfo::LookupError;


#[allow(non_snake_case)]
pub struct WorkingClasses {
    pub Future: PyType,

    pub CancelledError: PyType,
    pub InvalidStateError: PyType,
    pub TimeoutError: PyType,

    pub Exception: PyType,
    pub BaseException: PyType,
    pub StopIteration: PyType,

    pub SocketTimeout: PyType,

    pub BrokenPipeError: PyType,
    pub ConnectionAbortedError: PyType,
    pub ConnectionRefusedError: PyType,
    pub ConnectionResetError: PyType,
    pub InterruptedError: PyType,
}

lazy_static! {
    pub static ref Classes: WorkingClasses = {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let builtins = py.import("builtins").unwrap();
        let socket = py.import("socket").unwrap();
        let exception = PyType::extract(
            py, &builtins.get(py, "Exception").unwrap()).unwrap().into_object();

        // for py2.7
        let ioerror = PyType::extract(
            py, &builtins.get(py, "IOError").unwrap()).unwrap().into_object();

        let cancelled;
        let invalid_state;
        let timeout;
        if let Ok(asyncio) = py.import("asyncio") {
            cancelled =  PyType::extract(
                py, &asyncio.get(py, "CancelledError").unwrap()).unwrap();
            invalid_state = PyType::extract(
                py, &asyncio.get(py, "InvalidStateError").unwrap()).unwrap();
            timeout = PyType::extract(
                py, &asyncio.get(py, "TimeoutError").unwrap()).unwrap();
        } else {
            let tokio = if let Ok(tokio) = PyModule::new(py, "tokio") {
                tokio
            } else {
                PyModule::import(py, "tokio").unwrap()
            };

            let has_exc = if let Ok(_) = tokio.get(py, "CancelledError") {
                true
            } else {
                false
            };

            cancelled = if !has_exc {
                PyErr::new_type(
                    py, "tokio.CancelledError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "CancelledError").unwrap()).unwrap()
            };

            invalid_state = if !has_exc {
                PyErr::new_type(
                    py, "tokio.InvalidStateError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "InvalidStateError").unwrap()).unwrap()
            };

            timeout = if !has_exc {
                PyErr::new_type(
                    py, "tokio.TimeoutError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "TimeoutError").unwrap()).unwrap()
            };
        }

        WorkingClasses {
            // asyncio types
            Future: py.get_type::<PyFuture>(),

            CancelledError: cancelled,
            InvalidStateError: invalid_state,
            TimeoutError: timeout,

            // general purpose types
            StopIteration: PyType::extract(
                py, &builtins.get(py, "StopIteration").unwrap()).unwrap(),
            Exception: PyType::extract(
                py, &builtins.get(py, "Exception").unwrap()).unwrap(),
            BaseException: PyType::extract(
                py, &builtins.get(py, "BaseException").unwrap()).unwrap(),

            BrokenPipeError: PyType::extract(
                py, &builtins.get(py, "BrokenPipeError").unwrap_or(
                    ioerror.clone_ref(py))).unwrap(),
            ConnectionAbortedError: PyType::extract(
                py, &builtins.get(py, "ConnectionAbortedError").unwrap_or(
                    ioerror.clone_ref(py))).unwrap(),
            ConnectionRefusedError: PyType::extract(
                py, &builtins.get(py, "ConnectionRefusedError").unwrap_or(
                    ioerror.clone_ref(py))).unwrap(),
            ConnectionResetError: PyType::extract(
                py, &builtins.get(py, "ConnectionResetError").unwrap_or(
                    ioerror.clone_ref(py))).unwrap(),
            InterruptedError: PyType::extract(
                py, &builtins.get(py, "InterruptedError").unwrap_or(
                    ioerror.clone_ref(py))).unwrap(),

            SocketTimeout: PyType::extract(
                py, &socket.get(py, "timeout").unwrap()).unwrap(),
        }
    };
}


// Temporarily acquire GIL
pub fn with_py<T, F>(f: F) -> T where F: FnOnce(Python) -> T {
    let gil = Python::acquire_gil();
    let py = gil.python();
    f(py)
}


pub trait PyLogger {

    fn into_log(&self, py: Python, msg: &str);

    fn log_error(self, py: Python, msg: &str) -> Self;

}

impl<T> PyLogger for PyResult<T> {

    fn into_log(&self, py: Python, msg: &str) {
        if let &Err(ref err) = self {
            error!("{} {:?}", msg, err);
            err.clone_ref(py).print(py);
        }
    }

    fn log_error(self, py: Python, msg: &str) -> Self {
        match &self {
            &Err(ref err) => {
                error!("{} {:?}", msg, err);
                err.clone_ref(py).print(py);
            }
            _ => (),
        }
        self
    }
}


impl PyLogger for PyErr {

    fn into_log(&self, py: Python, msg: &str) {
        error!("{} {:?}", msg, self);
        self.clone_ref(py).print(py);
    }

    fn log_error(self, py: Python, msg: &str) -> Self {
        error!("{} {:?}", msg, self);
        self.clone_ref(py).print(py);
        self
    }
}


/// Converts into PyErr
pub trait ToPyErr {

    fn to_pyerr(&self, Python) -> PyErr;

}

/// Create OSError from io::Error
impl ToPyErr for io::Error {

    fn to_pyerr(&self, py: Python) -> PyErr {
        let tp;
        let exc_type = match self.kind() {
            io::ErrorKind::BrokenPipe => &Classes.BrokenPipeError,
            io::ErrorKind::ConnectionRefused => &Classes.ConnectionRefusedError,
            io::ErrorKind::ConnectionAborted => &Classes.ConnectionAbortedError,
            io::ErrorKind::ConnectionReset => &Classes.ConnectionResetError,
            io::ErrorKind::Interrupted => &Classes.InterruptedError,
            _ => {
                tp = py.get_type::<exc::OSError>();
                &tp
            }
        };

        let errno = self.raw_os_error().unwrap_or(0);
        let errdesc = self.description();

        let err = exc_type.call(
            py,
            PyTuple::new(py, &[errno.to_py_object(py).into_object(),
                               errdesc.to_py_object(py).into_object()]), None);

        match err {
            Ok(err) => PyErr::from_instance(py, err),
            Err(err) => err
        }
    }

}


impl ToPyErr for LookupError {

    fn to_pyerr(&self, py: Python) -> PyErr {
        match self {
            &LookupError::IOError(ref err) => err.to_pyerr(py),
            &LookupError::Other(ref err_str) =>
                PyErr::new::<exc::RuntimeError, _>(py, err_str),
            &LookupError::NulError(_) =>
                PyErr::new::<exc::RuntimeError, _>(py, "nil pointer"),
            &LookupError::Generic =>
                PyErr::new::<exc::RuntimeError, _>(py, "generic error"),
        }
    }
}


//
// convert PyFloat or PyInt into Duration
//
pub fn parse_seconds(py: Python, name: &str, value: PyObject) -> PyResult<Duration> {
    if let Ok(f) = PyFloat::downcast_from(py, value.clone_ref(py)) {
        let val = f.value(py);
        Ok(Duration::new(val as u64, (val.fract() * 1_000_000_000.0) as u32))
    } else if let Ok(i) = PyInt::downcast_from(py, value) {
        if let Ok(val) = i.as_object().extract::<c_long>(py) {
            Ok(Duration::new((val * 1000) as u64, 0))
        } else {
            Ok(Duration::new(0, 0))
        }
    } else {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("'{}' must be int of float type", name)))
    }
}


//
// convert PyFloat or PyInt into u64 (milliseconds)
//
pub fn parse_millis(py: Python, name: &str, value: PyObject) -> PyResult<u64> {
    if let Ok(f) = PyFloat::downcast_from(py, value.clone_ref(py)) {
        Ok((f.value(py) * 1000.0) as u64)
    } else if let Ok(i) = PyInt::downcast_from(py, value) {
        if let Ok(val) = i.as_object().extract::<c_long>(py) {
            Ok((val * 1000) as u64)
        } else {
            Ok(0)
        }
    } else {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("'{}' must be int of float type", name)))
    }
}
