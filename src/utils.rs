#![allow(non_upper_case_globals)]

use pyo3;
use pyo3::*;
use std::os::raw::c_long;
use std::time::Duration;
use std::fmt::Write;

use pyfuture::PyFuture;
use addrinfo::LookupError;


#[allow(non_snake_case)]
pub struct WorkingClasses {
    pub Future: Py<PyType>,

    pub Asyncio: Py<PyModule>,
    pub CancelledError: Py<PyType>,
    pub InvalidStateError: Py<PyType>,
    pub TimeoutError: Py<PyType>,
    pub SSLProto: Py<PyType>,
    pub Coroutines: Py<PyModule>,
    pub UnixEvents: Py<PyModule>,

    pub Helpers: Py<PyModule>,

    pub Exception: Py<PyType>,
    pub BaseException: Py<PyType>,
    pub StopIteration: Py<PyType>,

    pub Socket: Py<PyModule>,
    pub GaiError: Py<PyType>,
    pub SocketTimeout: Py<PyType>,
    pub GetNameInfo: PyObject,

    pub Sys: Py<PyModule>,
    pub Traceback: Py<PyModule>,
    pub ExtractStack: PyObject,
}

impl WorkingClasses {
    pub fn print_stack(&self, py: Python) {
        let _ = Classes.Traceback.as_ref(py).call("print_stack", NoArgs, None);
    }
}

lazy_static! {
    pub static ref Classes: WorkingClasses = {
        let gil = Python::acquire_gil();
        let py = gil.python();
        let builtins = py.import("builtins").unwrap();
        let socket = py.import("socket").unwrap();
        let tb = py.import("traceback").unwrap();
        let asyncio = py.import("asyncio").unwrap();
        let sslproto = py.import("asyncio.sslproto").unwrap();

        WorkingClasses {
            // asyncio types
            Future: py.get_type::<PyFuture>().into(),

            Asyncio: asyncio.into(),
            CancelledError: PyType::downcast_from(
                asyncio.get("CancelledError").unwrap()).unwrap().into(),
            InvalidStateError: PyType::downcast_from(
                asyncio.get("InvalidStateError").unwrap()).unwrap().into(),
            TimeoutError: PyType::downcast_from(
                asyncio.get("TimeoutError").unwrap()).unwrap().into(),
            SSLProto: PyType::downcast_from(
                &sslproto.get("SSLProtocol").unwrap()).unwrap().into(),
            Coroutines: py.import("asyncio.coroutines").unwrap().into(),
            UnixEvents: py.import("asyncio.unix_events").unwrap().into(),

            Helpers: py.import("tokio.helpers").unwrap().into(),

            // general purpose types
            StopIteration: PyType::downcast_from(
                builtins.get("StopIteration").unwrap()).unwrap().into(),
            Exception: PyType::downcast_from(
                builtins.get("Exception").unwrap()).unwrap().into(),
            BaseException: PyType::downcast_from(
                builtins.get("BaseException").unwrap()).unwrap().into(),

            SocketTimeout: PyType::downcast_from(
                socket.get("timeout").unwrap()).unwrap().into(),
            GaiError: PyType::downcast_from(
                socket.get("gaierror").unwrap()).unwrap().into(),
            GetNameInfo: socket.get("getnameinfo").unwrap().into(),
            Socket: socket.into(),

            Sys: py.import("sys").unwrap().into(),
            Traceback: tb.into(),
            ExtractStack: tb.get("extract_stack").unwrap().into(),
        }
    };
}


// Temporarily acquire GIL
pub fn with_py<T, F>(f: F) -> T where F: FnOnce(Python) -> T {
    let gil = Python::acquire_gil();
    let py = gil.python();
    f(py)
}

pub fn iscoroutine(ob: &PyObjectRef) -> bool {
    unsafe {
        (pyo3::ffi::PyCoro_Check(ob.as_ptr()) != 0 ||
         pyo3::ffi::PyCoroWrapper_Check(ob.as_ptr()) != 0 ||
         pyo3::ffi::PyAsyncGen_Check(ob.as_ptr()) != 0 ||
         pyo3::ffi::PyIter_Check(ob.as_ptr()) != 0 ||
         pyo3::ffi::PyGen_Check(ob.as_ptr()) != 0)
    }
}


pub trait PyLogger {

    fn into_log(self, py: Python, msg: &str);

    fn log_error(self, py: Python, msg: &str) -> Self;

}

impl<T> PyLogger for PyResult<T> {

    #[inline]
    default fn into_log(self, py: Python, msg: &str) {
        match self {
            Ok(_) => (),
            Err(err) => {
                error!("{} {:?}", msg, err);
                err.print(py);
            }
        }
    }

    #[inline]
    default fn log_error(self, py: Python, msg: &str) -> Self {
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

impl<T> PyLogger for PyResult<T> where T: IntoPyPointer {

    #[inline]
    fn into_log(self, py: Python, msg: &str) {
        match self {
            Ok(ob) => py.release(ob),
            Err(err) => {
                error!("{} {:?}", msg, err);
                err.print(py);
            }
        }
    }

    #[inline]
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

    fn into_log(self, py: Python, msg: &str) {
        error!("{} {:?}", msg, self);
        self.clone_ref(py).print(py);
    }

    fn log_error(self, py: Python, msg: &str) -> Self {
        error!("{} {:?}", msg, self);
        self.clone_ref(py).print(py);
        self
    }
}


impl ToPyErr for LookupError {

    fn to_pyerr(&self, py: Python) -> PyErr {
        match self {
            &LookupError::IOError(ref err) => err.to_pyerr(py),
            &LookupError::Other(ref err_str) =>
                PyErr::new_err(py, Classes.GaiError.as_ref(py), (err_str.to_object(py),)),
            &LookupError::NulError(_) =>
                PyErr::new_err(py, Classes.GaiError.as_ref(py), ("nil pointer",)),
            &LookupError::Generic =>
                PyErr::new_err(py, Classes.GaiError.as_ref(py), ("generic error",)),
        }
    }
}


//
// Format exception
//
pub fn print_exception(py: Python, w: &mut String, err: PyErr) {
    let res = Classes.Traceback.as_ref(py).call(
        "format_exception", (err.ptype, err.pvalue, err.ptraceback), None);
    if let Ok(lines) = res {
        if let Ok(lines) = PyList::downcast_from(lines) {
            for idx in 0..lines.len() {
                let _ = write!(w, "{}", lines.get_item(idx as isize));
            }
        }
    }
}

//
// convert PyFloat or PyInt into Duration
//
pub fn parse_seconds(py: Python, name: &str, value: &PyObjectRef) -> PyResult<Option<Duration>> {
    if let Ok(f) = PyFloat::downcast_from(value) {
        let val = f.value();
        if val < 0.0 {
            Ok(None)
        } else {
            Ok(Some(Duration::new(val as u64, (val.fract() * 1_000_000_000.0) as u32)))
        }
    } else if let Ok(i) = PyLong::downcast_from(value) {
        if let Ok(val) = i.extract::<c_long>() {
            if val < 0 {
                Ok(None)
            } else {
                Ok(Some(Duration::new(val as u64, 0)))
            }
        } else {
            Ok(None)
        }
    } else {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("'{}' must be int of float type", name)))
    }
}


//
// convert PyFloat or PyInt into u64 (milliseconds)
//
pub fn parse_millis(py: Python, name: &str, value: &PyObjectRef) -> PyResult<u64> {
    if let Ok(f) = PyFloat::downcast_from(value) {
        let val = f.value();
        if val > 0.0 {
            Ok((val * 1000.0) as u64)
        } else {
            Ok(0)
        }
    } else if let Ok(i) = PyLong::downcast_from(value) {
        if let Ok(val) = i.extract::<c_long>() {
            if val < 0 {
                Ok(0)
            } else {
                Ok((val * 1000) as u64)
            }
        } else {
            Ok(0)
        }
    } else {
        Err(PyErr::new::<exc::TypeError, _>(
            py, format!("'{}' must be int of float type: {:?}", name, value.get_type())))
    }
}
