use std::io;
use std::ptr;
use libc::c_void;

use cpython::*;
use cpython::_detail::ffi;
use bytes::{Bytes, BytesMut};


//
// Buffer interface for Bytes
//
py_class!(pub class PyBytes |py| {
    data _bytes: Bytes;

    def __len__(&self) -> PyResult<usize> {
        Ok(self._bytes(py).len())
    }

    def __buffer_get__(&self, view, flags) -> bool {
        unsafe {
            (*view).obj = ptr::null_mut();
        }

        if flags != 0 {
            unsafe {
                let msg = ::std::ffi::CStr::from_ptr(
                    concat!(
                        "Only simple, read-only buffer is available","\0").as_ptr() as *const _);
                ffi::PyErr_SetString(ffi::PyExc_BufferError, msg.as_ptr());
            }
            return false;
        }

        let bytes = self._bytes(py);

        unsafe {
            (*view).buf = bytes.as_ref().as_ptr() as *mut c_void;
            (*view).len = bytes.len() as isize;
            (*view).readonly = 1;
            (*view).itemsize = 1;
            (*view).format = ptr::null_mut();
            (*view).ndim = 1;
            (*view).shape = ptr::null_mut();
            (*view).strides = ptr::null_mut();
            (*view).suboffsets = ptr::null_mut();
            (*view).internal = ptr::null_mut();
        }

        return true
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
                    io::ErrorKind::Other, "Can not create TokioBytes instance")),
        }
    }

    pub fn extend_into(&self, py: Python, dst: &mut BytesMut)  {
        dst.extend(self._bytes(py).as_ref())
    }

    pub fn len(&self, py: Python) -> usize {
        self._bytes(py).len()
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
