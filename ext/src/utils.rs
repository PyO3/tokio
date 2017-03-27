#![allow(non_upper_case_globals)]

use cpython::*;
use std::os::raw::c_long;
use std::ops::Deref;
use std::clone::Clone;
use std::time::Duration;
use std::thread;
use tokio_core::reactor;

use future::TokioFuture;


// tokio handle
#[doc(hidden)]
pub struct Handle {
    pub h: reactor::Handle,
}

unsafe impl Send for Handle {}

impl Handle {
    pub fn new(h: reactor::Handle) -> Handle {
        Handle{h: h}
    }
}

impl Clone for Handle {

    fn clone(&self) -> Handle {
        Handle {h: self.h.clone()}
    }
}

impl Deref for Handle {
    type Target = reactor::Handle;

    fn deref(&self) -> &reactor::Handle {
        &self.h
    }
}


#[allow(non_snake_case)]
pub struct WorkingClasses {
    pub Future: PyType,

    pub CancelledError: PyType,
    pub InvalidStateError: PyType,
    pub TimeoutError: PyType,

    pub Exception: PyType,
    pub BaseException: PyType,
    pub StopIteration: PyType,
}

lazy_static! {
    pub static ref Classes: WorkingClasses = {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let builtins = py.import("builtins").unwrap();
        let exception = PyType::extract(
            py, &builtins.get(py, "Exception").unwrap()).unwrap().into_object();

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
            }
        } else {
            WorkingClasses {
                // asyncio types
                Future: py.get_type::<TokioFuture>(),

                CancelledError: PyErr::new_type(
                    py, "tokio.CancelledError", Some(exception.clone_ref(py)), None),
                InvalidStateError: PyErr::new_type(
                    py, "tokio.InvalidStateError", Some(exception.clone_ref(py)), None),
                TimeoutError: PyErr::new_type(
                    py, "tokio,TimeoutError", Some(exception.clone_ref(py)), None),

                // general purpose types
                StopIteration: PyType::extract(
                    py, &builtins.get(py, "StopIteration").unwrap()).unwrap(),
                Exception: PyType::extract(
                    py, &builtins.get(py, "Exception").unwrap()).unwrap(),
                BaseException: PyType::extract(
                    py, &builtins.get(py, "BaseException").unwrap()).unwrap(),
            }

        }
    };
}


pub fn no_loop_exc(py: Python) -> PyErr {
    let cur = thread::current();
    PyErr::new::<exc::RuntimeError, _>(
        py,
        format!("There is no current event loop in thread {}.",
                cur.name().unwrap_or("unknown")).to_py_object(py))
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
