#[allow(non_snake_case)]

use std::borrow::Borrow;
use std::iter::Iterator;
use std::collections::VecDeque;

use pyo3::*;
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream, future};

use {PyFuture, PyFuturePtr, TokioEventLoop, TokioEventLoopPtr, pybytes, with_py};
use pyunsafe::{GIL, Sender};
use http::codec::EncoderMessage;
use http::{Request, Version, Headers, ConnectionType, ContentCompression};


#[py::class]
pub struct PyRequest {
    _loop: TokioEventLoop,
    connection: ConnectionType,
    method: PyString,
    url: Url,
    path: PyString,
    version: PyTuple,
    headers: RawHeaders,
    _content: StreamReader,
    _match_info: PyObject,
    _writer: PayloadWriter,
    _time_service: PyObject,
}


#[py::methods]
impl PyRequest {

    #[getter(_method)]
    fn get_method_prop(&self, py: Python) -> PyResult<PyString> {
        Ok(self.method.clone_ref(py))
    }

    #[getter]
    fn get_method(&self, py: Python) -> PyResult<PyString> {
        Ok(self.method.clone_ref(py))
    }

    #[getter]
    fn get_path(&self, py: Python) -> PyResult<PyString> {
        Ok(self.path.clone_ref(py))
    }
    #[getter]
    fn get_rel_url(&self, py: Python) -> PyResult<Url> {
        Ok(self.url.clone_ref(py))
    }
    #[getter]
    fn get_version(&self, py: Python) -> PyResult<PyTuple> {
        Ok(self.version.clone_ref(py))
    }
    #[getter]
    fn get_headers(&self, py: Python) -> PyResult<RawHeaders> {
        Ok(self.headers.clone_ref(py))
    }
    #[getter]
    fn get_content(&self, py: Python) -> PyResult<StreamReader> {
        Ok(self._content.clone_ref(py))
    }
    #[getter]
    fn get_keep_alive(&self, py: Python) -> PyResult<bool> {
        if let &ConnectionType::KeepAlive = self.connection {
            Ok(true)
        } else {
            Ok(false)
        }
    }
    #[getter]
    fn get_match_info(&self, py: Python) -> PyResult<PyObject> {
        Ok(self._match_info.clone_ref(py))
    }
    #[setter]
    fn set_match_info(&mut self, py: Python, value: &PyObject) -> PyResult<()> {
        self._match_info = value.clone_ref(py);
        Ok(())
    }
    #[getter(_writer)]
    fn get_writer_prop(&self, py: Python) -> PyResult<PayloadWriter> {
        Ok(self._writer.clone_ref(py))
    }
    #[getter]
    fn get_writer(&self, py: Python) -> PyResult<PayloadWriter> {
        Ok(self._writer.clone_ref(py))
    }
    #[getter]
    fn get_time_service(&self, py: Python) -> PyResult<PyObject> {
        Ok(self._time_service.clone_ref(py))
    }
    #[setter]
    fn set_time_service(&mut self, py: Python, value: &PyObject) -> PyResult<()> {
        self._time_service = value.clone_ref(py);
        Ok(())
    }

    fn _prepare_hook(&self, py: Python, _resp: &PyObject) -> PyResult<PyObject> {
        Ok(PyFuture::done_fut(py, self._loop, py.None())?.into())
    }
}


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
            version, headers, content, py.None(), writer, py.None())
    }

    pub fn content(&self) -> &StreamReader {
        self._content(GIL::python())
    }
}

#[py::class]
pub struct StreamReader {
    _loop: TokioEventLoopPtr,
    _size: usize,
    _total_bytes: usize,
    _eof: bool,
    _eof_waiter: Option<PyFuturePtr>,
    _waiter: Option<PyFuturePtr>,
    _buffer: VecDeque<pybytes::PyBytesPtr>,
    _exception: Option<PyObject>,
}


#[py::methods]
impl StreamReader {
    #[getter]
    fn get_total_bytes(&self, py: Python) -> PyResult<usize> {
        Ok(self._total_bytes)
    }

    fn exception(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref exc) = self._exception {
            Ok(exc.clone_ref(py))
        } else {
            Ok(py.None())
        }
    }

    fn on_eof(&self, py: Python) -> PyResult<()> {
        Ok(())
    }

    fn is_eof(&self, py: Python) -> PyResult<bool> {
        Ok(*self._eof(py))
    }

    fn at_eof(&self, py: Python) -> PyResult<bool> {
        Ok(*self._eof(py) && self._buffer(py).len() == 0)
    }

    fn wait_eof(&self, py: Python) -> PyResult<PyObject> {
        let fut = if let Some(ref fut) = *self._eof_waiter(py) {
            Some(fut.clone_ref(py))
        } else {
            None
        };

        if let Some(fut) = fut {
            Ok(fut.into_object())
        } else {
            let fut = PyFuture::new(py, self._loop(py))?;
            *self._eof_waiter_mut(py) = Some(fut.clone_ref(py));
            Ok(fut.into_object())
        }
    }

    fn unread_data(&self, py: Python) -> PyResult<PyObject> {
        Ok(py.None())
    }

    fn readline(&self, py: Python) -> PyResult<PyObject> {
        Ok(py.None())
    }

    #[defaults(n="-1")]
    fn read(&self, py: Python, n: isize) -> PyResult<PyObject> {
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
            *self._waiter_mut(py) = Some(fut.clone_ref(py));

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

    fn readany(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if self._buffer(py).borrow().is_empty() && !*self._eof(py) {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);
            *self._waiter_mut(py) = Some(fut.clone_ref(py));

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

    fn readchunk(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if self._buffer(py).borrow().is_empty() && !*self._eof(py) {
            let fut = PyFuture::new(py, self._loop(py))?;
            let fut_read = fut.clone_ref(py);
            *self._waiter_mut(py) = Some(fut.clone_ref(py));

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

    fn readexactly(&self, py: Python) -> PyResult<PyObject> {
        Ok(py.None())
    }

    #[defaults(n="-1")]
    fn read_nowait(&self, py: Python, n: isize) -> PyResult<PyObject> {
        if let Some(ref exc) = *self._exception(py).borrow() {
            Err(PyErr::from_instance(py, exc.clone_ref(py)))
        } else if let Some(_) = *self._waiter(py).borrow() {
            Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Called while some coroutine is waiting for incoming data."))
        } else {
            self._read_nowait(py, n).map(|b| b.into_object())
        }
    }
}


impl StreamReader {

    fn new(py: Python, evloop: &TokioEventLoop) -> PyResult<StreamReader> {
        StreamReader::create_instance(
            py, evloop.clone_ref(py), 0, 0, false, None, None, VecDeque::new(), None)
    }

    pub fn set_exception(&self) {
    }

    pub fn feed_eof(&self, py: Python) {
        *self._eof_mut(py) = true
    }

    pub fn feed_data(&self, py: Python, bytes: pybytes::PyBytes) {
        let total_bytes = self._total_bytes_mut(py);
        *total_bytes = *total_bytes + bytes.len();
        self._buffer_mut(py).push_back(bytes);

        if let Some(fut) = self._waiter_mut(py).take() {
            let _ = fut.set(py, Ok(py.None()));
        }
    }

    pub fn _read_nowait_chunk(&self, py: Python, n: isize) -> PyResult<pybytes::PyBytes> {
        let size = if n < 0 { 0 } else { n as usize };
        let mut buffer = self._buffer_mut(py);

        let first_chunk = buffer.pop_front().unwrap();
        let result = if n != -1 && first_chunk.len() > size {
            buffer.push_front(first_chunk.slice_from(py, size)?);
            first_chunk.slice_to(py, size)?
        } else {
            first_chunk
        };

        *self._size_mut(py) = *self._size(py) - result.len();
        Ok(result)
    }

    pub fn _read_nowait(&self, py: Python, n: isize) -> PyResult<pybytes::PyBytes> {
        let mut size = 0;
        let mut chunks = Vec::new();
        let mut counter = if n < 0 { 0 } else { n as usize };
        let mut buffer = self._buffer_mut(py);

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

#[py::class]
pub struct RawHeaders {
    headers: Headers,
}

#[py::methods]
impl RawHeaders {

    fn items(&self, py: Python) -> PyResult<PyObject> {
        let mut items = Vec::new();

        for (name, value) in self.headers(py).headers() {
            items.push(
                (PyString::new(py, name.as_str()),
                 PyString::new(py, value.as_str())).to_py_object(py).into_object());
        }
        Ok(PyList::new(py, items.as_slice()).into_object())
    }

    fn get(&self, py: Python, key: &PyString, default: Option<PyObject>) -> PyResult<PyObject> {
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

    fn __getitem__(&self, py: Python, key: PyString) -> PyResult<PyObject> {
        let key = key.to_string(py)?;
        if let Some(val) = self.headers(py).get(key.borrow()) {
            Ok(PyString::new(py, val).into_object())
        } else {
            Err(PyErr::new::<exc::KeyError, _>(py, "item not found"))
        }
    }

    fn __contains__(&self, py: Python, key: PyString) -> PyResult<bool> {
        let key = key.to_string(py)?;
        if let Some(_) = self.headers(py).get(key.borrow()) {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

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

        if *self._size(py) > 0 {
            Ok(Async::Ready(Some(self._read_nowait_chunk(py, -1))))
        } else {
            let mut fut = if let Some(fut) = self._waiter_mut(py).take() {
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

#[py::class]
pub struct Url {
    path: PyString
}

#[py::methods]
impl Url {

    #[getter]
    fn get_raw_path(&self, py: Python) -> PyResult<PyString> {
        Ok(self.path(py).clone_ref(py))
    }
}


impl Url {
    fn new(py: Python, path: PyString) -> PyResult<Url> {
        Url::create_instance(py, path)
    }
}


const SEP: &'static [u8] = b": ";
const END: &'static [u8] = b"\r\n";


#[py::class]
pub struct PayloadWriter {
    _loop: TokioEventLoop,
    _sender: Option<Sender<EncoderMessage>>,
    _length: u64,
    _chunked: bool,
    _compress: ContentCompression,
}

#[py::methods]
impl PayloadWriter {

    #[getter]
    fn get_length(&self, py: Python) -> PyResult<u64> {
        Ok(*self._length(py))
    }
    #[setter]
    fn set_length(&self, py: Python, value: u64) -> PyResult<()> {
        *self._length_mut(py) = value;
        Ok(())
    }
    #[getter]
    fn get_output_size(&self, py: Python) -> PyResult<u64> {
        Ok(*self._length(py))
    }

    fn enable_chunking(&self, py: Python) -> PyResult<PyObject> {
        *self._chunked_mut(py) = true;
        Ok(py.None())
    }

    fn enable_compression(&self, py: Python, encoding: Option<PyString>) -> PyResult<PyObject> {
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
        *self._compress_mut(py) = enc;

        Ok(py.None())
    }

    #[defaults(_drain=true)]
    fn write(&self, py: Python, chunk: PyBytes, _drain: bool) -> PyResult<PyObject> {
        self.send_maybe(py, EncoderMessage::PyBytes(chunk));
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

    // Build Request message from status line and headers object
    // status_line - string with \r\n
    // headers = dict like object
    fn write_headers(&self, py: Python, status_line: &PyString, headers: &PyObject) -> PyResult<PyObject> {
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

    fn write_eof(&self, py: Python, chunk: Option<PyBytes>) -> PyResult<PyObject> {
        if let Some(chunk) = chunk {
            self.send_maybe(py, EncoderMessage::PyBytes(chunk));
        }
        self._sender_mut(py).take();

        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }

    #[defaults(_last=false)]
    fn drain(&self, py: Python, _last: bool) -> PyResult<PyObject> {
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?.into_object())
    }
}


impl PayloadWriter {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               sender: Sender<EncoderMessage>) -> PyResult<PayloadWriter> {
        PayloadWriter::create_instance(
            py, evloop.clone_ref(py), Some(sender), 0, false, ContentCompression::Default)
    }

    fn send_maybe(&self, py: Python, msg: EncoderMessage) {
        if let Some(ref mut sender) = *self._sender_mut(py) {
            let _ = sender.send(msg);
        }
    }
}
