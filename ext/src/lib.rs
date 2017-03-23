extern crate futures;
extern crate tokio_core;
#[macro_use] extern crate log;
#[macro_use] extern crate cpython;

#[cfg(feature="cpython/python27-sys")]
extern crate python27_sys as ffi;

#[cfg(feature="cpython/python3-sys")]
extern crate python3_sys as ffi;

mod handle;
mod utils;
mod event_loop;
use event_loop::{TokioEventLoop, new_event_loop, run_event_loop};


py_module_initializer!(_ext, init_ext, PyInit__ext, |py, m| {
    m.add(py, "__doc__", "Asyncio event loop based on tokio")?;

    init_tokio_module(py, m)?;
    Ok(())
});


pub fn init_tokio_module(py: cpython::Python, m: &cpython::PyModule) -> cpython::PyResult<()> {
    m.add_class::<TokioEventLoop>(py)?;
    m.add(py, "new_event_loop", py_fn!(py, new_event_loop()))?;
    m.add(py, "run_event_loop", py_fn!(py, run_event_loop(event_loop: &TokioEventLoop)))?;
    Ok(())
}
