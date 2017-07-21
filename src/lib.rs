#![feature(const_fn, proc_macro, specialization)]
#![recursion_limit="1024"]

extern crate mio;
extern crate chan;
extern crate libc;
extern crate net2;
extern crate bytes;
extern crate twoway;
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;
extern crate tokio_signal;
extern crate tokio_uds;
extern crate boxfnonce;
extern crate env_logger;
extern crate pyo3;
#[macro_use] extern crate log;
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
mod fd;
mod event_loop;
mod transport;
mod socket;
mod server;
mod client;
mod signals;
mod callbacks;

pub use pyo3::*;
pub use utils::{Classes, PyLogger, with_py};
pub use pybytes::PyBytes;
pub use pyfuture::{PyFut, PyFuture};
pub use pytask::{PyTask, PyTaskFut};
pub use handle::PyHandle;
pub use event_loop::{TokioEventLoop, new_event_loop};
pub use server::create_server;
pub use client::create_connection;


#[py::modinit(_tokio)]
/// Asyncio event loop based on tokio-rs
fn init_async_tokio(py: Python, m: &PyModule) -> PyResult<()> {
    let _ = env_logger::init();

    #[pyfn(m, "new_event_loop")]
    fn _new_event_loop(py: Python) -> PyResult<Py<TokioEventLoop>> {
        new_event_loop(py).into()
    }

    register_classes(py, m)
}


pub fn register_classes(_py: pyo3::Python, m: &pyo3::PyModule) -> pyo3::PyResult<()> {
    m.add_class::<event_loop::TokioEventLoop>()?;
    m.add_class::<pytask::PyTask>()?;
    m.add_class::<pyfuture::PyFuture>()?;
    m.add_class::<pybytes::PyBytes>()?;
    m.add_class::<handle::PyHandle>()?;
    m.add_class::<server::TokioServer>()?;
    m.add_class::<socket::Socket>()?;
    m.add_class::<transport::PyTcpTransport>()?;

    //m.add_class::<http::PyRequest>(py)?;
    //m.add_class::<http::StreamReader>(py)?;
    //m.add_class::<http::RawHeaders>(py)?;
    //m.add_class::<http::Url>(py)?;
    //m.add_class::<http::PayloadWriter>(py)?;
    //m.add_class::<http::pytransport::PyHttpTransport>(py)?;

    // touch classes
    let _ = Classes.Exception;

    Ok(())
}
