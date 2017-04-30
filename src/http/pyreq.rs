use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::iter::Iterator;
use std::collections::VecDeque;

use cpython::*;
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream, future};

use ::{PyFuture, TokioEventLoop, pybytes, with_py};
use pyunsafe::{GIL, Sender};
use http::codec::EncoderMessage;
use http::{Request, Version, Headers, ConnectionType, ContentCompression};


py_class!(pub class PyRequest |py| {
    data _loop: TokioEventLoop;
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

    def _prepare_hook(&self, _resp: &PyObject) -> PyResult<PyObject> {
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

});


impl PyRequest {
    pub fn new(py: Python, req: Request,
               evloop: &TokioEventLoop, sender: Sender<EncoderMessage>) -> PyResult<PyRequest> {
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
        let content = StreamReader::new(py, evloop)?;
        let headers = RawHeaders::create_instance(py, req.headers)?;
        let writer = PayloadWriter::new(py, evloop, sender)?;

        PyRequest::create_instance(
            py, evloop.clone_ref(py), conn, meth, url, path,
            version, headers, content,
            RefCell::new(py.None()), writer, RefCell::new(py.None()))
    }

    pub fn content(&self) -> &StreamReader {
        self._content(GIL::python())
    }
}


py_class!(pub class StreamReader |py| {
    data _loop: TokioEventLoop;
    data _size: Cell<usize>;
    data _total_bytes: Cell<usize>;
    data _eof: Cell<bool>;
    data _eof_waiter: RefCell<Option<PyFuture>>;
    data _waiter: RefCell<Option<PyFuture>>;
    data _buffer: RefCell<VecDeque<pybytes::PyBytes>>;
    data _exception: RefCell<Option<PyObject>>;

    property total_bytes {
        get(&slf) -> PyResult<usize> {
            Ok(slf._total_bytes(py).get())
        }
    }

    def exception(&self) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Ok(exc.clone_ref(py))
        } else {
            Ok(py.None())
        }
    }

    def on_eof(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def is_eof(&self) -> PyResult<bool> {
        Ok(self._eof(py).get())
    }

    def at_eof(&self) -> PyResult<PyObject> {
        if self._eof(py).get() && self._buffer(py).borrow().len() == 0 {
            Ok(py.True().into_object())
        } else {
            Ok(py.False().into_object())
        }
    }

    def wait_eof(&self) -> PyResult<PyObject> {
        let fut = if let Some(ref fut) = *self._eof_waiter(py).borrow() {
            Some(fut.clone_ref(py))
        } else {
            None
        };

        if let Some(fut) = fut {
            Ok(fut.into_object())
        } else {
            let fut = PyFuture::new(py, self._loop(py))?;
            *self._eof_waiter(py).borrow_mut() = Some(fut.clone_ref(py));
            Ok(fut.into_object())
        }
    }

    def unread_data(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def readline(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def read(&self, n: isize = -1) -> PyResult<PyObject> {
        if n == 0 {
            let chunk = pybytes::PyBytes::new(py, Bytes::new())?.into_object();
            Ok(PyFuture::done_fut(py, self._loop(py), chunk)?.into_object())
        } else if n < 0 {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);

            let stream = self.clone_ref(py).collect().and_then(move |chunks| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                let mut size = 0;
                for chunk in chunks.iter() {
                    if let &Ok(ref chunk) = chunk {
                        size += chunk.len();
                    }
                }

                let mut buf = BytesMut::with_capacity(size);
                for chunk in chunks {
                    chunk.unwrap().extend_into(py, &mut buf);
                }
                fut_read.set(py, pybytes::PyBytes::new(
                    py, buf.freeze()).map(|b| b.into_object()));
                future::ok(())
            });
            self._loop(py).href().spawn(stream.map_err(|_| {}));

            Ok(fut.into_object())

        } else {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);
            *self._waiter(py).borrow_mut() = Some(fut.clone_ref(py));

            // wait until we get more data
            let stream = self.clone_ref(py);
            let waiter = fut.clone_ref(py).and_then(move |_| {
                with_py(|py| fut_read.set(
                    py, stream._read_nowait(py, n).map(|b| b.into_object())));
                future::ok(())
            });
            self._loop(py).href().spawn(waiter.map_err(|_| {}));

            Ok(fut.into_object())
        }
    }

    def readany(&self) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if self._buffer(py).borrow().is_empty() && !self._eof(py).get() {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);
            *self._waiter(py).borrow_mut() = Some(fut.clone_ref(py));

            // wait until we get more data
            let stream = self.clone_ref(py);
            let waiter = fut.clone_ref(py).and_then(move |_| {
                with_py(|py| fut_read.set(py, stream._read_nowait(py, -1)
                                          .map(|b| b.into_object())));
                future::ok(())
            }).map_err(|_| {});
            self._loop(py).href().spawn(waiter);

            Ok(fut.into_object())
        } else {
            let chunk = self._read_nowait(py, -1)?;
            Ok(PyFuture::done_fut(py, self._loop(py), chunk.into_object())?.into_object())
        }
    }

    def readchunk(&self) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if self._buffer(py).borrow().is_empty() && !self._eof(py).get() {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);
            *self._waiter(py).borrow_mut() = Some(fut.clone_ref(py));

            // wait until we get more data
            let stream = self.clone_ref(py);
            let waiter = fut.clone_ref(py).and_then(move |_| {
                with_py(|py|
                        fut_read.set(
                            py, stream._read_nowait_chunk(py, -1).map(|b| b.into_object())));
                future::ok(())
            }).map_err(|_| {});
            self._loop(py).href().spawn(waiter);

            Ok(fut.into_object())
        } else {
            let chunk = self._read_nowait_chunk(py, -1)?;
            Ok(PyFuture::done_fut(py, self._loop(py), chunk.into_object())?.into_object())
        }
    }

    def readexactly(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }

    def read_nowait(&self, n: isize = -1) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if let Some(_) = *self._waiter(py).borrow() {
            Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Called while some coroutine is waiting for incoming data."))
        } else {
            self._read_nowait(py, n).map(|b| b.into_object())
        }
    }

});


impl StreamReader {

    fn new(py: Python, evloop: &TokioEventLoop) -> PyResult<StreamReader> {
        StreamReader::create_instance(
            py, evloop.clone_ref(py), Cell::new(0), Cell::new(0), Cell::new(false),
            RefCell::new(None),
            RefCell::new(None),
            RefCell::new(VecDeque::new()),
            RefCell::new(None))
    }

    pub fn set_exception(&self) {
    }

    pub fn feed_eof(&self, py: Python) {
        self._eof(py).set(true)
    }

    pub fn feed_data(&self, py: Python, bytes: pybytes::PyBytes) {
        let total_bytes = self._total_bytes(py);
        total_bytes.set(total_bytes.get() + bytes.len());
        self._buffer(py).borrow_mut().push_back(bytes);

        if let Some(fut) = self._waiter(py).borrow_mut().take() {
            let _ = fut.set(py, Ok(py.None()));
        }
    }

    pub fn _read_nowait_chunk(&self, py: Python, n: isize) -> PyResult<pybytes::PyBytes> {
        let size = if n < 0 { 0 } else { n as usize };
        let mut buffer = self._buffer(py).borrow_mut();

        let first_chunk = buffer.pop_front().unwrap();
        let result = if n != -1 && first_chunk.len() > size {
            buffer.push_front(first_chunk.slice_from(py, size)?);
            first_chunk.slice_to(py, size)?
        } else {
            first_chunk
        };

        self._size(py).set(self._size(py).get() - result.len());
        Ok(result)
    }

    pub fn _read_nowait(&self, py: Python, n: isize) -> PyResult<pybytes::PyBytes> {
        let mut size = 0;
        let mut chunks = Vec::new();
        let mut counter = if n < 0 { 0 } else { n as usize };
        let mut buffer = self._buffer(py).borrow_mut();

        loop {
            if let Some(chunk) = buffer.pop_front() {
                let result = if n != -1 && chunk.len() > counter {
                    buffer.push_front(chunk.slice_from(py, counter)?);
                    chunk.slice_to(py, counter)?
                } else {
                    chunk
                };
                size += result.len();
                if counter > 0 {
                    counter = counter - result.len()
                } else {
                    break
                }
            } else {
                break
            }
        }

        if chunks.len() == 1 {
            Ok(pybytes::PyBytes::new(py, chunks.pop().unwrap())?)
        } else {
            let mut buf = BytesMut::with_capacity(size);
            for chunk in chunks {
                buf.extend(chunk);
            }
            Ok(pybytes::PyBytes::new(py, buf.freeze())?)
        }
    }
}


py_class!(pub class RawHeaders |py| {
    data headers: Headers;

    def items(&self) -> PyResult<PyObject> {
        let mut items = Vec::new();

        for (name, value) in self.headers(py).headers() {
            items.push(
                (PyString::new(py, name.as_str()),
                 PyString::new(py, value.as_str())).to_py_object(py).into_object());
        }
        Ok(PyList::new(py, items.as_slice()).into_object())
    }

    def get(&self, key: &PyString, default: Option<PyObject> = None) -> PyResult<PyObject> {
        let key = key.to_string(py)?;
        if let Some(val) = self.headers(py).get(key.borrow()) {
            Ok(PyString::new(py, val).into_object())
        } else {
            if let Some(default) = default {
                Ok(default)
            } else {
                Ok(py.None())
            }
        }
    }

    def __getitem__(&self, key: PyString) -> PyResult<PyObject> {
        let key = key.to_string(py)?;
        if let Some(val) = self.headers(py).get(key.borrow()) {
            Ok(PyString::new(py, val).into_object())
        } else {
            Err(PyErr::new::<exc::KeyError, _>(py, "item not found"))
        }
    }

    def __contains__(&self, key: PyString) -> PyResult<bool> {
        let key = key.to_string(py)?;
        if let Some(_) = self.headers(py).get(key.borrow()) {
            Ok(true)
        } else {
            Ok(false)
        }
    }

});

impl RawHeaders {
    pub fn new(py: Python, headers: Headers) -> PyResult<RawHeaders> {
        RawHeaders::create_instance(py, headers)
    }
}


impl Stream for StreamReader {
    type Item = PyResult<pybytes::PyBytes>;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let gil = Python::acquire_gil();
        let py = gil.python();

        if self._size(py).get() > 0 {
            Ok(Async::Ready(Some(self._read_nowait_chunk(py, -1))))
        } else {
            let mut fut = if let Some(fut) = self._waiter(py).borrow_mut().take() {
                fut
            } else {
                PyFuture::new(py, self._loop(py)).unwrap()
            };

            match fut.poll() {
                Ok(Async::Ready(_)) => {
                    self.poll()
                },
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(_) => Err(())
            }
        }
    }
}


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
    data _loop: TokioEventLoop;
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

    def write(&self, chunk: PyBytes, _drain: bool = true) -> PyResult<PyObject> {
        self.send_maybe(py, EncoderMessage::PyBytes(chunk));
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

    // Build Request message from status line and headers object
    // status_line - string with \r\n
    // headers = dict like object
    def write_headers(&self, status_line: &PyString, headers: &PyObject) -> PyResult<PyObject> {
        let mut buf = BytesMut::with_capacity(512);

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

                // encode name
                let key = PyString::downcast_from(py, item.get_item(py, 0))?;
                buf.extend(key.to_string(py)?.as_bytes());
                buf.extend(SEP);

                // encode value, get string or convert to string
                let value = item.get_item(py, 1);
                if let Ok(value) = PyString::downcast_from(py, value.clone_ref(py)) {
                    buf.extend(value.to_string(py)?.as_bytes());
                } else {
                    buf.extend(format!("{}", value).as_bytes());
                }

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

        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

    def drain(&self, _last: bool = false) -> PyResult<PyObject> {
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

});


impl PayloadWriter {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               sender: Sender<EncoderMessage>) -> PyResult<PayloadWriter> {
        PayloadWriter::create_instance(
            py, evloop.clone_ref(py), RefCell::new(Some(sender)),
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
