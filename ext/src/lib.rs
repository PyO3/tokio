extern crate futures;
extern crate tokio_core;
#[macro_use] extern crate log;
#[macro_use] extern crate cpython;

use cpython::*;

mod handle;
mod utils;
mod event_loop;
pub use event_loop::{TokioEventLoop, new_event_loop, spawn_worker};


py_module_initializer!(_ext, init_ext, PyInit__ext, |py, m| {
    m.add(py, "__doc__", "Asyncio event loop based on tokio")?;

    init_tokio_module(py, m)?;
    Ok(())
});


pub fn init_tokio_module(py: cpython::Python, m: &cpython::PyModule) -> cpython::PyResult<()> {
    m.add_class::<TokioEventLoop>(py)?;

    m.add(py, "spawn_worker", py_fn!(py, spawn_worker(name: &PyString)))?;
    m.add(py, "new_event_loop", py_fn!(py, new_event_loop()))?;
    Ok(())
}
