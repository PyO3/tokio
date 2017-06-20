use std::io;
use std::ptr;
use std::os::raw::{c_void, c_int};

use twoway;
use pyo3::{ffi, py};
use pyo3::buffer::PyBuffer;
use pyo3::{self, class, exc, Python, PyToken, Py, PyClone,
           ObjectProtocol, ToPyPointer, InstancePtr,
           PyResult, PyObject, PySlice, PyErr, PyDowncastFrom, PyDowncastInto, ToPyObject};
use bytes::{Bytes, BytesMut, BufMut};

#[py::class]
///
/// Buffer interface for Bytes
///
pub struct PyBytes {
    bytes: Bytes,
    token: PyToken,
}


#[py::methods]
impl PyBytes {

    fn find(&self, py: Python, sub: pyo3::PyBytes,
            start: Option<isize>, end: Option<isize>) -> PyResult<isize> {
        let mut pre = 0;
        let pos = match (&start, &end) {
            (&None, &None) => {
                twoway::find_bytes(self.bytes.as_ref(), sub.data(py))
            }
            _ => {
                let start = if let Some(start) = start {start} else {0};
                let end = if let Some(end) = end {end} else {-1};

                let slice = PySlice::new(py, start, end, 1);
                let indices = slice.indices(py, self.bytes.len() as i64)?;
                pre = indices.start as usize;
                let end = (indices.stop+1) as usize;
                twoway::find_bytes(&self.bytes[pre..end], sub.data(py))
            }
        };

        if let Some(pos) = pos {
            Ok((pos+pre) as isize)
        } else {
            Ok(-1)
        }
    }

    #[args(maxsplit="-1")]
    fn split(&self, py: Python, sep: Option<PyObject>, maxsplit: i32) -> PyResult<pyo3::PyList> {
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

        let length = self.bytes.len();

        let mut start = 0;
        let mut result = Vec::new();

        loop {
            if maxsplit >= 0 && result.len() == maxsplit as usize {
                result.push(
                    py.init(|t| PyBytes{bytes: self.bytes.slice_from(start), token: t})?);
                break
            }

            let pos = {
                if remove_empty {
                    let mut p = None;
                    for idx in 0..length-start {
                        if sep.contains(&self.bytes[start+idx]) {
                            p = Some(idx);
                            break
                        }
                    }
                    p
                } else {
                    twoway::find_bytes(&self.bytes[start..length], sep.as_ref())
                }
            };

            if let Some(pos) = pos {
                let pos = start + pos;
                if ! (start == pos && remove_empty) {
                    result.push(
                        py.init(|t| PyBytes{bytes: self.bytes.slice(start, pos), token: t})?);
                }

                start = pos + sep_len;
                if start > length {
                    break
                }
            } else {
                if ! (start == length && remove_empty) {
                    result.push(
                        py.init(|t| PyBytes{bytes: self.bytes.slice_from(start), token: t})?);
                }
                break
            }
        }

        Ok(pyo3::PyList::new(py, result.as_slice()))
    }

    fn strip(&self, py: Python, sep: Option<PyObject>) -> PyResult<Py<PyBytes>> {
        let sep = if let Some(sep) = sep {
            PyBuffer::get(py, &sep)?.to_vec::<u8>(py)?
        } else {
            vec![b' ', b'\t', b'\n', b'\r', b'\x0b', b'\x0c']
        };

        let mut start = 0;
        let mut end = self.bytes.len();

        loop {
            if start == end {
                break
            }
            if sep.contains(&self.bytes[start]) {
                start += 1;
            } else {
                break
            }
        }
        loop {
            if end == start {
                break
            }

            if sep.contains(&self.bytes[end - 1]) {
                end -= 1;
            } else {
                break
            }
        }
        py.init(|t| PyBytes{bytes: self.bytes.slice(start, end), token: t})
    }

    fn decode(&self, py: Python, encoding: Option<pyo3::PyString>,
              errors: Option<pyo3::PyString>) -> PyResult<pyo3::PyString> {
        let bytes = self.as_ref();
        match (encoding, errors) {
            (Some(enc), Some(err)) =>
                Ok(pyo3::PyString::from_object(
                    py, bytes,
                    enc.to_string_lossy(py).as_ref(), err.to_string_lossy(py).as_ref())?),
            (Some(enc), None) =>
                Ok(pyo3::PyString::from_object(
                    py, bytes, enc.to_string_lossy(py).as_ref(), "strict")?),
            (None, Some(err)) =>
                Ok(pyo3::PyString::from_object(
                    py, bytes, "utf-8", err.to_string_lossy(py).as_ref())?),
            (None, None) =>
                Ok(pyo3::PyString::from_object(py, bytes, "utf-8", "strict")?),
        }
    }
}


impl PyBytes {

    pub fn new(py: Python, bytes: Bytes) -> PyResult<Py<PyBytes>> {
        py.init(|t| PyBytes {
            bytes: bytes,
            token: t})
    }

    pub fn from(py: Python, src: &mut BytesMut, length: usize)
                -> Result<Py<PyBytes>, io::Error>
    {
        let bytes = src.split_to(length).freeze();
        match PyBytes::new(py, bytes) {
            Ok(bytes) => Ok(bytes),
            Err(_) =>
                Err(io::Error::new(
                    io::ErrorKind::Other, "Can not create PyBytes instance")),
        }
    }

    pub fn extend_into(&self, dst: &mut BytesMut)  {
        dst.extend(self.bytes.as_ref())
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn slice_to(&self, py: Python, end: usize) -> PyResult<Py<PyBytes>> {
        let bytes = self.bytes.slice_to(end);
        py.init(|token| PyBytes {bytes: bytes, token: token})
    }

    pub fn slice_from(&self, py: Python, begin: usize) -> PyResult<Py<PyBytes>> {
        let bytes = self.bytes.slice_from(begin);
        py.init(|token| PyBytes {bytes: bytes, token: token})
    }
}

#[py::proto]
impl pyo3::class::PyObjectProtocol for PyBytes {

    fn __richcmp__(&self, py: Python,
                   other: PyObject, op: pyo3::CompareOp) -> PyResult<PyObject> {
        match op {
            pyo3::CompareOp::Eq => {
                if let Ok(other) = PyBytes::downcast_from(py, &other) {
                    Ok((self.bytes.as_ref() == other.bytes.as_ref()).to_object(py))
                }
                else if let Ok(other) = pyo3::PyBytes::downcast_from(py, &other) {
                    Ok((self.bytes.as_ref() == other.data(py).as_ref()).to_object(py))
                }
                else {
                    Err(PyErr::new::<exc::TypeError, _>(
                        py, format!("Can not compare PyBytes and {:?}", &other)))
                }
            },
            _ =>
                Err(PyErr::new::<exc::TypeError, _>(py, "Can not complete this operation")),
        }
    }
}


#[py::proto]
impl pyo3::class::PyNumberProtocol for PyBytes {

    fn __add__(&self, py: Python, rhs: PyObject) -> PyResult<PyObject> {
        let l = PyBuffer::get(py, self.as_ref())?;
        let r = PyBuffer::get(py, &rhs)?;

        match (l.as_slice::<u8>(py), r.as_slice::<u8>(py)) {
            (Some(lbuf), Some(rbuf)) => {
                if lbuf.len() == 0 {
                    return Ok(rhs.clone_ref(py))
                }
                if rbuf.len() == 0 {
                    return Ok(self.to_object(py))
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

                Ok(PyBytes::new(py, buf.freeze())?.into())
            },
            _ => Err(PyErr::new::<exc::TypeError, _>(
                py, format!("Can not sum {:?} and {:?}", self.as_ref(), rhs)))
        }
    }
}

#[py::proto]
impl pyo3::class::PyMappingProtocol for PyBytes {

    fn __len__(&self, _py: Python) -> PyResult<usize> {
        Ok(self.bytes.len())
    }

    fn __getitem__(&self, py: Python, key: PyObject) -> PyResult<PyObject> {
        // access by slice
        if let Ok(slice) = PySlice::downcast_from(py, &key) {
            let indices = slice.indices(py, self.bytes.len() as i64)?;

            let s = if indices.step == 1 {
                // continuous chunk of memory
                self.bytes.slice(indices.start as usize, indices.stop as usize)
            } else {
                // copy every "step" byte
                let mut buf = BytesMut::with_capacity(indices.slicelength as usize);

                let mut idx = indices.start;
                while idx < indices.stop {
                    buf.put_u8(self.bytes[idx as usize]);
                    idx += indices.step;
                }
                buf.freeze()
            };
            PyBytes::new(py, s).map(|ob| ob.into())
        }
        // access by index
        else if let Ok(idx) = key.extract::<isize>(py) {
            if idx < 0 {
                Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
            } else {
                let idx = idx as usize;

                if idx < self.bytes.len() {
                    Ok(self.bytes[idx].to_object(py))
                } else {
                    Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
                }
            }
        } else {
            Err(PyErr::new::<exc::TypeError, _>(py, "Index is not supported"))
        }
    }
}


#[py::proto]
impl class::PyBufferProtocol for PyBytes {

    fn bf_getbuffer(&self, py: Python, view: *mut ffi::Py_buffer, flags: c_int) -> PyResult<()> {
        if view == ptr::null_mut() {
            return Err(PyErr::new::<exc::BufferError, _>(py, "View is null"))
        }

        unsafe {
            (*view).obj = ptr::null_mut();
        }

        if (flags & ffi::PyBUF_WRITABLE) == ffi::PyBUF_WRITABLE {
            return Err(PyErr::new::<exc::BufferError, _>(py, "Object is not writable"))
        }

        unsafe {
            (*view).buf = self.bytes.as_ptr() as *mut c_void;
            (*view).len = self.bytes.len() as isize;
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

        Ok(())
    }
}
