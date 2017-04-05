use std::io;
use std::ptr;
use libc::c_void;

use cpython::*;
use cpython::_detail::ffi;
use bytes::{Bytes, BytesMut};


pub fn create_bytes(py: Python, bytes: Bytes) -> PyResult<TokioBytes> {
    TokioBytes::create_instance(py, bytes)
}


//
// Buffer interface for Bytes
//
py_class!(pub class TokioBytes |py| {
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
                    concat!("Only simple, read-only buffer is available",
                            "\0").as_ptr() as *const _);
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


impl TokioBytes {

    pub fn create(py: Python, src: Bytes) -> PyResult<TokioBytes> {
        TokioBytes::create_instance(py, src)
    }

    pub fn from(py: Python, src: &mut BytesMut, length: usize) -> Result<TokioBytes, io::Error> {
        let bytes = src.split_to(length).freeze();
        match TokioBytes::create(py, bytes) {
            Ok(bytes) => Ok(bytes),
            Err(_) =>
                Err(io::Error::new(
                    io::ErrorKind::Other, "Can not create TokioBytes instance")),
        }
    }

}
