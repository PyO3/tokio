use std::io;
use std::ptr;
use libc::c_void;

use cpython::{exc, Python, PyClone,
              PyResult, PyObject, PySlice, PyErr, PythonObjectWithCheckedDowncast};
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

    def __getitem__(&self, key: PyObject) -> PyResult<PyBytes> {
        // access by slice
        if let Ok(slice) = PySlice::downcast_from(py, key.clone_ref(py)) {
            let bytes = self._bytes(py);
            let indices = slice.indices(py, bytes.len() as i64)?;

            let s = if indices.step == 1 {
                bytes.slice(indices.start as usize, indices.stop as usize)
            } else {
                let mut buf = BytesMut::with_capacity(indices.slicelength as usize);

                let mut idx = indices.start;
                while idx < indices.stop {
                    buf.put_u8(bytes[idx as usize]);
                    idx += indices.step;
                }
                buf.freeze()
            };
            Ok(PyBytes::new(py, s)?)
        }
        // access by index
        else if let Ok(idx) = key.extract::<isize>(py) {
            if idx < 0 {
                Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
            } else {
                let idx = idx as usize;
                let bytes = self._bytes(py);

                if idx < bytes.len() {
                    Ok(PyBytes::new(py, bytes.slice(idx, idx+1))?)
                } else {
                    Err(PyErr::new::<exc::IndexError, _>(py, "Index out of range"))
                }
            }
        } else {
            Err(PyErr::new::<exc::TypeError, _>(py, "Index is not supported"))
        }
    }

    def __buffer_get__(&self, view, flags) -> bool {
        unsafe {
            (*view).obj = ptr::null_mut();
        }

        if view == ptr::null_mut() {
            unsafe {
                let msg = ::std::ffi::CStr::from_ptr("View is null\0".as_ptr() as *const _);
                ffi::PyErr_SetString(ffi::PyExc_BufferError, msg.as_ptr());
            }
            return false;
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
