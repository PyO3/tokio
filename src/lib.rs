#![feature(proc_macro, specialization)]
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
#[macro_use] extern crate log;
#[macro_use] extern crate pyo3;
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

pub use pyo3::*;
pub use utils::{Classes, PyLogger, with_py};
pub use pybytes::PyBytes;
pub use pyfuture::{PyFuture, PyFuturePtr};
pub use pytask::{PyTask, PyTaskPtr};
pub use handle::PyHandle;
pub use event_loop::{TokioEventLoop, TokioEventLoopPtr, new_event_loop};
pub use server::create_server;
pub use client::create_connection;


py_module_init!(tokio, PyInit__tokio, |py, m| {
    let _ = env_logger::init();

    m.add(py, "__doc__", "Asyncio event loop based on tokio-rs")?;
    m.add(py, "new_event_loop", py_fn!(py, new_event_loop()))?;

    register_classes(py, m)?;
    Ok(())
});


pub fn register_classes(py: pyo3::Python, m: &pyo3::PyModule) -> pyo3::PyResult<()> {
    m.add_class::<event_loop::TokioEventLoop>(py)?;
    m.add_class::<pytask::PyTask>(py)?;
    m.add_class::<pytask::PyTaskIter>(py)?;
    m.add_class::<pyfuture::PyFuture>(py)?;
    m.add_class::<pyfuture::PyFutureIter>(py)?;
    m.add_class::<pybytes::PyBytes>(py)?;
    m.add_class::<handle::PyHandle>(py)?;
    m.add_class::<server::TokioServer>(py)?;
    m.add_class::<socket::Socket>(py)?;
    m.add_class::<transport::PyTcpTransport>(py)?;

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
