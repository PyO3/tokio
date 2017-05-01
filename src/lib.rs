extern crate chan;
extern crate libc;
extern crate net2;
extern crate bytes;
extern crate futures;
extern crate native_tls;
extern crate tokio_io;
extern crate tokio_core;
extern crate tokio_signal;
extern crate tokio_tls;
extern crate boxfnonce;
#[macro_use] extern crate log;
#[macro_use] extern crate cpython;
#[macro_use] extern crate lazy_static;

pub mod fut;
pub mod http;
pub mod addrinfo;
pub mod utils;
pub mod handle;
pub mod pyfuture;
pub mod pybytes;
pub mod pytask;
pub mod pyunsafe;
mod event_loop;
mod transport;
mod server;
mod client;

pub use utils::{Classes, PyLogger, ToPyErr, with_py};
pub use pybytes::PyBytes;
pub use pyfuture::PyFuture;
pub use pytask::PyTask;
pub use handle::{PyHandle, PyTimerHandle};
pub use event_loop::{TokioEventLoop, new_event_loop};
pub use server::create_server;
pub use client::create_connection;


py_module_initializer!(tokio, init_ext, PyInit_tokio, |py, m| {
    m.add(py, "__doc__", "Asyncio event loop based on tokio-rs")?;
    m.add(py, "new_event_loop", py_fn!(py, new_event_loop()))?;

    register_classes(py, m)?;
    Ok(())
});


pub fn register_classes(py: cpython::Python, m: &cpython::PyModule) -> cpython::PyResult<()> {
    m.add_class::<event_loop::TokioEventLoop>(py)?;
    m.add_class::<pyfuture::PyFuture>(py)?;
    m.add_class::<handle::PyHandle>(py)?;
    m.add_class::<handle::PyTimerHandle>(py)?;
    m.add_class::<server::TokioServer>(py)?;

    // touch classes
    let _ = Classes.Exception;

    Ok(())
}
