use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use cpython::*;

use ::future;
use pyunsafe::{GIL, Handle};
use http::{Request, Version, Headers, ConnectionType};


py_class!(pub class PyRequest |py| {
    data handle: Handle;
    data connection: ConnectionType;
    data method: PyString;
    data url: Url;
    data path: PyString;
    data version: PyTuple;
    data headers: RawHeaders;
    data _content: StreamReader;
    data _match_info: RefCell<PyObject>;
    data _writer: RefCell<PyObject>;
    data _time_service: RefCell<PyObject>;

    property _method {
        get(&slf) -> PyResult<PyString> {
            Ok(slf.method(py).clone_ref(py))
        }
    }

    property method {
        get(&slf) -> PyResult<PyString> {
            Ok(slf.method(py).clone_ref(py))
        }
    }

    property path {
        get(&slf) -> PyResult<PyString> {
            Ok(slf.path(py).clone_ref(py))
        }
    }

    property rel_url {
        get(&slf) -> PyResult<Url> {
            Ok(slf.url(py).clone_ref(py))
        }
    }

    property version {
        get(&slf) -> PyResult<PyTuple> {
            Ok(slf.version(py).clone_ref(py))
        }
    }

    property headers {
        get(&slf) -> PyResult<RawHeaders> {
            Ok(slf.headers(py).clone_ref(py))
        }
    }

    property content {
        get(&slf) -> PyResult<StreamReader> {
            Ok(slf._content(py).clone_ref(py))
        }
    }

    property keep_alive {
        get(&slf) -> PyResult<bool> {
            if let &ConnectionType::KeepAlive = slf.connection(py) {
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    property match_info {
        get(&slf) -> PyResult<PyObject> {
            Ok(slf._match_info(py).borrow().clone_ref(py))
        }
        set(&slf, value: &PyObject) -> PyResult<()> {
            *slf._match_info(py).borrow_mut() = value.clone_ref(py);
            Ok(())
        }
    }

    property _writer {
        get(&slf) -> PyResult<PyObject> {
            Ok(slf._writer(py).borrow().clone_ref(py))
        }
        set(&slf, value: &PyObject) -> PyResult<()> {
            *slf._writer(py).borrow_mut() = value.clone_ref(py);
            Ok(())
        }
    }

    property time_service {
        get(&slf) -> PyResult<PyObject> {
            Ok(slf._time_service(py).borrow().clone_ref(py))
        }
        set(&slf, value: &PyObject) -> PyResult<()> {
            *slf._time_service(py).borrow_mut() = value.clone_ref(py);
            Ok(())
        }
    }

    def _prepare_hook(&self, resp: &PyObject) -> PyResult<PyObject> {
        Ok(future::done_future(py, self.handle(py).clone(), py.None())?.into_object())
    }

});


impl PyRequest {
    pub fn new(py: Python, req: Request, h: Handle) -> PyResult<PyRequest> {
        let conn = req.connection;
        let meth = PyString::new(py, req.method());
        let path = PyString::new(py, req.path());
        let url = Url::new(py, path.clone_ref(py))?;
        let version = match req.version {
            Version::Http10 => PyTuple::new(
                py, &[(1 as i32).to_py_object(py).into_object(),
                      (0 as i32).to_py_object(py).into_object()]),
            Version::Http11 => PyTuple::new(
                py, &[(1 as i32).to_py_object(py).into_object(),
                      (1 as i32).to_py_object(py).into_object()]),
        };
        let content = StreamReader::new(py, h.clone())?;
        let headers = RawHeaders::create_instance(py, req.headers)?;

        PyRequest::create_instance(
            py, h, conn, meth, url, path,
            version, headers, content,
            RefCell::new(py.None()), RefCell::new(py.None()), RefCell::new(py.None()))
    }

    pub fn content(&self) -> &StreamReader {
        self._content(GIL::python())
    }
}


py_class!(pub class StreamReader |py| {
    data _handle: Handle;
    data _eof: Cell<bool>;

    def exception(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def on_eof(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def is_eof(&self) -> PyResult<bool> {
        Ok(self._eof(py).get())
    }

    def at_eof(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def wait_eof(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def unread_data(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def readline(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def read(&self, n: i32 = -1) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def readany(&self) -> PyResult<PyObject> {
        Ok(future::done_future(py, self._handle(py).clone(), py.None())?.into_object())
        //Ok(py.None())
    }

    def readchunk(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def readexactly(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def read_nowait(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

});


impl StreamReader {

    fn new(py: Python, h: Handle) -> PyResult<StreamReader> {
        StreamReader::create_instance(py, h, Cell::new(false))
    }
    
    pub fn set_exception(&self) {
    }

    pub fn feed_eof(&self, py: Python) {
        self._eof(py).set(true)
    }

    pub fn feed_data(&self) {
    }

}


py_class!(pub class RawHeaders |py| {
    data headers: Headers;

    def get(&self, key: &PyString) -> PyResult<PyObject> {
        let key = key.to_string(py)?;
        if let Some(val) = self.headers(py).get(key.borrow()) {
            Ok(PyString::new(py, val).into_object())
        } else {
            Ok(py.None())
        }
    }

});


py_class!(pub class Url |py| {
    data path: PyString;

    property raw_path {
        get(&slf) -> PyResult<PyString> {
            Ok(slf.path(py).clone_ref(py))
        }
    }

});


impl Url {

    fn new(py: Python, path: PyString) -> PyResult<Url> {
        Url::create_instance(py, path)
    }

}
