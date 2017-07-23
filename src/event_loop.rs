#![allow(unused_variables)]

use std::io;
use std::net;
use std::borrow::{Borrow, BorrowMut};
use std::cell::Cell;
use std::error::Error;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::fmt::Write;
use std::str::FromStr;
use std::path::Path;
use std::os::raw::c_int;
use std::os::unix;
use std::os::unix::io::{RawFd, FromRawFd};

use libc;
use pyo3::*;
use futures::{future, sync, unsync, Async, Future, Stream};
use futures::sync::{oneshot};
use tokio_core::reactor::{self, CoreId, Remote};
use tokio_signal;
use tokio_signal::unix::Signal;
use tokio_core::net::TcpStream;
use tokio_uds::{UnixStream, UnixListener};

use {PyFut, PyFuture, PyTask, PyTaskFut};
use addrinfo;
use client;
use handle::PyHandle;
use fd;
use fut::{Until, UntilError};
//use http;
use signals;
use server;
use utils::{self, with_py, Classes};
use pyunsafe::{GIL, Core, Handle, OneshotSender};
use transport;
use callbacks;


thread_local!(
    pub static ID: Cell<Option<CoreId>> = Cell::new(None);
);

pub fn new_event_loop(py: Python) -> PyResult<Py<TokioEventLoop>> {
    let core = reactor::Core::new().unwrap();
    let handle = core.handle();
    let remote = core.remote();
    let signals = signals::Signals::new(&handle);
    let cbs = Box::new(callbacks::Callbacks::new());
    let cbs_ptr: *mut callbacks::Callbacks = cbs.as_ref() as *const _ as *mut _;
    handle.spawn(cbs);

    py.init(|t| TokioEventLoop{
        token: t,
        id: Some(core.id()),
        core: Some(Core::new(core)),
        handle: Handle::new(handle),
        remote: remote,
        instant: Instant::now(),
        lookup: Some(addrinfo::start_workers(3)),
        runner: None,
        executor: None,
        exception_handler: py.None(),
        slow_callback_duration: 100,
        debug: false,
        current_task: None,
        signals: signals,
        readers: HashMap::new(),
        writers: HashMap::new(),
        callbacks: cbs_ptr,
    })
}

pub fn thread_safe_check(py: Python, id: &Option<CoreId>) -> Option<PyErr> {
    if let &Some(id) = id {
        let check = ID.with(|cell| {
            match cell.borrow().get() {
                None => true,
                Some(ref curr_id) => return *curr_id == id,
            }
        });

        if !check {
            Some(PyErr::new::<exc::RuntimeError, _>(
                py, "Non-thread-safe operation invoked on an event loop \
                     other than the current one"))
        } else {
            None
        }
    } else {
        Some(PyErr::new::<exc::RuntimeError, _>(py, "Event loop is closed"))
    }
}

#[derive(Debug)]
enum RunStatus {
    Stopped,
    CtrlC,
    Error,
    PyRes(PyResult<PyObject>),
}


#[py::class]
pub struct TokioEventLoop {
    token: PyToken,
    id: Option<CoreId>,
    core: Option<Core>,
    handle: Handle,
    remote: Remote,
    instant: Instant,
    lookup: Option<addrinfo::LookupWorkerSender>,
    runner: Option<oneshot::Sender<PyResult<()>>>,
    executor: Option<PyObject>,
    exception_handler: PyObject,
    slow_callback_duration: u64,
    debug: bool,
    current_task: Option<PyObject>,
    signals: sync::mpsc::UnboundedSender<signals::SignalsMessage>,
    readers: HashMap<c_int, OneshotSender<()>>,
    writers: HashMap<c_int, OneshotSender<()>>,
    callbacks: *mut callbacks::Callbacks,
}

#[py::methods]
impl TokioEventLoop {

    ///
    /// Return the currently running task in an event loop or None.
    ///
    pub fn current_task(&self, py: Python) -> PyResult<PyObject>
    {
        match self.current_task {
            Some(ref task) => Ok(task.clone_ref(py)),
            None => Ok(py.None())
        }
    }

    ///
    /// Create a Future object attached to the loop.
    ///
    fn create_future(&self, py: Python) -> PyResult<Py<PyFuture>>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        PyFuture::new(py, self.into())
    }

    ///
    /// Schedule a coroutine object.
    ///
    /// Return a task object.
    ///
    fn create_task(&self, py: Python, coro: &PyObjectRef) -> PyResult<PyObject>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        if let Some(fut) = PyFuture::try_downcast_from(coro) {
            return Ok(fut.into())
        }
        Ok(PyTask::new(py, coro.into(), &self)?.into())
    }

    ///
    /// Return the time according to the event loop's clock.
    ///
    /// This is a float expressed in seconds since event loop creation.
    ///
    fn time(&self) -> PyResult<f64>
    {
        let time = self.instant.elapsed();
        Ok(time.as_secs() as f64 + (time.subsec_nanos() as f64 / 1_000_000_000.0))
    }

    ///
    /// Return the time according to the event loop's clock (milliseconds)
    ///
    fn millis(&self) -> PyResult<u64>
    {
        let time = self.instant.elapsed();
        Ok(time.as_secs() * 1000 + (time.subsec_nanos() as u64 / 1_000_000))
    }

    ///
    /// def call_soon(self, callback, *args):
    ///
    /// Arrange for a callback to be called as soon as possible.
    ///
    /// This operates as a FIFO queue: callbacks are called in the
    /// order in which they are registered.  Each callback will be
    /// called exactly once.
    ///
    /// Any positional arguments after the callback will be passed to
    /// the callback when it is called.
    ///
    #[args(args="*", kwargs="**")]
    fn call_soon(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                 -> PyResult<PyObject>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        if args.len() < 1 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 1 arguments"))
        } else {
            // get params
            let callback = args.get_item(0).into();

            let h = PyHandle::new(py, &self, callback, args.split_from(1))?;
            h.call_soon(py, &self);
            Ok(h.into())
        }
    }

    ///
    /// def call_soon_threadsafe(self, callback, *args):
    ///
    /// Like call_soon(), but thread-safe.
    ///
    #[args(args="*", kwargs="**")]
    fn call_soon_threadsafe(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                            -> PyResult<PyObject>
    {
        if args.len() < 1 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 1 arguments"))
        } else {
            // get params
            let callback = args.get_item(0).into();

            // create handle and schedule work
            let h = PyHandle::new(py, &self, callback, args.split_from(1))?;
            h.call_soon_threadsafe(py, &self);

            Ok(h.into())
        }
    }

    ///
    /// def call_later(self, delay, callback, *args)
    ///
    /// Arrange for a callback to be called at a given time.
    ///
    /// Return a Handle: an opaque object with a cancel() method that
    /// can be used to cancel the call.
    ///
    /// The delay can be an int or float, expressed in seconds.  It is
    /// always relative to the current time.
    ///
    /// Each callback will be called exactly once.  If two callbacks
    /// are scheduled for exactly the same time, it undefined which
    /// will be called first.
    ///
    /// Any positional arguments after the callback will be passed to
    /// the callback when it is called.
    ///
    #[args(args="*", kwargs="**")]
    fn call_later(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                  -> PyResult<PyObject>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        if args.len() < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let callback = args.get_item(1).into();
            let delay = utils::parse_millis(py, "delay", args.get_item(0))?;

            // create handle and schedule work
            let mut h = PyHandle::new(py, &self, callback, args.split_from(2))?;
            if delay == 0 {
                h.call_soon(py, &self);
            } else {
                h.call_later(py, &self, Duration::from_millis(delay));
            };
            Ok(h.into())
        }
    }

    ///
    /// def call_at(self, when, callback, *args):
    ///
    /// Like call_later(), but uses an absolute time.
    ///
    /// Absolute time corresponds to the event loop's time() method.
    ///
    #[args(args="*", kwargs="**")]
    fn call_at(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>) -> PyResult<PyObject>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        if args.len() < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let callback = args.get_item(1).into();

            // create handle and schedule work
            let mut h = PyHandle::new(py, &self, callback, args.split_from(2))?;

            // calculate delay
            if let Some(when) = utils::parse_seconds(py, "when", args.get_item(0).into())? {
                h.call_later(py, self, when - self.instant.elapsed());
            } else {
                h.call_soon(py, self);
            }
            Ok(h.into())
        }
    }

    ///
    /// def add_signal_handler(self, sig, callback, *args)
    ///
    /// Add a handler for a signal.  UNIX only.
    ///
    /// Raise ValueError if the signal number is invalid or uncatchable.
    /// Raise RuntimeError if there is a problem setting up the handler.
    #[args(args="*", kwargs="**")]
    fn add_signal_handler(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                          -> PyResult<()>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        if args.len() < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let sig = args.get_item(0).extract::<c_int>()?;
            let callback: PyObject = args.get_item(1).into();

            // coroutines are not allowed as handlers
            let iscoro: bool = Classes.Coroutines.as_ref(py).call(
                "iscoroutine", (callback.clone_ref(py),), None)?.extract()?;
            let iscorof: bool = Classes.Coroutines.as_ref(py).call(
                "iscoroutinefunction", (callback.clone_ref(py),), None)?.extract()?;
            if iscoro || iscorof {
                return Err(PyErr::new::<exc::TypeError, _>(
                    py, "coroutines cannot be used with add_signal_handler()"))
            }

            // create signal
            let signal = match Signal::new(sig, self.href()).poll() {
                Ok(Async::Ready(signal)) => signal,
                Ok(Async::NotReady) => unreachable!(),
                Err(err) =>
                    return Err(PyErr::new::<exc::RuntimeError, _>(
                        py, format!("sig {} cannot be caught", sig)))
            };

            // create handle and schedule work
            let h = PyHandle::new(py, &self, callback, args.split_from(2))?;

            // register signal handler
            let _ = self.signals.send(signals::SignalsMessage::Add(sig, signal, h));

            Ok(())
        }
    }

    ///
    /// Remove a handler for a signal.  UNIX only.
    ///
    /// Return True if a signal handler was removed, False if not.
    fn remove_signal_handler(&self, py: Python, sig: c_int) -> PyResult<bool>
    {
        // un-register signal handler
        let _ = self.signals.send(signals::SignalsMessage::Remove(sig));

        Ok(true)
    }

    #[args(args="*", kwargs="**")]
    fn _add_reader(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                   -> PyResult<()>
    {
        if args.len() < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let fd: c_int = args.get_item(0).extract()?;
            let callback = args.get_item(1).into();

            // create handle
            let h = PyHandle::new(py, &self, callback, args.split_from(2))?;
            match fd::PyFdHandle::reader(fd, self.href(), h) {
                Ok(tx) => {
                    self.readers.insert(fd, OneshotSender::new(tx));
                    Ok(())
                },
                Err(err) => Err(err.to_pyerr(py)),
            }
        }
    }

    fn _remove_reader(&mut self, py: Python, fd: c_int) -> PyResult<bool>
    {
        if let Some(tx) = self.readers.remove(&fd) {
            let _ = tx.send(());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[args(args="*", kwargs="**")]
    fn _add_writer(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                   -> PyResult<()>
    {
        if args.len() < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let fd: c_int = args.get_item(0).extract()?;
            let callback = args.get_item(1).into();

            // create handle
            let h = PyHandle::new(py, &self, callback, args.split_from(2))?;
            match fd::PyFdHandle::writer(fd, self.href(), h) {
                Ok(tx) => {
                    self.writers.insert(fd, OneshotSender::new(tx));
                    Ok(())
                },
                Err(err) => Err(err.to_pyerr(py)),
            }
        }
    }

    fn _remove_writer(&mut self, py: Python, fd: c_int) -> PyResult<bool> {
        if let Some(tx) = self.writers.remove(&fd) {
            let _ = tx.send(());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Add a reader callback
    #[args(args="*", kwargs="**")]
    fn add_reader(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                  -> PyResult<()>
    {
        return self._add_reader(py, args, kwargs)
    }

    /// Remove a reader callback
    fn remove_reader(&mut self, py: Python, fd:c_int) -> PyResult<bool>
    {
        self._remove_reader(py, fd)
    }

    /// Add a writer callback
    #[args(args="*", kwargs="**")]
    fn add_writer(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                  -> PyResult<()>
    {
        return self._add_writer(py, args, kwargs)
    }

    /// Remove a writer callback
    fn remove_writer(&mut self, py: Python, fd: c_int)
                     -> PyResult<bool>
    {
        return self._remove_writer(py, fd)
    }

    /// Receive data from the socket.
    ///
    /// The return value is a bytes object representing the data received.
    /// The maximum amount of data to be received at once is specified by
    /// nbytes.
    ///
    /// This method is a coroutine.
    fn sock_recv(&self, py: Python, sock: &PyObjectRef, n: PyObject) -> PyResult<Py<PyFuture>>
    {
        let _ = self.is_socket_nonblocking(sock)?;

        // create readiness stream
        let fd = {
            let fd = self.get_socket_fd(sock)?;
            match fd::PyFdReadable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, self.into(), Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);
        let sock: PyObject = sock.into();

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let fut = fut_ready.as_mut(py);

            // fut cancelled
            if fut.is_cancelled() {
                return future::ok(Some(()));
            }

            let res = sock.as_ref(py).call_method("recv", (n.clone_ref(py),), None);

            match res {
                Err(err) => {
                    if err.matches(
                        py, (py.get_type::<exc::BlockingIOError>(),
                             py.get_type::<exc::InterruptedError>())) {
                        // skill blocking, continue
                        future::ok(None)
                    } else {
                        future::err(err)
                    }
                }
                Ok(result) => {
                    fut.set(py, Ok(result.into()));
                    future::ok(Some(()))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    fut_err.with_mut(|py, fut| fut.set(py, Err(err)));
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);

        Ok(fut)
    }

    /// Send data to the socket.
    ///
    /// The socket must be connected to a remote socket. This method continues
    /// to send data from data until either all data has been sent or an
    /// error occurs. None is returned on success. On error, an exception is
    /// raised, and there is no way to determine how much data, if any, was
    /// successfully processed by the receiving end of the connection.
    ///
    /// This method is a coroutine.
    fn sock_sendall(&self, py: Python, sock: &PyObjectRef, data: PyObject)
                    -> PyResult<Py<PyFuture>>
    {
        let _ = self.is_socket_nonblocking(sock)?;

        // data is empty, nothing to do
        if ! data.is_true(py)? {
            return Ok(PyFuture::done_res(py, self.into(), Ok(py.None()))?)
        }

        // create readyness stream for write operation
        let fd = {
            let fd = self.get_socket_fd(sock)?;
            match fd::PyFdWritable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, self.into(), Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, self.into())?;
        let fut_ready = fut.clone_ref(py);
        let fut_err = fut.clone_ref(py);
        let wrp: Py<PyList> = PyList::new(py, &[&data]).into();
        let sock: PyObject = sock.into();

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let fut = fut_ready.as_mut(py);
            let wrp_mut = wrp.as_mut(py);

            if fut.is_cancelled() {
                return future::ok(Some(()));
            }

            let data = wrp_mut.get_item(0);
            let res = sock.call_method(py, "send", (data.to_object(py),), None);

            match res {
                Err(err) => {
                    if err.matches(
                        py, (py.get_type::<exc::BlockingIOError>(),
                             py.get_type::<exc::InterruptedError>())) {
                        // skill blocking, continue
                        future::ok(None)
                    } else {
                        future::err(err)
                    }
                }
                Ok(result) => {
                    if let Ok(n) = result.as_ref(py).extract::<c_int>() {
                        let len = data.len().unwrap() as c_int;
                        if n == len {
                            // all data is sent
                            fut.set(py, Ok(py.None()));
                            future::ok(Some(()))
                        } else {
                            // some data got send
                            let slice = PySlice::new(py, n as isize, len as isize, 1);
                            match data.call_method("__getitem__", (slice,), None) {
                                Ok(data) => {
                                    match wrp_mut.set_item(0, data) {
                                        Ok(_) => future::ok(None),
                                        Err(e) => future::err(e)
                                    }
                                },
                                Err(e) => future::err(e)
                            }
                        }
                    } else {
                        // exception
                        future::err(PyErr::new::<exc::OSError, _>(
                            py, format!("sendall call failed {}", sock.as_ref(py))))
                    }
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    fut_err.with_mut(|py, fut| fut.set(py, Err(err)));
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    /// Connect to a remote socket at address.
    ///
    /// This method is a coroutine.
    fn sock_connect(&self, py: Python, sock: &PyObjectRef, address: &PyObjectRef)
                    -> PyResult<Py<PyFuture>>
    {
        let _ = self.is_socket_nonblocking(sock)?;

        //if not hasattr(socket, 'AF_UNIX') or sock.family != socket.AF_UNIX:
        //resolved = base_events._ensure_resolved(
        //    address, family=sock.family, proto=sock.proto, loop=self)
        //    if not resolved.done():
        //yield from resolved
        //    _, _, _, _, address = resolved.result()[0]

        // try to connect
        let res = sock.call_method("connect", (address,), None);

        // if connect is blocking, create readiness stream
        let fd = match res {
            Ok(_) => {
                return Ok(PyFuture::done_fut(py, self.into(), py.None())?);
            },
            Err(err) => {
                if ! err.matches(py, (py.get_type::<exc::BlockingIOError>(),
                                      py.get_type::<exc::InterruptedError>())) {
                    return Ok(PyFuture::done_res(py, self.into(), Err(err))?);
                }
                let fd = self.get_socket_fd(&sock)?;
                match fd::PyFdWritable::new(fd, self.href()) {
                    Err(err) => return Ok(
                        PyFuture::done_res(py, self.into(), Err(err.to_pyerr(py)))?),
                    Ok(fd) => fd
                }
            }
        };

        // wait until sock get connected
        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);
        let sock: PyObject = sock.into();
        let address: PyObject = address.into();

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let fut = fut_ready.as_mut(py);

            if fut.is_cancelled() {
                return future::ok(Some(()))
            }

            let res = sock.call_method(
                py, "getsockopt", (libc::SOL_SOCKET, libc::SO_ERROR), None);

            match res {
                Err(err) => {
                    if err.matches(
                        py, (py.get_type::<exc::BlockingIOError>(),
                             py.get_type::<exc::InterruptedError>())) {
                        // skill blocking, continue
                        future::ok(None)
                    } else {
                        // actual python exception
                        future::err(err)
                    }
                }
                Ok(result) => {
                    if let Ok(err) = result.as_ref(py).extract::<i32>() {
                        if err == 0 {
                            fut.set(py, Ok(py.None()));
                            return future::ok(Some(()))
                        }
                    }

                    // Jump to any except clause below.
                    future::err(PyErr::new::<exc::OSError, _>(
                        py, (result, format!("Connect call failed {}", address.as_ref(py)))))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    fut_err.with_mut(|py, fut| fut.set(py, Err(err)));
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    /// Accept a connection.
    ///
    /// The socket must be bound to an address and listening for connections.
    /// The return value is a pair (conn, address) where conn is a new socket
    /// object usable to send and receive data on the connection, and address
    /// is the address bound to the socket on the other end of the connection.
    ///
    /// This method is a coroutine.
    fn sock_accept(&self, py: Python, sock: &PyObjectRef) -> PyResult<Py<PyFuture>> {
        let _ = self.is_socket_nonblocking(sock)?;

        // create readiness stream
        let fd = {
            let fd = self.get_socket_fd(sock)?;
            match fd::PyFdReadable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, self.into(), Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);
        let sock: PyObject = sock.into();

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let mut fut = fut_ready.as_mut(py);

            // fut cancelled
            if fut.is_cancelled() {
                return future::ok(Some(()));
            }
            let res = sock.call_method(py, "accept", NoArgs, None);

            match res {
                Err(err) => {
                    if err.matches(
                        py, (py.get_type::<exc::BlockingIOError>(),
                             py.get_type::<exc::InterruptedError>())) {
                        // skill blocking, continue
                        future::ok(None)
                    } else {
                        future::err(err)
                    }
                }
                Ok(result) => {
                    if let Some(result) = PyTuple::try_downcast_from(result.as_ref(py)) {
                        let _ = result.get_item(0).call_method("setblocking", (false,), None);
                    }

                    fut.set(py, Ok(result.into()));
                    future::ok(Some(()))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    fut_err.with_mut(|py, fut| fut.set(py, Err(err)));
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    ///
    /// Stop running the event loop.
    ///
    fn stop(&mut self) -> PyResult<bool> {
        let runner = self.runner.take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(Ok(()));
                Ok(true)
            },
            None => Ok(false),
        }
    }

    fn is_running(&self) -> PyResult<bool> {
        Ok(self.runner.is_some())
    }

    fn is_closed(&self) -> PyResult<bool> {
        Ok(self.id.is_none())
    }

    ///
    /// Close the event loop. The event loop must not be running.
    ///
    fn close(&mut self, py: Python) -> PyResult<()> {
        if let Ok(running) = self.is_running() {
            if running {
                return Err(
                    PyErr::new::<exc::RuntimeError, _>(
                        py, "Cannot close a running event loop"));
            }
        }

        // shutdown executor
        if let Some(executor) = self.executor.take() {
            let kwargs = PyDict::new(py);
            kwargs.set_item("wait", false)?;
            let _ = executor.call_method(py, "shutdown", NoArgs, Some(&kwargs));
        }

        // drop CORE
        self.core.take();

        if let Some(id) = self.id.take() {
            ID.with(|mut cell| {
                let curr = if let Some(gid) = cell.borrow().get() {
                    gid == id
                } else {
                    false
                };
                if curr {
                    cell.borrow_mut().take();
                }
            });
        }

        // drop address lookup workers
        self.lookup.take();

        Ok(())
    }

    ///
    /// Executor api
    ///
    #[args(args="*", kwargs="**")]
    fn run_in_executor(&mut self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                       -> PyResult<&PyObjectRef>
    {
        if self.debug {
            if let Some(err) = thread_safe_check(py, &self.id) {
                return Err(err)
            }
        }

        // get params
        if args.len() < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }

        let evloop: PyObject = self.into();
        let executor = args.get_item(0);
        let args = args.split_from(1);

        // get or create default executor
        let fut = if executor.is_none() {
            let executor = if let Some(ref ex) = self.executor {
                ex.as_ref(py)
            } else {
                let concurrent = py.import("concurrent.futures")?;
                self.executor = Some(
                    concurrent.call("ThreadPoolExecutor", NoArgs, None)?.into());
                self.executor.as_ref().unwrap().as_ref(py)
            };
            // submit function
            executor.call_method("submit", args, None)?
        } else {
            // submit function
            executor.call_method("submit", args, None)?
        };

        // wrap_future
        let kwargs = PyDict::new(py);
        kwargs.set_item("loop", evloop)?;
        Classes.Asyncio.as_ref(py).call("wrap_future", (fut,), Some(&kwargs))
    }

    fn set_default_executor(&mut self, py: Python, executor: PyObject) -> PyResult<()> {
        self.executor = Some(executor);
        Ok(())
    }

    /// return list of tuples
    /// item = (family, type, proto, canonname, sockaddr)
    /// sockaddr(IPV4) = (address, port)
    /// sockaddr(IPV6) = (address, port, flow info, scope id)
    #[args(args="*", kwargs="**")]
    fn getaddrinfo(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                   -> PyResult<Py<PyFuture>> {
        // parse params
        let len = args.len();
        if len < 1 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "host is required"))
        }
        if len < 2 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "port is required"))
        }

        // parse host (string, unicode, bytes, None)
        let host_arg = args.get_item(0);
        let host = if host_arg.is_none() {
            None
        } else if let Some(host) = PyString::try_downcast_from(host_arg) {
            Some(String::from(host.to_string_lossy()))
        } else {
            if let Ok(host) = PyString::from_object(host_arg, "utf-8\0", "strict\0") {
                Some(String::from(host.to_string_lossy()))
            } else {
                return Err(PyErr::new::<exc::TypeError, _>(
                    py, format!("string or none type is required as host, got: {:?}", host_arg)))
            }
        };

        // parse port (int, string, unicode or none)
        let port_arg = args.get_item(1);
        let port = if port_arg.is_none() {
            None
        } else if let Some(port) = PyString::try_downcast_from(port_arg) {
            Some(String::from(port.to_string_lossy()))
        } else if let Ok(port) = port_arg.extract::<u16>() {
            Some(port.to_string())
        } else {
            Some(String::from(
                PyString::from_object(&port_arg, "utf-8\0", "strict\0")?.to_string_lossy()))
        };

        let mut family: i32 = 0;
        let mut socktype: i32 = 0;
        let mut _proto: i32 = 0;
        let mut flags: i32 = 0;

        if let Some(kwargs) = kwargs {
            if let Some(f) = kwargs.get_item("family") {
                family = f.extract()?
            }
            if let Some(s) = kwargs.get_item("type") {
                socktype = s.extract()?
            }
            if let Some(p) = kwargs.get_item("proto") {
                _proto = p.extract()?
            }
            if let Some(f) = kwargs.get_item("flags") {
                flags = f.extract()?
            }
        }

        // result future
        let res = PyFuture::new(py, self.into())?;

        // create processing future
        let fut = res.clone_ref(py);
        let fut_err = res.clone_ref(py);

        // lookup process future
        let lookup = addrinfo::lookup(
            self.lookup.as_ref().unwrap(), host, port, family, flags,
            addrinfo::SocketType::from_int(socktype));

        // convert addr info to python comaptible  values
        let process = lookup.and_then(move |result| {
            fut.with_mut(|py, fut| {
                match result {
                    Err(ref err) => fut.set(py, Err(err.to_pyerr(py))),
                    Ok(ref addrs) => {
                        // create socket.gethostname compatible result
                        let list = PyList::empty(py);
                        for info in addrs {
                            let addr = match info.sockaddr {
                                net::SocketAddr::V4(addr) => {
                                    (format!("{}", addr.ip()), addr.port()).into_tuple(py)
                                }
                                net::SocketAddr::V6(addr) => {
                                    (format!("{}", addr.ip()),
                                     addr.port(), addr.flowinfo(), addr.scope_id(),
                                    ).into_tuple(py)
                                },
                            };

                            let cname = match info.canonname {
                                Some(ref cname) => PyString::new(py, cname.as_str()),
                                None => PyString::new(py, ""),
                            };

                            let item: PyObject = (info.family.to_int(),
                                                  info.socktype.to_int(),
                                                  info.protocol.to_int(),
                                                  cname, addr).into_tuple(py).into();
                            list.insert(list.len() as isize, item)
                                .expect("Except to succeed");
                        }
                        fut.set(py, Ok(list.into()));
                    },
                }
            });
            future::ok(())
        }).map_err(move |err| fut_err.with_mut(|py, fut| {
            let err = PyErr::new::<exc::RuntimeError, _>(py, "Unknown runtime error");
            fut.set(py, Err(err));
        }));

        // start task
        self.handle.spawn(process);

        Ok(res)
    }

    // TODO need rust version, use python code for now
    #[args(flags=0)]
    fn getnameinfo(&mut self, py: Python, sockaddr: PyObject, flags: i32)
                   -> PyResult<&PyObjectRef>
    {
        self.run_in_executor(
            py, (py.None(), Classes.GetNameInfo.clone_ref(py),
                 sockaddr, flags).into_tuple(py).as_ref(py), None)
    }

    fn connect_read_pipe(&self, py: Python, protocol_factory: PyObject, pipe: PyObject)
                         -> PyResult<Py<PyFuture>> {
        let protocol = protocol_factory.call(py, NoArgs, None)?;
        let waiter = PyFuture::new(py, self.into())?;

        // create unix transport
        let cls = Classes.UnixEvents.as_ref(py).get("_UnixReadPipeTransport")?;
        let transport: PyObject = cls.call(
            (self, pipe, protocol.clone_ref(py),
             waiter.clone_ref(py), py.None()), None)?.into();

        // wait for transport get ready
        let fut = PyFuture::new(py, self.into())?;
        let fut_ready = fut.clone_ref(py);
        let waiter: PyFut = waiter.into();

        self.href().spawn(
            waiter.then(move |res| {
                let gil = Python::acquire_gil();
                let py = gil.python();
                let mut fut = fut_ready.as_mut(py);

                match res {
                    Ok(res) => match res {
                        Ok(res) => fut.set(py, Ok((transport, protocol).to_object(py))),
                        Err(err) => {
                            let _ = transport.as_ref(py).call_method("close", NoArgs, None);
                            fut.set(py, Err(err));
                        }
                    },
                    Err(_) => {
                        let _ = fut.cancel(py);
                    }
                }
                Ok(())
            }));

        Ok(fut)
    }

    fn connect_write_pipe(&self, py: Python, protocol_factory: &PyObjectRef, pipe: PyObject)
                          -> PyResult<Py<PyFuture>> {
        let protocol: PyObject = protocol_factory.call(NoArgs, None)?.into();
        let waiter = PyFuture::new(py, self.into())?;

        // create unix transport
        let cls = Classes.UnixEvents.as_ref(py).get("_UnixWritePipeTransport")?;
        let transport: PyObject = cls.call(
            (self, pipe, protocol.clone_ref(py),
             waiter.clone_ref(py), py.None()), None)?.into();

        // wait for transport get ready
        let fut = PyFuture::new(py, self.into())?;
        let fut_ready = fut.clone_ref(py);
        let waiter: PyFut = waiter.into();

        self.href().spawn(
            waiter.then(move |res| {
                let gil = Python::acquire_gil();
                let py = gil.python();
                let mut fut = fut_ready.as_mut(py);

                match res {
                    Ok(res) => match res {
                        Ok(res) => fut.set(py, Ok((transport, protocol).to_object(py))),
                        Err(err) => {
                            let _ = transport.as_ref(py).call_method("close", NoArgs, None);
                            fut.set(py, Err(err));
                        }
                    },
                    Err(_) => {
                        let _ = fut.cancel(py);
                    }
                }
                Ok(())
            }));

        Ok(fut)
    }

    ///
    /// subprocess_shell
    ///
    fn _socketpair(&self, py: Python) -> PyResult<PyObject> {
        Ok(Classes.Socket.as_ref(py).call("socketpair", NoArgs, None)?.into())
    }

    fn _child_watcher_callback(&self, py: Python, pid: PyObject,
                               returncode: PyObject, transp: &PyObjectRef) -> PyResult<PyObject>
    {
        let process_exited = transp.getattr("_process_exited")?;
        self.call_soon_threadsafe(
            py, (process_exited, returncode).into_tuple(py).as_ref(py), None)
    }

    #[args(args="*", kwargs="**")]
    fn subprocess_shell(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                        -> PyResult<Py<PyFuture>> {
        if args.len() < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }

        let protocol_factory = args.get_item(0);
        let cmd = args.get_item(1);
        let kwargs = if let Some(kw) = kwargs {
            kw
        } else {
            PyDict::new(py)
        };

        let stdin: i32 = if let Some(val) = kwargs.get_item("stdin") {
            let _ = kwargs.del_item("stdin")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let stdout: i32 = if let Some(val) = kwargs.get_item("stdout") {
            let _ = kwargs.del_item("stdout")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let stderr: i32 = if let Some(val) = kwargs.get_item("stderr") {
            let _ = kwargs.del_item("stderr")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let newlines = if let Some(val) = kwargs.get_item("universal_newlines") {
            let _ = kwargs.del_item("universal_newlines")?;
            if val.is_none() {
                false
            } else {
                val.extract::<bool>()?
            }
        } else {
            false
        };
        let shell = if let Some(val) = kwargs.get_item("shell") {
            let _ = kwargs.del_item("shell")?;
            if val.is_none() {
                true
            } else {
                val.extract::<bool>()?
            }
        } else {
            true
        };
        let bufsize = if let Some(val) = kwargs.get_item("bufsize") {
            let _ = kwargs.del_item("bufsize")?;
            if val.is_none() {
                0
            } else {
                val.extract::<i32>()?
            }
        } else {
            0
        };

        //if not isinstance(cmd, (bytes, str)):
        //raise ValueError("cmd must be a string")

        if newlines {
            return Err(PyErr::new::<exc::ValueError, _>(
                py, "universal_newlines must be False"))
        }
        if ! shell {
            return Err(PyErr::new::<exc::ValueError, _>(py, "shell must be True"))
        }
        if bufsize != 0 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "bufsize must be 0"))
        }

        let protocol: PyObject = protocol_factory.call(NoArgs, None)?.into();

        let ev = Classes.UnixEvents.as_ref(py).get("_UnixSelectorEventLoop")?;
        let coro = ev.call_method(
            "_make_subprocess_transport",
            (self, protocol.clone_ref(py), cmd, true,
             stdin, stdout, stderr, bufsize), Some(kwargs))?;

        let fut = PyFuture::new(py, self.into())?;
        let fut_ready = fut.clone_ref(py);

        let task: PyTaskFut = PyTask::new(py, coro.into(), &self)?.into();

        self.href().spawn(task.then(move |res| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let fut = fut_ready.as_mut(py);

            match res {
                Ok(res) => match res {
                    Ok(transport) => {
                        let result = (transport, protocol).to_object(py);
                        fut.set(py, Ok(result));
                    },
                    Err(err) => fut.set(py, Err(err))
                },
                Err(_) => {
                    let _ = fut.cancel(py);
                }
            }
            Ok(())
        }));

        Ok(fut)
    }

    ///
    /// subprocess_exec
    ///
    #[args(args="*", kwargs="**")]
    fn subprocess_exec(&self, py: Python, args: &PyTuple, kwargs: Option<&PyDict>)
                       -> PyResult<Py<PyFuture>>
    {
        if args.len() < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }

        let protocol_factory = args.get_item(0);
        let kwargs = if let Some(kw) = kwargs {
            kw
        } else {
            PyDict::new(py)
        };

        let stdin: i32 = if let Some(val) = kwargs.get_item("stdin") {
            let _ = kwargs.del_item("stdin")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let stdout: i32 = if let Some(val) = kwargs.get_item("stdout") {
            let _ = kwargs.del_item("stdout")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let stderr: i32 = if let Some(val) = kwargs.get_item("stderr") {
            let _ = kwargs.del_item("stderr")?;
            if val.is_none() {
                -1
            } else {
                val.extract()?
            }
        } else {
            -1
        };
        let newlines = if let Some(val) = kwargs.get_item("universal_newlines") {
            let _ = kwargs.del_item("universal_newlines")?;
            if val.is_none() {
                false
            } else {
                val.extract::<bool>()?
            }
        } else {
            false
        };
        let shell = if let Some(val) = kwargs.get_item("shell") {
            let _ = kwargs.del_item("shell")?;
            if val.is_none() {
                false
            } else {
                val.extract::<bool>()?
            }
        } else {
            false
        };
        let bufsize = if let Some(val) = kwargs.get_item("bufsize") {
            let _ = kwargs.del_item("bufsize")?;
            if val.is_none() {
                0
            } else {
                val.extract::<i32>()?
            }
        } else {
            0
        };

        if newlines {
            return Err(PyErr::new::<exc::ValueError, _>(
                py, "universal_newlines must be False"))
        }
        if shell {
            return Err(PyErr::new::<exc::ValueError, _>(py, "shell must be False"))
        }
        if bufsize != 0 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "bufsize must be 0"))
        }

        let popen_args = args.split_from(1);
        let protocol: PyObject = protocol_factory.call(NoArgs, None)?.into();

        let ev = Classes.UnixEvents.as_ref(py).get("_UnixSelectorEventLoop")?;
        let coro = ev.call_method(
            "_make_subprocess_transport",
            (self, protocol.clone_ref(py), popen_args, false,
             stdin, stdout, stderr, bufsize), Some(kwargs))?;

        let fut = PyFuture::new(py, self.into())?;
        let fut_ready = fut.clone_ref(py);

        let task: PyTaskFut = PyTask::new(py, coro.into(), &self)?.into();

        self.href().spawn(task.then(move |res| {
            let gil = Python::acquire_gil();
            let py = gil.python();
            let fut = fut_ready.as_mut(py);

            match res {
                Ok(res) => match res {
                    Ok(transport) => {
                        let result = (transport, protocol).to_object(py);

                        let _ = fut.set(py, Ok(result));
                    },
                    Err(err) => {
                        let _ = fut.set(py, Err(err));
                    }
                },
                Err(_) => {
                    let _ = fut.cancel(py);
                }
            }
            Ok(())
        }));

        Ok(fut)
    }

    ///
    /// Create a TCP server.
    ///
    /// The host parameter can be a string, in that case the TCP server is bound
    /// to host and port.
    ///
    /// The host parameter can also be a sequence of strings and in that case
    /// the TCP server is bound to all hosts of the sequence. If a host
    /// appears multiple times (possibly indirectly e.g. when hostnames
    /// resolve to the same IP address), the server is only bound once to that
    /// host.
    ///
    /// Return a Server object which can be used to stop the service.
    ///
    #[args("*", family=0, flags="addrinfo::AI_PASSIVE", backlog=100,
           reuse_address=true, reuse_port=true)]
    fn create_server(&self, py: Python, protocol_factory: PyObject,
                     host: Option<String>, port: Option<u16>,
                     family: i32, flags: i32,
                     sock: Option<&PyObjectRef>, backlog: i32, ssl: Option<PyObject>,
                     reuse_address: bool, reuse_port: bool)
                     -> PyResult<Py<PyFuture>>
    {
        self.create_server_helper(
            py, protocol_factory, host, port, family, flags,
            sock, backlog, ssl, reuse_address, reuse_port, transport::tcp_transport_factory)
    }

    /*#[defaults(family=0, flags="addrinfo::AI_PASSIVE", backlog=100,
               reuse_address=true, reuse_port=true)]
    fn create_http_server(&self, py: Python, protocol_factory: PyObject,
                          host: Option<PyString>, port: Option<u16>,
                          family: i32, flags: i32,
                          sock: Option<PyObject>,
                          backlog: i32, ssl: Option<PyObject>,
                          reuse_address: bool, reuse_port: bool) -> PyResult<PyFuturePtr>
    {
        self.create_server_helper(
            py, protocol_factory, host, port, family, flags,
            sock, backlog, ssl, reuse_address, reuse_port, http::http_transport_factory)
    }*/

    /// Connect to a TCP server.
    ///
    /// Create a streaming transport connection to a given Internet host and
    /// port: socket family AF_INET or socket.AF_INET6 depending on host (or
    /// family if specified), socket type SOCK_STREAM. protocol_factory must be
    /// a callable returning a protocol instance.
    ///
    /// This method is a coroutine which will try to establish the connection
    /// in the background.  When successful, the coroutine returns a
    /// (transport, protocol) pair.
    ///
    #[args("*", family=0, proto=0, flags="addrinfo::AI_PASSIVE")]
    fn create_connection(&self, py: Python, protocol_factory: PyObject,
                         host: Option<String>, port: Option<u16>,
                         ssl: Option<PyObject>,
                         family: i32, proto: i32, flags: i32,
                         sock: Option<&PyObjectRef>,
                         local_addr: Option<PyObject>,
                         server_hostname: Option<PyObject>) -> PyResult<Py<PyFuture>> {
        match (&server_hostname, &ssl) {
            (&Some(_), &None) =>
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "server_hostname is only meaningful with ssl")),
            (&None, &Some(_)) => {
                // Use host as default for server_hostname.  It is an error
                // if host is empty or not set, e.g. when an
                // already-connected socket was passed or when only a port
                // is given.  To avoid this error, you can pass
                // server_hostname='' -- this will bypass the hostname
                // check.  (This also means that if host is a numeric
                // IP/IPv6 address, we will attempt to verify that exact
                // address; this will probably fail, but it is possible to
                // create a certificate for a specific IP address, so we
                // don't judge it here.)
                if let None = host {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, "You must set server_hostname when using ssl without a host"));
                }
            }
            // server_hostname = host
            _ => (),
        }

        // server hostname for ssl validation
        let server_hostname = match server_hostname {
            Some(s) => Some(s),
            None => match host {
                Some(ref h) => Some(h.to_object(py)),
                None => None,
            }
        };

        let conn = if let (&None, &None) = (&host, &port) {
            let sock = if let Some(sock) = sock {
                // Try to use supplied python connected socket object
                if ! self.is_stream_socket(sock)? {
                    return Ok(PyFuture::done_res(
                        py, self.into(),
                        Err(PyErr::new::<exc::ValueError, _>(
                            py, format!("A Stream Socket was expected, got {:?}", sock))))?)
                }

                // check if socket is UNIX domain socket
                if self.is_uds_socket(sock)? {
                    return self.create_unix_connection(
                        py, protocol_factory, None, ssl, Some(sock), server_hostname);
                }

                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "host and port was not specified and no sock specified"));
            };

            let fileno = self.get_socket_fd(sock)?;
            let sockaddr = self.addr_from_socket(sock)?;

            // create TcpStream object
            let stream = unsafe {
                net::TcpStream::from_raw_fd(fileno as RawFd)
            };

            // tokio stream
            let stream = match TcpStream::from_stream(stream, self.href()) {
                Ok(stream) => stream,
                Err(err) => return Err(err.to_pyerr(py)),
            };

            let waiter = PyFuture::new(py, self.into())?;
            future::Either::A(
                client::create_sock_connection(
                    protocol_factory, self.into(),
                    stream, sockaddr, ssl, server_hostname, waiter))
        } else {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "host/port and sock can not be specified at the same time"))
            }

            // exctract hostname
            let port = port.map(|p| p.to_string());

            let evloop = self.into();
            let handle = self.handle.clone();
            let waiter = PyFuture::new(py, self.into())?;

            // resolve addresses and connect
            let fut = addrinfo::lookup(self.lookup.as_ref().unwrap(),
                                       host, port,
                                       family, flags, addrinfo::SocketType::Stream)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err.description()))
                .and_then(move |addrs| match addrs {
                    Err(err) => future::Either::A(
                        future::err(
                            io::Error::new(io::ErrorKind::Other, err.description()))),
                    Ok(addrs) => {
                        if addrs.is_empty() {
                            future::Either::A(future::err(
                                io::Error::new(
                                    io::ErrorKind::Other, "getaddrinfo() returned empty list")))
                        } else {
                            future::Either::B(
                                client::create_connection(
                                    protocol_factory, evloop,
                                    addrs, ssl, server_hostname, waiter))
                        }
                    }
                });

            future::Either::B(fut)
        };

        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_conn = fut.clone_ref(py);

        self.handle.spawn(
            conn
            // set exception to future
                .map_err(move |e| fut_err.with_mut(
                    |py, fut| fut.set(py, Err(e.to_pyerr(py)))))
            // set transport and protocol
                .map(move |res| fut_conn.with_mut(
                    |py, fut| fut.set(py, Ok(res.into_tuple(py).into()))))
        );
        Ok(fut)
    }

    ///
    /// Connect to a UDS client.
    ///
    #[args(backlog=100)]
    fn create_unix_server(&self, py: Python,
                          protocol_factory: PyObject,
                          path: Option<&str>,
                          sock: Option<&PyObjectRef>,
                          backlog: i32,
                          ssl: Option<PyObject>) -> PyResult<Py<PyFuture>>
    {
        let lst = if let Some(path) = path {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "path and sock can not be specified at the same time"))
            }

            UnixListener::bind(Path::new(path), self.href()).map_err(|e| e.to_pyerr(py))?
        } else {
            let sock = if let Some(sock) = sock {
                if ! self.is_uds_socket(sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A UNIX Domain Stream Socket was expected, got {:?}", sock)))
                }
                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "no path and sock were specified"))
            };

            // listen
            sock.call_method("listen", (backlog,), None)?;

            let fileno = self.get_socket_fd(sock)?;

            // create UnixListener object
            let lst = unsafe {
                unix::net::UnixListener::from_raw_fd(fileno as RawFd)
            };

            UnixListener::from_listener(lst, self.href()).map_err(|e| e.to_pyerr(py))?
        };

        let res = server::create_uds_server(
            py, &self, lst, ssl, protocol_factory)?;

        PyFuture::done_fut(py, self.into(), res)
    }

    ///
    /// Connect to a UDS client.
    ///
    fn create_unix_connection(&self, py: Python, protocol_factory: PyObject,
                              path: Option<&str>,
                              ssl: Option<PyObject>,
                              sock: Option<&PyObjectRef>,
                              server_hostname: Option<PyObject>) -> PyResult<Py<PyFuture>> {
        match (&server_hostname, &ssl) {
            (&Some(_), &None) =>
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "server_hostname is only meaningful with ssl")),
            (&None, &Some(_)) => {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "you have to pass server_hostname when using ssl"));
            }
            _ => (),
        }

        let stream = if let Some(path) = path {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "path and sock can not be specified at the same time"))
            }

            UnixStream::connect(Path::new(path), self.href()).map_err(|e| e.to_pyerr(py))?
        } else {
            let sock = if let Some(sock) = sock {
                if ! self.is_uds_socket(sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A UNIX Domain Stream Socket was expected, got {:?}", sock)))
                }
                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "no path and sock were specified"))
            };

            let fileno = self.get_socket_fd(sock)?;

            // create UnixStream object
            let stream = unsafe {
                unix::net::UnixStream::from_raw_fd(fileno as RawFd)
            };

            UnixStream::from_stream(stream, self.href()).map_err(|e| e.to_pyerr(py))?
        };

        // result future
        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_conn = fut.clone_ref(py);

        // create transport
        let waiter = PyFuture::new(py, self.into())?;
        let result = transport::tcp_transport_factory(
            self.into(), false, &protocol_factory, &ssl, server_hostname,
            stream, None, None, Some(waiter.clone_ref(py)))
            .map_err(|e| e.to_pyerr(py))?;
        let waiter: PyFut = waiter.into();

        // wait waiter completion
        self.handle.spawn(
            waiter
            // set exception to future
                .map_err(move |e| fut_err.with_mut(
                    |py, fut| {
                        let _ = fut.set(
                            py, Err(PyErr::new_err(
                                py, Classes.CancelledError.as_ref(py), NoArgs)));}))
            // set transport and protocol
                .map(move |res| {
                    let _ = fut_conn.with_mut(
                        |py, fut| fut.set(py, Ok(result.into_tuple(py).into())));})
        );

        Ok(fut)
    }

    /// Handle an accepted connection.
    ///
    /// This is used by servers that accept connections outside of
    /// asyncio but that use asyncio to handle connections.
    ///
    /// This method is a coroutine.  When completed, the coroutine
    /// returns a (transport, protocol) pair.
    fn connect_accepted_socket(&self, py: Python,
                               protocol_factory: PyObject,
                               sock: &PyObjectRef,
                               ssl: Option<PyObject>) -> PyResult<Py<PyFuture>> {
        if ! self.is_stream_socket(sock)? {
            return Err(PyErr::new::<exc::ValueError, _>(
                py, format!("A Stream Socket was expected, got {:?}", sock)))
        }

        let fileno = self.clone_socket_fd(sock)?;
        let addr = self.addr_from_socket(sock)?;

        // create TcpStream object
        let stream = unsafe {
            net::TcpStream::from_raw_fd(fileno as RawFd)
        };

        // tokio stream
        let stream = match TcpStream::from_stream(stream, self.href()) {
            Ok(stream) => stream,
            Err(err) => return Err(err.to_pyerr(py)),
        };

        let waiter = PyFuture::new(py, self.into())?;
        let peer = stream.peer_addr().expect("should never happen");

        let result = transport::tcp_transport_factory(
            self.into(), true, &protocol_factory, &ssl,
            None, stream, Some(&addr), Some(peer), Some(waiter.clone_ref(py)));

        // client future
        let fut = PyFuture::new(py, self.into())?;
        let fut_err = fut.clone_ref(py);
        let fut_conn = fut.clone_ref(py);

        // wait until transport get ready
        let waiter: PyFut = waiter.into();
        self.handle.spawn(
            waiter.then(move |_| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                match result {
                    Ok(transport) => {
                        let _ = fut_conn.as_mut(py).set(
                            py, Ok(transport.into_tuple(py).into()));
                    },
                    Err(err) => {
                        let _ = fut_err.as_mut(py).set(
                            py, Err(err.to_pyerr(py)));
                    },
                }
                Ok(())
            })
        );
        Ok(fut)
    }

    /// Return an exception handler, or None if the default one is in use.
    fn get_exception_handler(&self, py: Python) -> PyResult<PyObject> {
        Ok(self.exception_handler.clone_ref(py))
    }

    /// Set handler as the new event loop exception handler.
    ///
    /// If handler is None, the default exception handler will be set.
    ///
    /// If handler is a callable object, it should have a
    /// signature matching '(loop, context)', where 'loop'
    /// will be a reference to the active event loop, 'context'
    /// will be a dict object (see `call_exception_handler()`
    /// documentation for details about context).
    fn set_exception_handler(&mut self, py: Python, handler: &PyObjectRef) -> PyResult<()> {
        if !handler.is_none() && !handler.is_callable() {
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("A callable object or None is expected, got {:?}", handler)))
        } else {
            self.exception_handler = handler.into();
            Ok(())
        }
    }

    /// Call the current event loop's exception handler.
    ///
    /// The context argument is a dict containing the following keys:
    ///
    /// - 'message': Error message;
    /// - 'exception' (optional): Exception object;
    /// - 'future' (optional): Future instance;
    /// - 'handle' (optional): Handle instance;
    /// - 'protocol' (optional): Protocol instance;
    /// - 'transport' (optional): Transport instance;
    /// - 'socket' (optional): Socket instance;
    /// - 'asyncgen' (optional): Asynchronous generator that caused
    ///                          the exception.
    ///
    /// New keys maybe introduced in the future.
    ///
    /// Note: do not overload this method in an event loop subclass.
    /// For custom exception handling, use the `set_exception_handler()` method.
    pub fn call_exception_handler(&self, py: Python, context: &PyDict) -> PyResult<()> {
        if self.exception_handler.is_none() {
            let mut log = String::new();
            let  _ = match context.get_item("message") {
                Some(s) => writeln!(log, "{}", s),
                None => writeln!(log, "Unhandled exception in event loop")
            };
            if let Some(err) = context.get_item("exception") {
                utils::print_exception(py, &mut log, PyErr::from_instance(py, err));
            }
            error!("{}", log);
        } else {
            let res = self.exception_handler.call(
                py, (self, context.to_object(py)), None);
            if let Err(err) = res {
                // Exception in the user set custom exception handler.
                error!(
                    "Unhandled error in exception handler, exception: {:?}, context: {}",
                    err, &context);
            }
        }
        py.release(context);
        Ok(())
    }

    ///
    /// Run until stop() is called
    ///
    fn run_forever(&mut self, py: Python) -> PyResult<PyObject> {
        if let Some(_) = self.runner {
            return Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Event loop is running already"));
        }

        let evloop: Py<TokioEventLoop> = self.into();

        let result = py.allow_threads(|| {
            let ev: &mut TokioEventLoop = evloop.as_mut(GIL::python());
            if let Some(ref mut core) = evloop.as_mut(GIL::python()).core {
                let rx = {
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // set cancel sender
                    let (tx, rx) = oneshot::channel();
                    evloop.as_mut(py).runner = Some(tx);
                    rx
                };

                // SIGINT
                let ctrlc_f = tokio_signal::ctrl_c(ev.href());
                let ctrlc = core.0.run(ctrlc_f).unwrap().into_future();

                let fut = rx.select2(ctrlc).then(|res| {
                    match res {
                        Ok(future::Either::A((res, _))) => match res {
                            Ok(_) => future::ok(RunStatus::Stopped),
                            Err(err) => future::ok(RunStatus::PyRes(Err(err))),
                        },
                        Ok(future::Either::B(_)) => future::ok(RunStatus::CtrlC),
                        Err(_) => future::err(()),
                    }
                });

                // set ID for current thread
                let old = ID.with(|cell| cell.borrow().get());
                ID.with(|mut cell| cell.borrow_mut().set(ev.id));

                let result = match core.0.run(fut) {
                    Ok(status) => status,
                    Err(_) => RunStatus::Error,
                };
                if let Some(id) = old {
                    ID.with(|cell| cell.set(Some(id)));
                }

                Ok(result)
            } else {
                let gil = Python::acquire_gil();
                let py = gil.python();
                return Err(PyErr::new::<exc::RuntimeError, _>(py, "Event loop is closed"));
            }
        })?;
        py.release(evloop);

        let _ = self.stop();
        match result {
            RunStatus::Stopped => Ok(py.None()),
            RunStatus::CtrlC => Ok(py.None()),
            RunStatus::PyRes(res) => res,
            RunStatus::Error => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Unknown runtime error")),
        }
    }

    /// Run until the Future is done.
    ///
    /// If the argument is a coroutine, it is wrapped in a Task.
    ///
    /// WARNING: It would be disastrous to call run_until_complete()
    /// with the same coroutine twice -- it would wrap it in two
    /// different Tasks and that can't be good.
    ///
    /// Return the Future's result, or raise its exception.
    fn run_until_complete(&self, py: Python, fut: &PyObjectRef) -> PyResult<PyObject> {
        if let Some(_) = self.runner {
            return Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Event loop is running already"))
        }

        let ptr = self.into();

        // PyTask
        if let Some(fut) = PyTask::try_exact_downcast_from(fut) {
            if !fut.is_same_loop(&self) {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }
            let fut: PyTaskFut = fut.into();
            py.allow_threads(|| TokioEventLoop::run_future(ptr, Box::new(fut)))

        // PyFuture
        } else if let Some(fut) = PyFuture::try_exact_downcast_from(fut) {
            if !fut.is_same_loop(&self) {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }
            let fut: PyFut = fut.into();
            py.allow_threads(|| TokioEventLoop::run_future(ptr, Box::new(fut)))

        // asyncio.Future
        } else if fut.hasattr("_asyncio_future_blocking")? {
            let l = fut.getattr("_loop")?;
            if l.as_ptr() != self.as_ptr() {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }
            let fut: PyFut = PyFuture::from_fut(py, self.into(), fut)?.into();
            py.allow_threads(|| TokioEventLoop::run_future(ptr, Box::new(fut)))
        } else {
            if utils::iscoroutine(fut) {
                let fut: PyTaskFut = PyTask::new(py, fut.into(), &self)?.into();
                py.allow_threads(|| TokioEventLoop::run_future(ptr, Box::new(fut)))
            } else {
                return Err(PyErr::new::<exc::TypeError, _>(
                    py, "Future or Generator object is required"))
            }
        }
    }

    ///
    /// Event loop debug flag
    ///
    pub fn get_debug(&self) -> PyResult<bool> {
        Ok(self.debug)
    }

    fn set_debug(&mut self, enabled: bool) -> PyResult<()> {
        self.debug = enabled;
        Ok(())
    }

    ///
    /// slow_callback_duration
    ///
    #[getter]
    fn get_slow_callback_duration(&self) -> PyResult<f32> {
        Ok(self.slow_callback_duration as f32 / 1000.0)
    }
    #[setter]
    fn set_slow_callback_duration(&mut self, value: &PyObjectRef) -> PyResult<()> {
        let millis = utils::parse_millis(self.py(), "slow_callback_duration", value)?;
        self.slow_callback_duration = millis;
        Ok(())
    }
}


impl TokioEventLoop {

    /// Check if ``debug`` mode is enabled
    pub fn is_debug(&self) -> bool {
        self.debug
    }

    /// Get reference to tokio remote handle
    pub fn remote(&self) -> &Remote {
        &self.remote
    }

    /// Get reference to tokio handle
    pub fn href(&self) -> &Handle {
        &self.handle
    }

    /// Clone tokio handle
    pub fn get_handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Stop with py exception
    pub fn stop_with_err(&mut self, err: PyErr) {
        let runner = self.runner.take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(Err(err));
            },
            None => (),
        }
    }

    /// set current executing task (for asyncio.Task.current_task api)
    pub fn set_current_task(&mut self, task: PyObject) {
        self.current_task = Some(task)
    }

    pub fn schedule_callback(&self, cb: callbacks::Callback)  {
        unsafe {(&mut *self.callbacks).call_soon(cb)}
    }

    /// Linux's socket.type is a bitmask that can include extra info
    /// about socket, therefore we can't do simple
    /// `sock_type == socket.SOCK_STREAM`.
    fn is_stream_socket(&self, sock: &PyObjectRef) -> PyResult<bool> {
        let stream = addrinfo::SocketType::Stream.to_int() as i32;
        let socktype: i32 = sock.getattr("type")?.extract()?;
        Ok((socktype & stream) == stream)
    }

    fn is_uds_socket(&self, sock: &PyObjectRef) -> PyResult<bool> {
        if self.is_stream_socket(sock)? {
            let unix = addrinfo::Family::Unix.to_int() as i32;
            let family: i32 = sock.getattr("family")?.extract()?;
            Ok((family & unix) == unix)
        } else {
            Ok(false)
        }
    }

    /// Linux's socket.type is a bitmask that can include extra info
    /// about socket, therefore we can't do simple
    /// `sock_type == socket.SOCK_DGRAM`.
    fn _is_dgram_socket(&self, sock: &PyObjectRef) -> PyResult<bool> {
        let dgram = addrinfo::SocketType::DGram.to_int() as i32;
        let socktype: i32 = sock.getattr("type")?.extract()?;
        Ok((socktype & dgram) == dgram)
    }

    /// opened sockets only
    fn get_socket_fd(&self, sock: &PyObjectRef) -> PyResult<c_int> {
        let fileno: c_int = sock.call_method("fileno", NoArgs, None)?.extract()?;
        if fileno == -1 {
            Err(PyErr::new::<exc::OSError, _>(self.py(), "Bad file"))
        } else {
            Ok(fileno)
        }
    }

    /// clone socket fd
    fn clone_socket_fd(&self, sock: &PyObjectRef) -> PyResult<c_int> {
        let fd = self.get_socket_fd(sock)?;
        let fd = unsafe {
            libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0)
        };
        if fd == -1 {
            Err(PyErr::new::<exc::OSError, _>(self.py(), "Bad file"))
        } else {
            Ok(fd)
        }
    }

    /// check if socket is blocked
    fn is_socket_nonblocking(&self, sock: &PyObjectRef) -> PyResult<()> {
        if self.debug {
            let timeout = sock.call_method("gettimeout", NoArgs, None)?;
            if let Ok(timeout) = timeout.extract::<c_int>() {
                if timeout == 0 {
                    return Ok(())
                }
            }
            Err(PyErr::new::<exc::ValueError, _>(self.py(), "the socket must be non-blocking"))
        } else {
            return Ok(())
        }
    }

    /// Extract AddrInfo from python native socket object
    fn addr_from_socket(&self, sock: &PyObjectRef) -> PyResult<addrinfo::AddrInfo> {
        let family: i32 = sock.getattr("family")?.extract()?;
        let socktype: i32 = sock.getattr("type")?.extract()?;
        let proto: i32 = sock.getattr("proto")?.extract()?;

        let addr = PyTuple::downcast_from(sock.call_method("getsockname", NoArgs, None)?)?;

        let sockaddr = if addr.len() == 2 {
            // parse INET
            let s = PyString::downcast_from(addr.get_item(0))?;
            let ip = if let Ok(ip) = net::Ipv4Addr::from_str(s.to_string_lossy().as_ref()) {
                ip
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    self.py(), "Can not parse ip address"))
            };
            let port: u16 = addr.get_item(1).extract()?;

            net::SocketAddr::V4(net::SocketAddrV4::new(ip, port))

        } else if addr.len() == 4 {
            // parse INET6
            let s = PyString::downcast_from(addr.get_item(0))?;
            let ip = if let Ok(ip) = net::Ipv6Addr::from_str(s.to_string_lossy().as_ref()) {
                ip
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    self.py(), "Can not parse ip address"))
            };
            let port: u16 = addr.get_item(1).extract()?;
            let flowinfo: u32 = addr.get_item(2).extract()?;
            let scope_id: u32 = addr.get_item(3).extract()?;

            net::SocketAddr::V6(
                net::SocketAddrV6::new(ip, port, flowinfo, scope_id))

        } else {
            return Err(PyErr::new::<exc::ValueError, _>(
                self.py(), "Unknown address type"))
        };

        Ok(addrinfo::AddrInfo::new(
            0, addrinfo::Family::from_int(family as libc::c_int),
            addrinfo::SocketType::from_int(socktype as libc::c_int),
            addrinfo::Protocol::from_int(proto as libc::c_int),
            sockaddr, None))
    }

    pub fn create_server_helper(&self, py: Python, protocol_factory: PyObject,
                                host: Option<String>, port: Option<u16>,
                                family: i32, flags: i32, sock: Option<&PyObjectRef>,
                                backlog: i32, ssl: Option<PyObject>,
                                reuse_address: bool, reuse_port: bool,
                                transport_factory: transport::TransportFactory)
                                -> PyResult<Py<PyFuture>>
    {
        if let (&None, &None) = (&host, &port) {
            if let Some(sock) = sock {
                // only stream sockets
                if ! self.is_stream_socket(sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A Stream Socket was expected, got {:?}", sock)))
                }

                // check if socket is UNIX domain socket
                if self.is_uds_socket(sock)? {
                    return self.create_unix_server(
                        py, protocol_factory, None, Some(sock), backlog, ssl);
                }

                // listen
                sock.call_method("listen", (backlog,), None)?;

                // opened sockets only
                let fileno = self.get_socket_fd(sock)?;
                let sockaddr = self.addr_from_socket(sock)?;

                // create TcpListener object
                let listener = unsafe {
                    net::TcpListener::from_raw_fd(fileno as RawFd)
                };

                let res = server::create_sock_server(
                    py, &self, listener, sockaddr, ssl, protocol_factory, transport_factory);

                // waiter future
                return PyFuture::done_res(py, self.into(), res)
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "Neither host/port nor sock were specified"))
            }
        }

        // we have host or port, sock should be None
        if let Some(_) = sock {
            return Err(PyErr::new::<exc::ValueError, _>(
                py, "host/port and sock can not be specified at the same time"))
        }

        // waiter future
        let fut = PyFuture::new(py, self.into())?;
        let fut_srv = fut.clone_ref(py);
        let evloop: Py<TokioEventLoop> = self.into();

        // resolve addresses and start listening
        let conn = addrinfo::lookup(self.lookup.as_ref().unwrap(),
                                    host, port.map(|p| p.to_string()),
                                    family, flags, addrinfo::SocketType::Stream)
            .map_err(|err| with_py(
                |py| io::Error::new(io::ErrorKind::Other, err.description()).to_pyerr(py)))
            .then(move |result| {
                let gil = Python::acquire_gil();
                let py = gil.python();
                let fut = fut_srv.as_mut(py);

                match result {
                    Err(err) => {
                        let _ = fut.set(py, Err(err));
                    },
                    Ok(Err(err)) => {
                        let _ = fut.set(py, Err(err.to_pyerr(py)));
                    },
                    Ok(Ok(addrs)) => {
                        if addrs.is_empty() {
                            let _ = fut.set(
                                py, Err(PyErr::new::<exc::RuntimeError, _>(
                                    py, "getaddrinfo() returned empty list")));
                        } else {
                            let res = server::create_server(
                                py, evloop.as_ref(py), addrs, backlog, ssl,
                                reuse_address, reuse_port, protocol_factory, transport_factory);
                            let _ = fut.set(py, res);
                        }
                    }
                }
                future::ok(())
            });

        self.handle.spawn(conn);
        Ok(fut)
    }

    pub fn with<T, F>(&self, message: &str, f: F)
        where F: FnOnce() -> PyResult<T> {

        if let Err(err) = f() {
            self.log_exception(message, Some(err), None, None);
        }
    }

    pub fn log_error(&self, err: PyErr, message: &str) -> PyErr {
        self.log_exception(message, Some(err.clone_ref(self.py())), None, None);
        err
    }

    pub fn log_exception(&self, message: &str,
                         exception: Option<PyErr>,
                         source_traceback: Option<PyObject>,
                         kwargs: Option<&[(PyObject, PyObject)]>) {
        let _: PyResult<()> = {
            let context = PyDict::new(self.py());
            let _ = context.set_item("message", "Future exception was never retrieved");
            source_traceback.map(
                |tb| context.set_item("source_traceback", tb));
            exception.map(
                |mut exc| context.set_item("exception", exc.instance(self.py())));

            if let Some(kwargs) = kwargs {
                for &(ref key, ref val) in kwargs {
                    let _ = context.set_item(key.clone_ref(self.py()), val.clone_ref(self.py()));
                }
            }
            let _ = self.call_exception_handler(self.py(), context);

            Ok(())
        };
    }
}

impl TokioEventLoop {

    /// Run future to completion
    pub fn run_future(ptr: Py<TokioEventLoop>,
                      fut: Box<Future<Item=PyResult<PyObject>,
                                      Error=unsync::oneshot::Canceled>>) -> PyResult<PyObject> {
        let ev = ptr.as_mut(GIL::python());

        let res = match ptr.as_mut(GIL::python()).core {
            Some(ref mut core) => {
                let rx = {
                    // stop fut
                    let (tx, rx) = oneshot::channel();
                    ptr.with_mut(|py, ev| ev.runner = Some(tx));

                    rx
                };

                // SIGINT
                let ctrlc_f = tokio_signal::ctrl_c(ev.href());
                let ctrlc = core.0.run(ctrlc_f).unwrap().into_future();

                let sel = rx.select2(ctrlc).then(|res| {
                    match res {
                        Ok(future::Either::A((res, _))) => match res {
                            Ok(_) => future::ok(RunStatus::Stopped),
                            Err(err) => future::ok(RunStatus::PyRes(Err(err))),
                        },
                        Ok(_) => future::ok(RunStatus::Stopped),
                        Err(err) => future::err(err),
                    }
                });

                let old = ID.with(|cell| cell.borrow().get());
                ID.with(|mut cell| cell.borrow_mut().set(ev.id.clone()));

                // wait for completion
                let result = core.0.run(
                    fut.select2(sel).then(|res| {
                        match res {
                            Ok(future::Either::A((res, _))) => {
                                future::ok(RunStatus::PyRes(res))
                            },
                            Ok(future::Either::B((res, _))) => {
                                future::ok(res)
                            },
                            Err(err) => {
                                future::err(err)
                            },
                        }
                    }));

                if let Some(id) = old {
                    ID.with(|cell| cell.set(Some(id)));
                }

                result
            },
            None => {
                let gil = Python::acquire_gil();
                let py = gil.python();
                return Err(PyErr::new::<exc::RuntimeError, _>(py, "Event loop is closed"));
            },
        };

        let gil = Python::acquire_gil();
        let py = gil.python();

        let _ = ptr.as_mut(py).stop();

        match res {
            Ok(RunStatus::PyRes(res)) => res,
            Err(_) => Err(PyErr::new_err(py, Classes.CancelledError.as_ref(py), NoArgs)),
            _ => Ok(py.None())
        }
    }
}


impl PartialEq for TokioEventLoop {
    fn eq(&self, other: &TokioEventLoop) -> bool {
        self.id == other.id
    }
}
