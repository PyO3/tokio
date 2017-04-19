use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use cpython::*;
use bytes::BytesMut;

use ::PyFuture;
use pyunsafe::{GIL, Handle, Sender};
use http::codec::EncoderMessage;
use http::{Request, Version, Headers, ConnectionType, ContentCompression};


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
    data _writer: PayloadWriter;
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
        get(&slf) -> PyResult<PayloadWriter> {
            Ok(slf._writer(py).clone_ref(py))
        }
    }
    property writer {
        get(&slf) -> PyResult<PayloadWriter> {
            Ok(slf._writer(py).clone_ref(py))
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
        Ok(PyFuture::done_fut(py, self.handle(py).clone(), py.None())?.into_object())
    }

});


impl PyRequest {
    pub fn new(py: Python, req: Request,
               h: Handle, sender: Sender<EncoderMessage>) -> PyResult<PyRequest> {
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
        let writer = PayloadWriter::new(py, h.clone(), sender)?;

        PyRequest::create_instance(
            py, h, conn, meth, url, path,
            version, headers, content,
            RefCell::new(py.None()), writer, RefCell::new(py.None()))
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
        Ok(PyFuture::done_fut(py, self._handle(py).clone(), py.None())?.into_object())
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


const SEP: &'static [u8] = b": ";
const END: &'static [u8] = b"\r\n";


py_class!(pub class PayloadWriter |py| {
    data _loop: Handle;
    data _sender: RefCell<Option<Sender<EncoderMessage>>>;
    data _length: Cell<u64>;
    data _chunked: Cell<bool>;
    data _compress: Cell<ContentCompression>;

    property length {
        get(&slf) -> PyResult<u64> {
            Ok(slf._length(py).get())
        }
        set(&slf, value: u64) -> PyResult<()> {
            slf._length(py).set(value);
            Ok(())
        }
    }
    property output_size {
        get(&slf) -> PyResult<u64> {
            Ok(slf._length(py).get())
        }
    }

    def enable_chunking(&self) -> PyResult<PyObject> {
        self._chunked(py).set(true);
        Ok(py.None())
    }

    def enable_compression(&self, encoding: Option<PyString>) -> PyResult<PyObject> {
        let enc = if let Some(encoding) = encoding {
            let enc = encoding.to_string(py)?;
            if enc == "deflate" {
                ContentCompression::Deflate
            } else if enc == "gzip" {
                ContentCompression::Gzip
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(py, py.None()));
            }
        } else {
            ContentCompression::Deflate
        };
        self._compress(py).set(enc);

        Ok(py.None())
    }

    def write(&self, chunk: PyBytes, drain: bool = true) -> PyResult<PyObject> {
        self.send_maybe(py, EncoderMessage::PyBytes(chunk));
        Ok(PyFuture::done_fut(py, self._loop(py).clone(), py.None())?.into_object())
    }

    // Build Request message from status line and headers object
    // status_line - string with \r\n
    // headers = dict like object
    def write_headers(&self, status_line: &PyString, headers: &PyObject) -> PyResult<PyObject> {
        let mut buf = BytesMut::with_capacity(2048);

        buf.extend(status_line.to_string(py)?.as_bytes());

        let items = headers.call_method(py, "items", NoArgs, None)?;
        let items = items.call_method(py, "__iter__", NoArgs, None)?;
        let mut iter = PyIterator::from_object(py, items)?;
        loop {
            if let Some(item) = iter.next() {
                let item = PyTuple::downcast_from(py, item?)?;
                if item.len(py) < 2 {
                    return Err(PyErr::new::<exc::ValueError, _>(py, py.None()));
                }
                let key = PyString::downcast_from(py, item.get_item(py, 0))?;
                buf.extend(key.to_string(py)?.as_bytes());
                buf.extend(SEP);

                let value = PyString::downcast_from(py, item.get_item(py, 1))?;
                buf.extend(value.to_string(py)?.as_bytes());
                buf.extend(END);
            } else {
                break
            }
        }
        buf.extend(END);
        self.send_maybe(py, EncoderMessage::Bytes(buf.freeze()));

        Ok(py.None())
    }

    def write_eof(&self, chunk: Option<PyBytes>) -> PyResult<PyObject> {
        if let Some(chunk) = chunk {
            self.send_maybe(py, EncoderMessage::PyBytes(chunk));
        }
        self._sender(py).borrow_mut().take();

        Ok(PyFuture::done_fut(py, self._loop(py).clone(), py.None())?.into_object())
    }

    def drain(&self, last: bool = false) -> PyResult<PyObject> {
        Ok(PyFuture::done_fut(py, self._loop(py).clone(), py.None())?.into_object())
    }

});


impl PayloadWriter {

    pub fn new(py: Python, h: Handle, sender: Sender<EncoderMessage>) -> PyResult<PayloadWriter> {
        PayloadWriter::create_instance(
            py, h, RefCell::new(Some(sender)),
            Cell::new(0),
            Cell::new(false),
            Cell::new(ContentCompression::Default))
    }

    fn send_maybe(&self, py: Python, msg: EncoderMessage) {
        if let Some(ref mut sender) = *self._sender(py).borrow_mut() {
            let _ = sender.send(msg);
        }
    }
    
}
