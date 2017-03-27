use std::cell;
use cpython::*;
use futures::future::*;

use utils::{Classes, Handle};


//pub fn create_future(py: Python, h: Handle) -> PyResult<Future> {
//}



py_class!(pub class TokioTcpTrasnport |py| {

    def test(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }
});
