use std::io;
use std::ptr;
use libc::c_void;

use twoway;
use cpython::buffer::PyBuffer;
use cpython::{self, exc, Python, PythonObject, PyClone,
              PyResult, PyObject, PySlice, PyErr, PythonObjectWithCheckedDowncast, ToPyObject};
use cpython::_detail::ffi;
use bytes::{Bytes, BytesMut, BufMut};

use pyunsafe::GIL;

//
// Buffer interface for Bytes
//
py_class!(pub class PyBytes |py| {
    data _bytes: Bytes;

    def __len__(&self) -> PyResult<usize> {
        Ok(self._bytes(py).len())
    }

    def __getitem__(&self, key: PyObject) -> PyResult<PyObject> {
        // access by slice
        if let Ok(slice) = PySlice::downcast_from(py, key.clone_ref(py)) {
            let bytes = self._bytes(py);
            let indices = slice.indices(py, bytes.len() as i64)?;

            let s = if indices.step == 1 {
                // continuous chunk of memory
                bytes.slice(indices.start as usize, indices.stop as usize)
            } else {
                // copy every "step" byte
                let mut buf = BytesMut::with_capacity(indices.slicelength as usize);

                let mut idx = indices.start;
                while idx < indices.stop {
                    buf.put_u8(bytes[idx as usize]);
                    idx += indices.step;
                }
                buf.freeze()
            };
            Ok(PyBytes::new(py, s)?.into_object())
        }
        // access by index
        else if let Ok(idx) = key.extract::<isize>(py) {
            if idx < 0 {
                Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
            } else {
                let idx = idx as usize;
                let bytes = self._bytes(py);

                if idx < bytes.len() {
                    Ok(bytes[idx].to_py_object(py).into_object())
                } else {
                    Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
                }
            }
        } else {
            Err(PyErr::new::<exc::TypeError, _>(py, "Index is not supported"))
        }
    }

    def __buffer_get__(&self, view, flags) -> bool {
        if view == ptr::null_mut() {
            unsafe {
                let msg = ::std::ffi::CStr::from_ptr("View is null\0".as_ptr() as *const _);
                ffi::PyErr_SetString(ffi::PyExc_BufferError, msg.as_ptr());
            }
            return false;
        }

        unsafe {
            (*view).obj = ptr::null_mut();
        }

        if (flags & ffi::PyBUF_WRITABLE) == ffi::PyBUF_WRITABLE {
            unsafe {
                let msg = ::std::ffi::CStr::from_ptr(
                    "Object is not writable\0".as_ptr() as *const _);
                ffi::PyErr_SetString(ffi::PyExc_BufferError, msg.as_ptr());
            }
            return false;
        }

        let bytes = self._bytes(py);

        unsafe {
            (*view).buf = bytes.as_ptr() as *mut c_void;
            (*view).len = bytes.len() as isize;
            (*view).readonly = 1;
            (*view).itemsize = 1;

            (*view).format = ptr::null_mut();
            if (flags & ffi::PyBUF_FORMAT) == ffi::PyBUF_FORMAT {
                let msg = ::std::ffi::CStr::from_ptr("B\0".as_ptr() as *const _);
                (*view).format = msg.as_ptr() as *mut _;
            }

            (*view).ndim = 1;
            (*view).shape = ptr::null_mut();
            if (flags & ffi::PyBUF_ND) == ffi::PyBUF_ND {
                (*view).shape = (&((*view).len)) as *const _ as *mut _;
            }

            (*view).strides = ptr::null_mut();
            if (flags & ffi::PyBUF_STRIDES) == ffi::PyBUF_STRIDES {
                (*view).strides = &((*view).itemsize) as *const _ as *mut _;
            }

            (*view).suboffsets = ptr::null_mut();
            (*view).internal = ptr::null_mut();
        }

        true
    }

    def __add__(lhs, rhs) -> PyResult<PyObject> {
        let l = PyBuffer::get(py, lhs)?;
        let r = PyBuffer::get(py, rhs)?;

        match (l.as_slice::<u8>(py), r.as_slice::<u8>(py)) {
            (Some(lbuf), Some(rbuf)) => {
                if lbuf.len() == 0 {
                    return Ok(rhs.clone_ref(py))
                }
                if rbuf.len() == 0 {
                    return Ok(lhs.clone_ref(py))
                }
                let len = lbuf.len() + rbuf.len();
                let mut buf = BytesMut::with_capacity(len);

                {
                    let mut slice = unsafe { buf.bytes_mut() };
                    l.copy_to_slice(py, &mut slice[..lbuf.len()])?;
                    r.copy_to_slice(py, &mut slice[lbuf.len()..lbuf.len()+rbuf.len()])?;
                }
                unsafe {
                    buf.set_len(len)
                };

                Ok(PyBytes::new(py, buf.freeze())?.into_object())
            },
            _ => Err(PyErr::new::<exc::TypeError, _>(
                py, format!("Can not sum {:?} and {:?}", lhs, rhs)))
        }
    }

    def __richcmp__(&self, other: PyObject, op: cpython::CompareOp) -> PyResult<bool> {
        match op {
            cpython::CompareOp::Eq => {
                if let Ok(other) = PyBytes::downcast_from(py, other.clone_ref(py).into_object()) {
                    Ok(self._bytes(py).as_ref() == other._bytes(py).as_ref())
                }
                else if let Ok(other) = cpython::PyBytes::downcast_from(
                    py, other.clone_ref(py).into_object()) {
                    Ok(self._bytes(py).as_ref() == other.data(py).as_ref())
                }
                else {
                    Err(PyErr::new::<exc::TypeError, _>(
                        py, format!("Can not compare PyBytes and {:?}",
                                    other.get_type(py).into_object())))
                }
            },
            _ =>
                Err(PyErr::new::<exc::TypeError, _>(py, "Can not complete this operation")),
        }
    }

    def find(&self, sub: cpython::PyBytes,
             start: Option<isize> = None, end: Option<isize> = None) -> PyResult<isize> {
        let mut pre = 0;
        let pos = match (&start, &end) {
            (&None, &None) => {
                twoway::find_bytes(self._bytes(py).as_ref(), sub.data(py))
            }
            _ => {
                let start = if let Some(start) = start {start} else {0};
                let end = if let Some(end) = end {end} else {-1};

                let bytes = self._bytes(py);
                let slice = PySlice::new(py, start, end, 1);
                let indices = slice.indices(py, bytes.len() as i64)?;
                pre = indices.start as usize;
                let end = (indices.stop+1) as usize;
                twoway::find_bytes(&bytes[pre..end], sub.data(py))
            }
        };

        if let Some(pos) = pos {
            Ok((pos+pre) as isize)
        } else {
            Ok(-1)
        }
    }

    def split(&self,
              sep: Option<PyObject> = None,
              maxsplit: i32 = -1) -> PyResult<cpython::PyList> {
        let sep_len;
        let remove_empty;
        let sep = if let Some(sep) = sep {
            remove_empty = false;
            let v = PyBuffer::get(py, &sep)?.to_vec::<u8>(py)?;
            sep_len = v.len();
            v
        } else {
            remove_empty = true;
            sep_len = 1;
            vec![b' ', b'\t', b'\n', b'\r', b'\x0b', b'\x0c']
        };

        let bytes = self._bytes(py);
        let length = bytes.len();

        let mut start = 0;
        let mut result = Vec::new();

        loop {
            if maxsplit >= 0 && result.len() == maxsplit as usize {
                result.push(
                    PyBytes::create_instance(py, bytes.slice_from(start))?.into_object());
                break
            }

            let pos = {
                if remove_empty {
                    let mut p = None;
                    for idx in 0..length-start {
                        if sep.contains(&bytes[start+idx]) {
                            p = Some(idx);
                            break
                        }
                    }
                    p
                } else {
                    twoway::find_bytes(&bytes[start..length], sep.as_ref())
                }
            };

            if let Some(pos) = pos {
                let pos = start + pos;
                if ! (start == pos && remove_empty) {
                    result.push(
                        PyBytes::create_instance(py, bytes.slice(start, pos))?.into_object());
                }

                start = pos + sep_len;
                if start > length {
                    break
                }
            } else {
                if ! (start == length && remove_empty) {
                    result.push(
                        PyBytes::create_instance(py, bytes.slice_from(start))?.into_object());
                }
                break
            }
        }

        Ok(cpython::PyList::new(py, result.as_slice()))
    }

    def strip(&self, sep: Option<PyObject> = None) -> PyResult<PyBytes> {
        let sep = if let Some(sep) = sep {
            PyBuffer::get(py, &sep)?.to_vec::<u8>(py)?
        } else {
            vec![b' ', b'\t', b'\n', b'\r', b'\x0b', b'\x0c']
        };

        let bytes = self._bytes(py);
        let mut start = 0;
        let mut end = bytes.len();

        loop {
            if start == end {
                break
            }
            if sep.contains(&bytes[start]) {
                start += 1;
            } else {
                break
            }
        }
        loop {
            if end == start {
                break
            }

            if sep.contains(&bytes[end - 1]) {
                end -= 1;
            } else {
                break
            }
        }
        Ok(PyBytes::create_instance(py, bytes.slice(start, end))?)
    }

    def decode(&self, encoding: Option<cpython::PyString> = None,
               errors: Option<cpython::PyString> = None) -> PyResult<cpython::PyUnicode> {
        let bytes = self.as_object();
        match (encoding, errors) {
            (Some(enc), Some(err)) =>
                Ok(cpython::PyString::from_object(
                    py, bytes,
                    enc.to_string_lossy(py).as_ref(), err.to_string_lossy(py).as_ref())?),
            (Some(enc), None) =>
                Ok(cpython::PyString::from_object(
                    py, bytes, enc.to_string_lossy(py).as_ref(), "strict")?),
            (None, Some(err)) =>
                Ok(cpython::PyString::from_object(
                    py, bytes, "utf-8", err.to_string_lossy(py).as_ref())?),
            (None, None) =>
                Ok(cpython::PyString::from_object(py, bytes, "utf-8", "strict")?),
        }
    }
});


impl PyBytes {

    pub fn new(py: Python, bytes: Bytes) -> PyResult<PyBytes> {
        PyBytes::create_instance(py, bytes)
    }

    pub fn from(py: Python, src: &mut BytesMut, length: usize) -> Result<PyBytes, io::Error> {
        let bytes = src.split_to(length).freeze();
        match PyBytes::new(py, bytes) {
            Ok(bytes) => Ok(bytes),
            Err(_) =>
                Err(io::Error::new(
                    io::ErrorKind::Other, "Can not create PyBytes instance")),
        }
    }

    pub fn extend_into(&self, py: Python, dst: &mut BytesMut)  {
        dst.extend(self._bytes(py).as_ref())
    }

    pub fn len(&self) -> usize {
        self._bytes(GIL::python()).len()
    }

    pub fn slice_to(&self, py: Python, end: usize) -> PyResult<PyBytes> {
        let bytes = self._bytes(py).slice_to(end);
        PyBytes::create_instance(py, bytes)
    }

    pub fn slice_from(&self, py: Python, begin: usize) -> PyResult<PyBytes> {
        let bytes = self._bytes(py).slice_from(begin);
        PyBytes::create_instance(py, bytes)
    }
}
