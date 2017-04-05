#![allow(non_upper_case_globals)]

use cpython::*;
use std::io;
use std::thread;
use std::os::raw::c_long;
use std::time::Duration;

use future::TokioFuture;


#[allow(non_snake_case)]
pub struct WorkingClasses {
    pub Future: PyType,

    pub CancelledError: PyType,
    pub InvalidStateError: PyType,
    pub TimeoutError: PyType,

    pub Exception: PyType,
    pub BaseException: PyType,
    pub StopIteration: PyType,
    pub TypeError: PyType,
    pub OSError: PyType,

    pub SocketTimeout: PyType,
}

lazy_static! {
    pub static ref Classes: WorkingClasses = {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let builtins = py.import("builtins").unwrap();
        let socket = py.import("socket").unwrap();
        let exception = PyType::extract(
            py, &builtins.get(py, "Exception").unwrap()).unwrap().into_object();
        let socket_timeout = PyType::extract(
            py, &socket.get(py, "timeout").unwrap()).unwrap();

        if let Ok(asyncio) = py.import("asyncio") {
            WorkingClasses {
                // asyncio types
                Future: py.get_type::<TokioFuture>(),

                CancelledError: PyType::extract(
                    py, &asyncio.get(py, "CancelledError").unwrap()).unwrap(),
                InvalidStateError: PyType::extract(
                    py, &asyncio.get(py, "InvalidStateError").unwrap()).unwrap(),
                TimeoutError: PyType::extract(
                    py, &asyncio.get(py, "TimeoutError").unwrap()).unwrap(),

                // general purpose types
                StopIteration: PyType::extract(
                    py, &builtins.get(py, "StopIteration").unwrap()).unwrap(),
                Exception: PyType::extract(
                    py, &builtins.get(py, "Exception").unwrap()).unwrap(),
                BaseException: PyType::extract(
                    py, &builtins.get(py, "BaseException").unwrap()).unwrap(),
                OSError: PyType::extract(
                    py, &builtins.get(py, "OSError").unwrap()).unwrap(),
                TypeError: PyType::extract(
                    py, &builtins.get(py, "TypeError").unwrap()).unwrap(),

                SocketTimeout: socket_timeout,
            }
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

            let cancelled = if !has_exc {
                PyErr::new_type(
                    py, "tokio.CancelledError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "CancelledError").unwrap()).unwrap()
            };

            let invalid_state = if !has_exc {
                PyErr::new_type(
                    py, "tokio.InvalidStateError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "InvalidStateError").unwrap()).unwrap()
            };

            let timeout = if !has_exc {
                PyErr::new_type(
                    py, "tokio.TimeoutError", Some(exception.clone_ref(py)), None)
            } else {
                PyType::extract(
                    py, &tokio.get(py, "TimeoutError").unwrap()).unwrap()
            };

            WorkingClasses {
                // asyncio types
                Future: py.get_type::<TokioFuture>(),
                CancelledError: cancelled,
                TimeoutError: timeout,
                InvalidStateError: invalid_state,

                // general purpose types
                StopIteration: PyType::extract(
                    py, &builtins.get(py, "StopIteration").unwrap()).unwrap(),
                Exception: PyType::extract(
                    py, &builtins.get(py, "Exception").unwrap()).unwrap(),
                BaseException: PyType::extract(
                    py, &builtins.get(py, "BaseException").unwrap()).unwrap(),
                OSError: PyType::extract(
                    py, &builtins.get(py, "IOError").unwrap()).unwrap(),
                TypeError: PyType::extract(
                    py, &builtins.get(py, "TypeError").unwrap()).unwrap(),
                SocketTimeout: socket_timeout,
            }

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

    fn log_error(self: &Self, py: Python, msg: &str);

    fn log_if_error(self: Self, py: Python, msg: &str) -> Self;

}

impl<T> PyLogger for PyResult<T> {

    fn log_error(&self, py: Python, msg: &str) {
        if let &Err(ref err) = self {
            error!("{} {:?}", msg, err);
            err.clone_ref(py).print(py);
        }
    }

    fn log_if_error(self, py: Python, msg: &str) -> Self {
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


pub fn no_loop_exc(py: Python) -> PyErr {
    let cur = thread::current();
    PyErr::new::<exc::RuntimeError, _>(
        py,
        format!("There is no current event loop in thread {}.",
                cur.name().unwrap_or("unknown")).to_py_object(py))
}


//
// Create OSError
//
pub fn os_error(py: Python, err: &io::Error) -> PyErr {
    let inst = Classes.OSError.call(
        py,
        PyTuple::new(py, &[
            err.raw_os_error().unwrap_or(0).to_py_object(py).into_object()]), None);
    match inst {
        Ok(ob) => PyErr::from_instance(py, ob),
        Err(err) => err,
    }
}


//
// Check function arguments length
//
pub fn check_min_length(py: Python, args: &PyTuple, len: usize) -> PyResult<()> {
    if args.len(py) < len {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("function takes at least {} arguments", len).to_py_object(py)))
    } else {
        Ok(())
    }
}


//
// convert PyFloat or PyInt into Duration
//
pub fn parse_seconds(py: Python, name: &str, value: PyObject) -> PyResult<Duration> {
    if let Ok(f) = PyFloat::downcast_from(py, value.clone_ref(py)) {
        let val = f.value(py);
        Ok(Duration::new(val.ceil() as u64, (val.fract() * 1_000_000.0) as u32))
    } else if let Ok(i) = PyInt::downcast_from(py, value) {
        if let Ok(val) = i.as_object().extract::<c_long>(py) {
            Ok(Duration::new((val * 1000) as u64, 0))
        } else {
            Ok(Duration::new(0, 0))
        }
    } else {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("'{}' must be int of float type", name).to_py_object(py)))
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
            py, format!("'{}' must be int of float type", name).to_py_object(py)))
    }
}
