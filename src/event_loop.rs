#![allow(unused_variables)]

use std::io;
use std::thread;
use std::net;
use std::error::Error;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::fmt::Write;
use std::str::FromStr;
use std::path::Path;
use std::os::raw::c_int;
use std::os::unix;
use std::os::unix::io::{RawFd, FromRawFd};

use libc;
use cpython::*;
use futures::{future, sync, unsync, Async, Future, Stream};
use futures::sync::{oneshot};
use tokio_core::reactor::{Core, CoreId, Remote};
use tokio_signal;
use tokio_signal::unix::Signal;
use tokio_core::net::TcpStream;
use tokio_uds::{UnixStream, UnixListener};

use ::{PyFuture, PyTask};
use addrinfo;
use client;
use handle::PyHandle;
use fd;
use fut::{Until, UntilError};
use http;
use signals;
use server;
use utils::{self, with_py, ToPyErr, Classes};
use pyunsafe::{GIL, Handle, OneshotSender};
use transport;


thread_local!(
    pub static ID: RefCell<Option<CoreId>> = RefCell::new(None);
    pub static CORE: RefCell<Option<Core>> = RefCell::new(None);
);

pub fn no_loop_exc(py: Python) -> PyErr {
    let cur = thread::current();
    PyErr::new::<exc::RuntimeError, _>(
        py, format!("There is no current event loop in thread {}.",
                    cur.name().unwrap_or("unknown")))
}

pub fn new_event_loop(py: Python) -> PyResult<TokioEventLoop> {
    CORE.with(|cell| {
        let core = Core::new().unwrap();
        let handle = core.handle();
        let signals = signals::Signals::new(&handle);

        let evloop = TokioEventLoop::create_instance(
            py, core.id(),
            Handle::new(handle),
            core.remote(),
            Instant::now(),
            addrinfo::start_workers(3),
            RefCell::new(None),
            RefCell::new(None),
            RefCell::new(py.None()),
            Cell::new(100),
            Cell::new(false),
            RefCell::new(None),
            signals,
            RefCell::new(HashMap::new()),
            RefCell::new(HashMap::new()),
        );

        ID.with(|cell| { *cell.borrow_mut() = Some(core.id())});
        *cell.borrow_mut() = Some(core);
        evloop
    })
}

pub fn thread_safe_check(py: Python, id: &CoreId) -> Option<PyErr> {
    let check = ID.with(|cell| {
        match *cell.borrow() {
            None => false,
            Some(ref curr_id) => return curr_id == id,
        }
    });

    if !check {
        Some(PyErr::new::<exc::RuntimeError, _>(
            py, "Non-thread-safe operation invoked on an event loop \
                 other than the current one"))
    } else {
        None
    }
}

#[derive(Debug)]
enum RunStatus {
    Stopped,
    CtrlC,
    NoEventLoop,
    Error,
    PyRes(PyResult<PyObject>),
}


py_class!(pub class TokioEventLoop |py| {
    data id: CoreId;
    data handle: Handle;
    data _remote: Remote;
    data _instant: Instant;
    data _lookup: addrinfo::LookupWorkerSender;
    data _runner: RefCell<Option<oneshot::Sender<PyResult<()>>>>;
    data _executor: RefCell<Option<PyObject>>;
    data _exception_handler: RefCell<PyObject>;
    data _slow_callback_duration: Cell<u64>;
    data _debug: Cell<bool>;
    data _current_task: RefCell<Option<PyObject>>;
    data _signals: sync::mpsc::UnboundedSender<signals::SignalsMessage>;
    data _readers: RefCell<HashMap<c_int, OneshotSender<()>>>;
    data _writers: RefCell<HashMap<c_int, OneshotSender<()>>>;

    //
    // Return the currently running task in an event loop or None.
    //
    def current_task(&self) -> PyResult<PyObject> {
        match *self._current_task(py).borrow() {
            Some(ref task) => Ok(task.clone_ref(py).into_object()),
            None => Ok(py.None())
        }
    }

    //
    // Create a Future object attached to the loop.
    //
    def create_future(&self) -> PyResult<PyFuture> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        PyFuture::new(py, &self)
    }

    //
    // Schedule a coroutine object.
    //
    // Return a task object.
    //
    def create_task(&self, coro: PyObject) -> PyResult<PyTask> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        PyTask::new(py, coro, &self)
    }

    //
    // Return the time according to the event loop's clock.
    //
    // This is a float expressed in seconds since event loop creation.
    //
    def time(&self) -> PyResult<f64> {
        let time = self._instant(py).elapsed();
        Ok(time.as_secs() as f64 + (time.subsec_nanos() as f64 / 1_000_000_000.0))
    }

    //
    // Return the time according to the event loop's clock (milliseconds)
    //
    def millis(&self) -> PyResult<u64> {
        let time = self._instant(py).elapsed();
        Ok(time.as_secs() * 1000 + (time.subsec_nanos() as u64 / 1_000_000))
    }

    //
    // def call_soon(self, callback, *args):
    //
    // Arrange for a callback to be called as soon as possible.
    //
    // This operates as a FIFO queue: callbacks are called in the
    // order in which they are registered.  Each callback will be
    // called exactly once.
    //
    // Any positional arguments after the callback will be passed to
    // the callback when it is called.
    //
    def call_soon(&self, *args, **kwargs) -> PyResult<PyObject> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        if args.len(py) < 1 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 1 arguments"))
        } else {
            // get params
            let callback = args.get_item(py, 0);

            let h = PyHandle::new(py, &self,
                                  callback, PyTuple::new(py, &args.as_slice(py)[1..]))?;
            h.call_soon(py);
            Ok(h.into_object())
        }
    }

    //
    // def call_soon_threadsafe(self, callback, *args):
    //
    // Like call_soon(), but thread-safe.
    //
    def call_soon_threadsafe(&self, *args, **kwargs) -> PyResult<PyObject> {
        if args.len(py) < 1 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 1 arguments"))
        } else {
            // get params
            let callback = args.get_item(py, 0);

            // create handle and schedule work
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[1..]))?;

            h.call_soon_threadsafe(py);

            Ok(h.into_object())
        }
    }

    //
    // def call_later(self, delay, callback, *args)
    //
    // Arrange for a callback to be called at a given time.
    //
    // Return a Handle: an opaque object with a cancel() method that
    // can be used to cancel the call.
    //
    // The delay can be an int or float, expressed in seconds.  It is
    // always relative to the current time.
    //
    // Each callback will be called exactly once.  If two callbacks
    // are scheduled for exactly the same time, it undefined which
    // will be called first.

    // Any positional arguments after the callback will be passed to
    // the callback when it is called.
    //
    def call_later(&self, *args, **kwargs) -> PyResult<PyObject> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        if args.len(py) < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let callback = args.get_item(py, 1);
            let delay = utils::parse_millis(py, "delay", args.get_item(py, 0))?;

            // create handle and schedule work
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[2..]))?;

            if delay == 0 {
                h.call_soon(py);
            } else {
                h.call_later(py, Duration::from_millis(delay));
            };
            Ok(h.into_object())
        }
    }

    //
    // def call_at(self, when, callback, *args):
    //
    // Like call_later(), but uses an absolute time.
    //
    // Absolute time corresponds to the event loop's time() method.
    //
    def call_at(&self, *args, **kwargs) -> PyResult<PyObject> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        if args.len(py) < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let callback = args.get_item(py, 1);

            // create handle and schedule work
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[2..]))?;

            // calculate delay
            if let Some(when) = utils::parse_seconds(py, "when", args.get_item(py, 0))? {
                let time = when - self._instant(py).elapsed();

                h.call_later(py, time);
            } else {
                h.call_soon(py);
            }
            Ok(h.into_object())
        }
    }

    //
    // def add_signal_handler(self, sig, callback, *args)
    //
    // Add a handler for a signal.  UNIX only.
    //
    // Raise ValueError if the signal number is invalid or uncatchable.
    // Raise RuntimeError if there is a problem setting up the handler.
    def add_signal_handler(&self, *args, **kwargs) -> PyResult<()> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        if args.len(py) < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let sig = args.get_item(py, 0).extract::<c_int>(py)?;
            let callback = args.get_item(py, 1);

            // coroutines are not allowed as handlers
            let iscoro: bool = Classes.Coroutines.call(
                py, "iscoroutine", (&callback,), None)?.extract(py)?;
            let iscorof: bool = Classes.Coroutines.call(
                py, "iscoroutinefunction", (&callback,), None)?.extract(py)?;
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
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[2..]))?;

            // register signal handler
            let _ = self._signals(py).send(signals::SignalsMessage::Add(sig, signal, h));

            Ok(())
        }
    }

    //
    // Remove a handler for a signal.  UNIX only.
    //
    // Return True if a signal handler was removed, False if not.
    def remove_signal_handler(&self, sig: c_int) -> PyResult<bool> {
        // un-register signal handler
        let _ = self._signals(py).send(signals::SignalsMessage::Remove(sig));

        Ok(true)
    }

    def _add_reader(&self, *args, **kwargs) -> PyResult<()> {
        if args.len(py) < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let fd: c_int = args.get_item(py, 0).extract(py)?;
            let callback = args.get_item(py, 1);

            // create handle
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[2..]))?;
            match fd::PyFdHandle::reader(fd, self.href(), h) {
                Ok(tx) => {
                    self._readers(py).borrow_mut().insert(fd, OneshotSender::new(tx));
                    Ok(())
                },
                Err(err) => Err(err.to_pyerr(py)),
            }
        }
    }

    def _remove_reader(&self, fd: c_int) -> PyResult<bool> {
        if let Some(tx) = self._readers(py).borrow_mut().remove(&fd) {
            let _ = tx.send(());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    def _add_writer(&self, *args, **kwargs) -> PyResult<()> {
        if args.len(py) < 2 {
            Err(PyErr::new::<exc::TypeError, _>(py, "function takes at least 2 arguments"))
        } else {
            // get params
            let fd: c_int = args.get_item(py, 0).extract(py)?;
            let callback = args.get_item(py, 1);

            // create handle
            let h = PyHandle::new(
                py, &self, callback, PyTuple::new(py, &args.as_slice(py)[2..]))?;
            match fd::PyFdHandle::writer(fd, self.href(), h) {
                Ok(tx) => {
                    self._writers(py).borrow_mut().insert(fd, OneshotSender::new(tx));
                    Ok(())
                },
                Err(err) => Err(err.to_pyerr(py)),
            }
        }
    }

    def _remove_writer(&self, fd: c_int) -> PyResult<bool> {
        if let Some(tx) = self._writers(py).borrow_mut().remove(&fd) {
            let _ = tx.send(());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // Add a reader callback
    def add_reader(&self, *args, **kwargs) -> PyResult<()> {
        return self._add_reader(py, args, kwargs)
    }

    // Remove a reader callback
    def remove_reader(&self, fd:c_int) -> PyResult<bool> {
        self._remove_reader(py, fd)
    }

    // Add a writer callback
    def add_writer(&self, *args, **kwargs) -> PyResult<()> {
        return self._add_writer(py, args, kwargs)
    }

    // Remove a writer callback
    def remove_writer(&self, fd: c_int) -> PyResult<bool> {
        return self._remove_writer(py, fd)
    }

    // Receive data from the socket.
    //
    // The return value is a bytes object representing the data received.
    // The maximum amount of data to be received at once is specified by
    // nbytes.
    //
    // This method is a coroutine.
    def sock_recv(&self, sock: PyObject, n: PyObject) -> PyResult<PyFuture> {
        let _ = self.is_socket_nonblocking(py, &sock)?;

        // create readiness stream
        let fd = {
            let fd = self.get_socket_fd(py, &sock)?;
            match fd::PyFdReadable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, &self, Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, &self)?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            // fut cancelled
            if fut_ready.is_cancelled(py) {
                return future::ok(Some(()));
            }

            let res = sock.call_method(py, "recv", (n.clone_ref(py),), None);

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
                    let _ = fut_ready.set(py, Ok(result));
                    future::ok(Some(()))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    with_py(|py| {
                        let _ = fut_err.set(py, Err(err));
                    });
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);

        Ok(fut)
    }

    // Send data to the socket.
    //
    // The socket must be connected to a remote socket. This method continues
    // to send data from data until either all data has been sent or an
    // error occurs. None is returned on success. On error, an exception is
    // raised, and there is no way to determine how much data, if any, was
    // successfully processed by the receiving end of the connection.
    //
    // This method is a coroutine.
    def sock_sendall(&self, sock: PyObject, data: PyObject) -> PyResult<PyFuture> {
        let _ = self.is_socket_nonblocking(py, &sock)?;

        // data is empty, nothing to do
        if ! data.is_true(py).unwrap() {
            return Ok(PyFuture::done_res(py, &self, Ok(py.None()))?)
        }

        // create readyness stream for write operation
        let fd = {
            let fd = self.get_socket_fd(py, &sock)?;
            match fd::PyFdWritable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, &self, Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, &self)?;
        let fut_ready = fut.clone_ref(py);
        let fut_err = fut.clone_ref(py);
        let wrp = PyList::new(py, &[data]);

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            if fut_ready.is_cancelled(py) {
                return future::ok(Some(()));
            }

            let data = wrp.get_item(py, 0);
            let res = sock.call_method(py, "send", (data.clone_ref(py),), None);

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
                    if let Ok(n) = result.extract::<c_int>(py) {
                        let len = data.len(py).unwrap() as c_int;
                        if n == len {
                            // all data is sent
                            let _ = fut_ready.set(py, Ok(py.None()));
                            future::ok(Some(()))
                        } else {
                            // some data got send
                            let slice = PySlice::new(py, n as isize, len as isize, 1);
                            match data.call_method(py, "__getitem__", (slice,), None) {
                                Ok(data) => {
                                    wrp.set_item(py, 0, data);
                                    future::ok(None)
                                },
                                Err(err) =>
                                    future::err(err)
                            }
                        }
                    } else {
                        // exception
                        future::err(PyErr::new::<exc::OSError, _>(
                            py, format!("sendall call failed {}", sock)))
                    }
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    with_py(|py| {
                        let _ = fut_err.set(py, Err(err));
                    });
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    // Connect to a remote socket at address.
    //
    // This method is a coroutine.
    def sock_connect(&self, sock: PyObject, address: PyObject) -> PyResult<PyFuture> {
        let _ = self.is_socket_nonblocking(py, &sock)?;

        //if not hasattr(socket, 'AF_UNIX') or sock.family != socket.AF_UNIX:
        //resolved = base_events._ensure_resolved(
        //    address, family=sock.family, proto=sock.proto, loop=self)
        //    if not resolved.done():
        //yield from resolved
        //    _, _, _, _, address = resolved.result()[0]

        // try to connect
        let res = sock.call_method(py, "connect", (address.clone_ref(py),), None);

        // if connect is blocking, create readiness stream
        let fd = match res {
            Ok(_) => {
                return Ok(PyFuture::done_fut(py, &self, py.None())?);
            },
            Err(err) => {
                if ! err.matches(py, (py.get_type::<exc::BlockingIOError>(),
                                      py.get_type::<exc::InterruptedError>())) {
                    return Ok(PyFuture::done_res(py, &self, Err(err))?);
                }
                let fd = self.get_socket_fd(py, &sock)?;
                match fd::PyFdWritable::new(fd, self.href()) {
                    Err(err) => return Ok(
                        PyFuture::done_res(py, &self, Err(err.to_pyerr(py)))?),
                    Ok(fd) => fd
                }
            }
        };


        // wait until sock get connected
        let fut = PyFuture::new(py, &self)?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            if fut_ready.is_cancelled(py) {
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
                    if let Ok(err) = result.extract::<i32>(py) {
                        if err == 0 {
                            let _ = fut_ready.set(py, Ok(py.None()));
                            return future::ok(Some(()))
                        }
                    }

                    // Jump to any except clause below.
                    future::err(PyErr::new::<exc::OSError, _>(
                        py, (result, format!("Connect call failed {}", address))))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    with_py(|py| {
                        let _ = fut_err.set(py, Err(err));
                    });
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    // Accept a connection.
    //
    // The socket must be bound to an address and listening for connections.
    // The return value is a pair (conn, address) where conn is a new socket
    // object usable to send and receive data on the connection, and address
    // is the address bound to the socket on the other end of the connection.
    //
    // This method is a coroutine.
    def sock_accept(&self, sock: PyObject) -> PyResult<PyFuture> {
        let _ = self.is_socket_nonblocking(py, &sock)?;

        // create readiness stream
        let fd = {
            let fd = self.get_socket_fd(py, &sock)?;
            match fd::PyFdReadable::new(fd, self.href()) {
                Ok(fd) => fd,
                Err(err) => return Ok(
                    PyFuture::done_res(py, &self, Err(err.to_pyerr(py)))?),
            }
        };

        // wait until sock get ready
        let fut = PyFuture::new(py, &self)?;
        let fut_err = fut.clone_ref(py);
        let fut_ready = fut.clone_ref(py);

        let f = fd.until(move |_| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            // fut cancelled
            if fut_ready.is_cancelled(py) {
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
                    if let Ok(result) = PyTuple::downcast_from(py, result.clone_ref(py)) {
                        let _ = result.get_item(py, 0).call_method(
                            py, "setblocking", (false,), None);
                    }
                    
                    let _ = fut_ready.set(py, Ok(result));
                    future::ok(Some(()))
                }
            }
        }).map_err(move |err| {
            match err {
                UntilError::Error(err) => {
                    // actual python exception
                    with_py(|py| {
                        let _ = fut_err.set(py, Err(err));
                    });
                },
                _ => unreachable!(),
            };
        });

        self.href().spawn(f);
        Ok(fut)
    }

    //
    // Stop running the event loop.
    //
    def stop(&self) -> PyResult<PyBool> {
        let runner = self._runner(py).borrow_mut().take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(Ok(()));
                Ok(py.True())
            },
            None => Ok(py.False()),
        }
    }

    def is_running(&self) -> PyResult<bool> {
        Ok(match *self._runner(py).borrow() {
            Some(_) => true,
            None => false,
        })
    }

    def is_closed(&self) -> PyResult<bool> {
        ID.with(|cell| {
            match *cell.borrow() {
                None => Ok(true),
                Some(_) => Ok(false),
            }
        })
    }

    //
    // Close the event loop. The event loop must not be running.
    //
    def close(&self) -> PyResult<PyObject> {
        if let Ok(running) = self.is_running(py) {
            if running {
                return Err(
                    PyErr::new::<exc::RuntimeError, _>(
                        py, "Cannot close a running event loop"));
            }
        }

        // shutdown executor
        if let Some(executor) = self._executor(py).borrow_mut().take() {
            let kwargs = PyDict::new(py);
            kwargs.set_item(py, "wait", false)?;
            let _ = executor.call_method(py, "shutdown", NoArgs, Some(&kwargs));
        }

        // drop CORE
        ID.with(|cell| {cell.borrow_mut().take()});
        CORE.with(|cell| {cell.borrow_mut().take()});

        Ok(py.None())
    }

    //
    // Executor api
    //
    def run_in_executor(&self, *args, **kwargs) -> PyResult<PyObject> {
        if self._debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }
        // get params
        if args.len(py) < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }
        let mut executor = args.get_item(py, 0);
        let args = PyTuple::new(py, &args.as_slice(py)[1..]);

        // get or create default executor
        if executor == py.None() {
            let exec = self._executor(py).borrow().as_ref().map(|ex| ex.clone_ref(py));
            executor = if let Some(ex) = exec {
                ex
            } else {
                let concurrent = py.import("concurrent.futures")?;
                let executor = concurrent.call(py, "ThreadPoolExecutor", NoArgs, None)?;
                *self._executor(py).borrow_mut() = Some(executor.clone_ref(py));
                executor
            };
        }

        // submit function
        let fut = executor.call_method(py, "submit", args, None)?;

        // wrap_future
        let kwargs = PyDict::new(py);
        let _ = kwargs.set_item(py, "loop", self.clone_ref(py))?;
        Classes.Asyncio.call(py, "wrap_future", (fut,), Some(&kwargs))
    }

    def set_default_executor(&self, executor: PyObject) -> PyResult<PyObject> {
        *self._executor(py).borrow_mut() = Some(executor);
        Ok(py.None())
    }

    // return list of tuples
    // item = (family, type, proto, canonname, sockaddr)
    // sockaddr(IPV4) = (address, port)
    // sockaddr(IPV6) = (address, port, flow info, scope id)
    def getaddrinfo(&self, *args, **kwargs) -> PyResult<PyFuture> {
        // parse params
        let len = args.len(py);
        if len < 1 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "host is required"))
        }
        if len < 2 {
            return Err(PyErr::new::<exc::ValueError, _>(py, "port is required"))
        }

        // parse host (string, unicode, bytes, None)
        let host_arg = args.get_item(py, 0);
        let host = if host_arg == py.None() {
            None
        } else if let Ok(host) = PyString::downcast_from(py, host_arg.clone_ref(py)) {
            Some(String::from(host.to_string_lossy(py)))
        } else {
            if let Ok(host) = PyString::from_object(py, &host_arg, "utf-8", "strict") {
                Some(String::from(host.to_string_lossy(py)))
            } else {
                return Err(PyErr::new::<exc::TypeError, _>(
                    py, "string or none type is required as host"))
            }
        };

        // parse port (int, string, unicode or none)
        let port_arg = args.get_item(py, 1);
        let port = if port_arg == py.None() {
            None
        } else if let Ok(port) = PyString::downcast_from(py, port_arg.clone_ref(py)) {
            Some(String::from(port.to_string_lossy(py)))
        } else if let Ok(port) = port_arg.extract::<u16>(py) {
            Some(port.to_string())
        } else {
            Some(String::from(
                PyString::from_object(
                    py, &port_arg, "utf-8", "strict")?.to_string_lossy(py)))
        };

        let mut family: i32 = 0;
        let mut socktype: i32 = 0;
        let mut _proto: i32 = 0;
        let mut flags: i32 = 0;

        if let Some(kwargs) = kwargs {
            if let Some(f) = kwargs.get_item(py, "family") {
                family = f.extract(py)?
            }
            if let Some(s) = kwargs.get_item(py, "type") {
                socktype = s.extract(py)?
            }
            if let Some(p) = kwargs.get_item(py, "proto") {
                _proto = p.extract(py)?
            }
            if let Some(f) = kwargs.get_item(py, "flags") {
                flags = f.extract(py)?
            }
        }

        // result future
        let res = PyFuture::new(py, &self)?;

        // create processing future
        let fut = res.clone_ref(py);
        let fut_err = res.clone_ref(py);

        // lookup process future
        let lookup = addrinfo::lookup(
            &self._lookup(py), host,
            port, family, flags, addrinfo::SocketType::from_int(socktype));

        // convert addr info to python comaptible  values
        let process = lookup.and_then(move |result| {
            with_py(|py| match result {
                Err(ref err) => {
                    let _ = fut.set(py, Err(err.to_pyerr(py)));
                },
                Ok(ref addrs) => {
                    // create socket.gethostname compatible result
                    let list = PyList::new(py, &[]);
                    for info in addrs {
                        let addr = match info.sockaddr {
                            net::SocketAddr::V4(addr) => {
                                (format!("{}", addr.ip()), addr.port()).to_py_tuple(py)
                            }
                            net::SocketAddr::V6(addr) => {
                                (format!("{}", addr.ip()),
                                 addr.port(), addr.flowinfo(), addr.scope_id(),).to_py_tuple(py)
                            },
                        };

                        let cname = match info.canonname {
                            Some(ref cname) => PyString::new(py, cname.as_str()),
                            None => PyString::new(py, ""),
                        };

                        let item = (info.family.to_int(),
                                    info.socktype.to_int(),
                                    info.protocol.to_int(),
                                    cname, addr).to_py_tuple(py).into_object();
                        list.insert_item(py, list.len(py), item);
                    }
                    let _ = fut.set(py, Ok(list.into_object()));
                },
            });
            future::ok(())
        }).map_err(move |err| {
            with_py(|py| {
                let err = PyErr::new::<exc::RuntimeError, _>(py, "Unknown runtime error");
                fut_err.set(py, Err(err))
            });
        });

        // start task
        self.handle(py).spawn(process);

        Ok(res)
    }

    // TODO need rust version, use python code for now
    def getnameinfo(&self, sockaddr: PyObject, flags: i32 = 0) -> PyResult<PyObject> {
        self.run_in_executor(
            py, &(py.None(), Classes.GetNameInfo.clone_ref(py),
                  sockaddr, flags).to_py_tuple(py), None)
    }

    def connect_read_pipe(&self, protocol_factory: PyObject, pipe: PyObject)
                          -> PyResult<PyFuture> {
        let protocol = protocol_factory.call(py, NoArgs, None)?;
        let waiter = PyFuture::new(py, &self)?;

        // create unix transport
        let cls = Classes.UnixEvents.get(py, "_UnixReadPipeTransport")?;
        let transport = cls.call(
            py, (&self, pipe, protocol.clone_ref(py),
                 waiter.clone_ref(py), py.None()), None)?;

        // wait for transport get ready
        let fut = PyFuture::new(py, &self)?;
        let fut_ready = fut.clone_ref(py);

        self.href().spawn(
            waiter.then(move |res| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                match res {
                    Ok(res) => match res {
                        Ok(res) => {
                            let _ = fut_ready.set(
                                py, Ok((transport, protocol).to_py_tuple(py).into_object()));
                        },
                        Err(err) => {
                            let _ = transport.call_method(py, "close", NoArgs, None);
                            let _ = fut_ready.set(py, Err(err));
                        }
                    },
                    Err(_) => {
                        let _ = fut_ready.cancel(py);
                    }
                }
                Ok(())
            }));

        Ok(fut)
    }

    def connect_write_pipe(&self, protocol_factory: PyObject, pipe: PyObject)
                           -> PyResult<PyFuture> {
        let protocol = protocol_factory.call(py, NoArgs, None)?;
        let waiter = PyFuture::new(py, &self)?;

        // create unix transport
        let cls = Classes.UnixEvents.get(py, "_UnixWritePipeTransport")?;
        let transport = cls.call(
            py, (&self, pipe, protocol.clone_ref(py),
                 waiter.clone_ref(py), py.None()), None)?;

        // wait for transport get ready
        let fut = PyFuture::new(py, &self)?;
        let fut_ready = fut.clone_ref(py);

        self.href().spawn(
            waiter.then(move |res| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                match res {
                    Ok(res) => match res {
                        Ok(res) => {
                            let _ = fut_ready.set(
                                py, Ok((transport, protocol).to_py_tuple(py).into_object()));
                        },
                        Err(err) => {
                            let _ = transport.call_method(py, "close", NoArgs, None);
                            let _ = fut_ready.set(py, Err(err));
                        }
                    },
                    Err(_) => {
                        let _ = fut_ready.cancel(py);
                    }
                }
                Ok(())
            }));

        Ok(fut)
    }

    //
    // subprocess_shell
    //
    def _socketpair(&self) -> PyResult<PyObject> {
        Classes.Socket.call(py, "socketpair", NoArgs, None)
    }

    def _child_watcher_callback(&self, pid: PyObject, returncode: PyObject, transp: PyObject)
                                -> PyResult<PyObject> {
        let process_exited = transp.getattr(py, "_process_exited")?;
        self.call_soon_threadsafe(
            py, &(process_exited, returncode).to_py_tuple(py), None)
    }

    def subprocess_shell(&self, *args, **kwargs) -> PyResult<PyFuture> {
        if args.len(py) < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }

        let protocol_factory = args.get_item(py, 0);
        let cmd = args.get_item(py, 1);
        let empty;
        let kwargs = if let Some(kw) = kwargs {
            kw
        } else {
            empty = PyDict::new(py);
            &empty
        };

        let stdin: i32 = if let Some(val) = kwargs.get_item(py, "stdin") {
            let _ = kwargs.del_item(py, "stdin")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let stdout: i32 = if let Some(val) = kwargs.get_item(py, "stdout") {
            let _ = kwargs.del_item(py, "stdout")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let stderr: i32 = if let Some(val) = kwargs.get_item(py, "stderr") {
            let _ = kwargs.del_item(py, "stderr")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let newlines = if let Some(val) = kwargs.get_item(py, "universal_newlines") {
            let _ = kwargs.del_item(py, "universal_newlines")?;
            if val == py.None() {
                false
            } else {
                val.extract::<bool>(py)?
            }
        } else {
            false
        };
        let shell = if let Some(val) = kwargs.get_item(py, "shell") {
            let _ = kwargs.del_item(py, "shell")?;
            if val == py.None() {
                true
            } else {
                val.extract::<bool>(py)?
            }
        } else {
            true
        };
        let bufsize = if let Some(val) = kwargs.get_item(py, "bufsize") {
            let _ = kwargs.del_item(py, "bufsize")?;
            if val == py.None() {
                0
            } else {
                val.extract::<i32>(py)?
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

        let protocol = protocol_factory.call(py, NoArgs, None)?;

        let ev = Classes.UnixEvents.get(py, "_UnixSelectorEventLoop")?;
        let coro = ev.call_method(
            py, "_make_subprocess_transport",
            (&self, protocol.clone_ref(py), cmd, true,
             stdin, stdout, stderr, bufsize), Some(kwargs))?;

        let fut = PyFuture::new(py, &self)?;
        let fut_ready = fut.clone_ref(py);

        let task = PyTask::new(py, coro, &self)?;

        self.href().spawn(task.then(move |res| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            match res {
                Ok(res) => match res {
                    Ok(transport) => {
                        let result = (transport, protocol).to_py_object(py).into_object();

                        let _ = fut_ready.set(py, Ok(result));
                    },
                    Err(err) => {
                        let _ = fut_ready.set(py, Err(err));
                    }
                },
                Err(_) => {
                    let _ = fut_ready.cancel(py);
                }
            }
            Ok(())
        }));

        Ok(fut)
    }

    //
    // subprocess_exec
    //
    def subprocess_exec(&self, *args, **kwargs) -> PyResult<PyFuture> {
        if args.len(py) < 2 {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, "function takes at least 2 arguments"))
        }

        let protocol_factory = args.get_item(py, 0);
        let empty;
        let kwargs = if let Some(kw) = kwargs {
            kw
        } else {
            empty = PyDict::new(py);
            &empty
        };

        let stdin: i32 = if let Some(val) = kwargs.get_item(py, "stdin") {
            let _ = kwargs.del_item(py, "stdin")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let stdout: i32 = if let Some(val) = kwargs.get_item(py, "stdout") {
            let _ = kwargs.del_item(py, "stdout")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let stderr: i32 = if let Some(val) = kwargs.get_item(py, "stderr") {
            let _ = kwargs.del_item(py, "stderr")?;
            if val == py.None() {
                -1
            } else {
                val.extract(py)?
            }
        } else {
            -1
        };
        let newlines = if let Some(val) = kwargs.get_item(py, "universal_newlines") {
            let _ = kwargs.del_item(py, "universal_newlines")?;
            if val == py.None() {
                false
            } else {
                val.extract::<bool>(py)?
            }
        } else {
            false
        };
        let shell = if let Some(val) = kwargs.get_item(py, "shell") {
            let _ = kwargs.del_item(py, "shell")?;
            if val == py.None() {
                false
            } else {
                val.extract::<bool>(py)?
            }
        } else {
            false
        };
        let bufsize = if let Some(val) = kwargs.get_item(py, "bufsize") {
            let _ = kwargs.del_item(py, "bufsize")?;
            if val == py.None() {
                0
            } else {
                val.extract::<i32>(py)?
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

        let popen_args = PyTuple::new(py, &args.as_slice(py)[1..]);

        let protocol = protocol_factory.call(py, NoArgs, None)?;

        let ev = Classes.UnixEvents.get(py, "_UnixSelectorEventLoop")?;
        let coro = ev.call_method(
            py, "_make_subprocess_transport",
            (&self, protocol.clone_ref(py), popen_args, false,
             stdin, stdout, stderr, bufsize), Some(kwargs))?;

        let fut = PyFuture::new(py, &self)?;
        let fut_ready = fut.clone_ref(py);

        let task = PyTask::new(py, coro, &self)?;

        self.href().spawn(task.then(move |res| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            match res {
                Ok(res) => match res {
                    Ok(transport) => {
                        let result = (transport, protocol).to_py_object(py).into_object();

                        let _ = fut_ready.set(py, Ok(result));
                    },
                    Err(err) => {
                        let _ = fut_ready.set(py, Err(err));
                    }
                },
                Err(_) => {
                    let _ = fut_ready.cancel(py);
                }
            }
            Ok(())
        }));

        Ok(fut)
    }

    //
    // Create a TCP server.
    //
    // The host parameter can be a string, in that case the TCP server is bound
    // to host and port.
    //
    // The host parameter can also be a sequence of strings and in that case
    // the TCP server is bound to all hosts of the sequence. If a host
    // appears multiple times (possibly indirectly e.g. when hostnames
    // resolve to the same IP address), the server is only bound once to that
    // host.
    //
    // Return a Server object which can be used to stop the service.
    //
    def create_server(&self, protocol_factory: PyObject,
                      host: Option<PyString> = None, port: Option<u16> = None,
                      family: i32 = 0,
                      flags: i32 = addrinfo::AI_PASSIVE,
                      sock: Option<PyObject> = None,
                      backlog: i32 = 100,
                      ssl: Option<PyObject> = None,
                      reuse_address: bool = true,
                      reuse_port: bool = true) -> PyResult<PyFuture> {

        self.create_server_helper(
            py, protocol_factory, host, port, family, flags,
            sock, backlog, ssl, reuse_address, reuse_port, transport::tcp_transport_factory)
    }

    def create_http_server(&self, protocol_factory: PyObject,
                           host: Option<PyString> = None, port: Option<u16> = None,
                           family: i32 = 0,
                           flags: i32 = addrinfo::AI_PASSIVE,
                           sock: Option<PyObject> = None,
                           backlog: i32 = 100,
                           ssl: Option<PyObject> = None,
                           reuse_address: bool = true,
                           reuse_port: bool = true) -> PyResult<PyFuture> {

        self.create_server_helper(
            py, protocol_factory, host, port, family, flags,
            sock, backlog, ssl, reuse_address, reuse_port, http::http_transport_factory)
    }

    // Connect to a TCP server.
    //
    // Create a streaming transport connection to a given Internet host and
    // port: socket family AF_INET or socket.AF_INET6 depending on host (or
    // family if specified), socket type SOCK_STREAM. protocol_factory must be
    // a callable returning a protocol instance.
    //
    // This method is a coroutine which will try to establish the connection
    // in the background.  When successful, the coroutine returns a
    // (transport, protocol) pair.
    //
    def create_connection(&self, protocol_factory: PyObject,
                          host: Option<PyString>, port: Option<u16> = None,
                          ssl: Option<PyObject> = None,
                          family: i32 = 0, proto: i32 = 0,
                          flags: i32 = addrinfo::AI_PASSIVE,
                          sock: Option<PyObject> = None,
                          local_addr: Option<PyObject> = None,
                          server_hostname: Option<PyObject> = None) -> PyResult<PyFuture> {
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
                Some(ref h) => Some(h.clone_ref(py).into_object()),
                None => None,
            }
        };

        let conn = if let (&None, &None) = (&host, &port) {
            let sock = if let Some(sock) = sock {
                // Try to use supplied python connected socket object
                if ! self.is_stream_socket(py, &sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A Stream Socket was expected, got {:?}", sock)))
                }

                // check if socket is UNIX domain socket
                if self.is_uds_socket(py, &sock)? {
                    return self.create_unix_connection(
                        py, protocol_factory, None, ssl, Some(sock), server_hostname);
                }

                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "host and port was not specified and no sock specified"));
            };

            let fileno = self.get_socket_fd(py, &sock)?;
            let sockaddr = self.addr_from_socket(py, sock)?;

            // create TcpStream object
            let stream = unsafe {
                net::TcpStream::from_raw_fd(fileno as RawFd)
            };

            // tokio stream
            let stream = match TcpStream::from_stream(stream, self.href()) {
                Ok(stream) => stream,
                Err(err) => return Err(err.to_pyerr(py)),
            };

            let waiter = PyFuture::new(py, &self)?;
            future::Either::A(
                client::create_sock_connection(
                    protocol_factory, &self,
                    stream, sockaddr, ssl, server_hostname, waiter))
        } else {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "host/port and sock can not be specified at the same time"))
            }

            // exctract hostname
            let host = host.map(|s| String::from(s.to_string_lossy(py)));
            let port = port.map(|p| p.to_string());

            let evloop = self.clone_ref(py);
            let handle = self.handle(py).clone();
            let waiter = PyFuture::new(py, &self)?;

            // resolve addresses and connect
            let fut = addrinfo::lookup(&self._lookup(py),
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
                                    protocol_factory, evloop, addrs,
                                    ssl, server_hostname, waiter))
                        }
                    }
                });

            future::Either::B(fut)
        };

        let fut = PyFuture::new(py, &self)?;
        let fut_err = fut.clone_ref(py);
        let fut_conn = fut.clone_ref(py);

        self.handle(py).spawn(
            conn
                // set exception to future
                .map_err(move |e| with_py(|py| {fut_err.set(py, Err(e.to_pyerr(py)));}))
                // set transport and protocol
                .map(move |res| with_py(|py| {
                    fut_conn.set(py, Ok(res.to_py_tuple(py).into_object()));}))
        );

        Ok(fut)
    }

    //
    // Connect to a UDS client.
    //
    def create_unix_server(&self, protocol_factory: PyObject,
                           path: Option<PyObject> = None,
                           sock: Option<PyObject> = None,
                           backlog: i32 = 100,
                           ssl: Option<PyObject> = None) -> PyResult<PyFuture> {
        let path = path.unwrap_or(py.None());

        let lst = if path != py.None() {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "path and sock can not be specified at the same time"))
            }

            let s = PyString::downcast_from(py, path)?;
            let str = s.to_string(py)?;
            let path = Path::new(str.as_ref());

            UnixListener::bind(path, self.href()).map_err(|e| e.to_pyerr(py))?
        } else {
            let sock = if let Some(sock) = sock {
                if ! self.is_uds_socket(py, &sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A UNIX Domain Stream Socket was expected, got {:?}", sock)))
                }
                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "no path and sock were specified"))
            };

            // listen
            sock.call_method(py, "listen", (backlog,), None)?;

            let fileno = self.get_socket_fd(py, &sock)?;

            // create UnixListener object
            let lst = unsafe {
                unix::net::UnixListener::from_raw_fd(fileno as RawFd)
            };

            UnixListener::from_listener(lst, self.href()).map_err(|e| e.to_pyerr(py))?
        };

        let res = server::create_uds_server(
            py, self.clone_ref(py), lst, ssl, protocol_factory)?;

        return Ok(PyFuture::done_fut(py, &self, res)?)
    }

    //
    // Connect to a UDS client.
    //
    def create_unix_connection(&self, protocol_factory: PyObject,
                               path: Option<PyObject> = None,
                               ssl: Option<PyObject> = None,
                               sock: Option<PyObject> = None,
                               server_hostname: Option<PyObject> = None) -> PyResult<PyFuture> {
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

        let path = path.unwrap_or(py.None());

        let stream = if path != py.None() {
            if let Some(_) = sock {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "path and sock can not be specified at the same time"))
            }

            let s = PyString::downcast_from(py, path)?;
            let str = s.to_string(py)?;
            let path = Path::new(str.as_ref());

            UnixStream::connect(path, self.href()).map_err(|e| e.to_pyerr(py))?
        } else {
            let sock = if let Some(sock) = sock {
                if ! self.is_uds_socket(py, &sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A UNIX Domain Stream Socket was expected, got {:?}", sock)))
                }
                sock
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "no path and sock were specified"))
            };

            let fileno = self.get_socket_fd(py, &sock)?;

            // create UnixStream object
            let stream = unsafe {
                unix::net::UnixStream::from_raw_fd(fileno as RawFd)
            };

            UnixStream::from_stream(stream, self.href()).map_err(|e| e.to_pyerr(py))?
        };

        // result future
        let fut = PyFuture::new(py, &self)?;
        let fut_err = fut.clone_ref(py);
        let fut_conn = fut.clone_ref(py);

        // create transport
        let waiter = PyFuture::new(py, &self)?;
        let result = transport::tcp_transport_factory(
            &self, false, &protocol_factory, &ssl, server_hostname,
            stream, None, None, Some(waiter.clone_ref(py)))
            .map_err(|e| e.to_pyerr(py))?;

        // wait waiter completion
        self.handle(py).spawn(
            waiter
            // set exception to future
                .map_err(move |e| with_py(|py| {
                    fut_err.set(
                        py, Err(PyErr::new_err(py, &Classes.CancelledError, NoArgs)));}))
            // set transport and protocol
                .map(move |res| with_py(|py| {
                    fut_conn.set(py, Ok(result.to_py_tuple(py).into_object()));}))
        );

        Ok(fut)
    }

    // Return an exception handler, or None if the default one is in use.
    def get_exception_handler(&self) -> PyResult<PyObject> {
        Ok(self._exception_handler(py).borrow().clone_ref(py))
    }

    // Set handler as the new event loop exception handler.
    //
    // If handler is None, the default exception handler will
    // be set.
    //
    // If handler is a callable object, it should have a
    // signature matching '(loop, context)', where 'loop'
    // will be a reference to the active event loop, 'context'
    // will be a dict object (see `call_exception_handler()`
    // documentation for details about context).
    def set_exception_handler(&self, handler: PyObject) -> PyResult<PyObject> {
        if handler != py.None() && !handler.is_callable(py) {
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("A callable object or None is expected, got {:?}", handler)))
        } else {
            *self._exception_handler(py).borrow_mut() = handler;
            Ok(py.None())
        }
    }

    // Call the current event loop's exception handler.
    //
    // The context argument is a dict containing the following keys:
    //
    // - 'message': Error message;
    // - 'exception' (optional): Exception object;
    // - 'future' (optional): Future instance;
    // - 'handle' (optional): Handle instance;
    // - 'protocol' (optional): Protocol instance;
    // - 'transport' (optional): Transport instance;
    // - 'socket' (optional): Socket instance;
    // - 'asyncgen' (optional): Asynchronous generator that caused
    //                          the exception.
    //
    // New keys maybe introduced in the future.
    //
    // Note: do not overload this method in an event loop subclass.
    // For custom exception handling, use the `set_exception_handler()` method.
    def call_exception_handler(&self, context: PyDict) -> PyResult<PyObject> {
        let handler = self._exception_handler(py).borrow();
        if *handler == py.None() {
            let mut log = String::new();
            let  _ = match context.get_item(py, "message") {
                Some(s) => writeln!(log, "{}", s),
                None => writeln!(log, "Unhandled exception in event loop")
            };
            if let Some(err) = context.get_item(py, "exception") {
                utils::print_exception(py, &mut log, PyErr::from_instance(py, err));
            }
            error!("{}", log);
        } else {
            let res = handler.call(py, (self.clone_ref(py), context.clone_ref(py),), None);
            if let Err(err) = res {
                // Exception in the user set custom exception handler.
                error!(
                    "Unhandled error in exception handler, exception: {:?}, context: {}",
                    err, context.into_object());
            }
        }
        Ok(py.None())
    }

    //
    // Run until stop() is called
    //
    def run_forever(&self, stop_on_sigint: bool = true) -> PyResult<PyObject> {
        if let Some(_) = *self._runner(py).borrow() {
            return Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Event loop is running already"));
        }

        let res = py.allow_threads(|| {
            CORE.with(|cell| {
                match *cell.borrow_mut() {
                    Some(ref mut core) => {
                        let rx = {
                            let gil = Python::acquire_gil();
                            let py = gil.python();

                            // set cancel sender
                            let (tx, rx) = oneshot::channel();
                            *(self._runner(py)).borrow_mut() = Some(tx);
                            rx
                        };

                        // SIGINT
                        if stop_on_sigint {
                            let ctrlc_f = tokio_signal::ctrl_c(&core.handle());
                            let ctrlc = core.run(ctrlc_f).unwrap().into_future();

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
                            match core.run(fut) {
                                Ok(status) => status,
                                Err(_) => RunStatus::Error,
                            }
                        } else {
                            match core.run(rx) {
                                Ok(_) => RunStatus::Stopped,
                                Err(_) => RunStatus::Error,
                            }
                        }
                    }
                    None => RunStatus::NoEventLoop,
                }
            })
        });

        let _ = self.stop(py);

        match res {
            RunStatus::Stopped => Ok(py.None()),
            RunStatus::CtrlC => Ok(py.None()),
            RunStatus::PyRes(res) => res,
            RunStatus::Error => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Unknown runtime error")),
            RunStatus::NoEventLoop => Err(no_loop_exc(py)),
        }
    }

    //
    // Run until the Future is done.
    //
    // If the argument is a coroutine, it is wrapped in a Task.
    //
    // WARNING: It would be disastrous to call run_until_complete()
    // with the same coroutine twice -- it would wrap it in two
    // different Tasks and that can't be good.
    //
    // Return the Future's result, or raise its exception.
    //
    def run_until_complete(&self, fut: PyObject) -> PyResult<PyObject> {
        if let Some(_) = *self._runner(py).borrow() {
            return Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Event loop is running already"))
        }

        // PyTask
        if let Ok(fut) = PyTask::downcast_from(py, fut.clone_ref(py)) {
            if !fut.is_same_loop(py, &self) {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }
            py.allow_threads(|| self.run_future(Box::new(fut)))
        // PyFuture
        } else if let Ok(fut) = PyFuture::downcast_from(py, fut.clone_ref(py)) {
            if !fut.is_same_loop(py, &self) {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }
            py.allow_threads(|| self.run_future(Box::new(fut)))
        // asyncio.Future
        } else if fut.hasattr(py, "_asyncio_future_blocking")? {
            let l = fut.getattr(py, "_loop")?;
            if l != self.to_py_object(py).into_object() {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "loop argument must agree with Future"))
            }

            let fut = PyFuture::from_fut(py, &self, fut)?;
            py.allow_threads(|| self.run_future(Box::new(fut)))
        } else {
            if utils::iscoroutine(&fut) {
                let fut = PyTask::new(py, fut.clone_ref(py), &self)?;
                py.allow_threads(|| self.run_future(Box::new(fut)))
            } else {
                return Err(PyErr::new::<exc::TypeError, _>(
                    py, "Future or Generator object is required"))
            }
        }
    }

    //
    // Event loop debug flag
    //
    def get_debug(&self) -> PyResult<bool> {
        Ok(self._debug(py).get())
    }

    def set_debug(&self, enabled: bool) -> PyResult<PyObject> {
        self._debug(py).set(enabled);
        Ok(py.None())
    }

    //
    // slow_callback_duration
    //
    property slow_callback_duration {
        get(&slf) -> PyResult<f32> {
            Ok(slf._slow_callback_duration(py).get() as f32 / 1000.0)
        }
        set(&slf, value: PyObject) -> PyResult<()> {
            let millis = utils::parse_millis(py, "slow_callback_duration", value)?;
            slf._slow_callback_duration(py).set(millis);
            Ok(())
        }
    }

});


impl TokioEventLoop {

    /// Check if ``debug`` mode is enabled
    pub fn is_debug(&self) -> bool {
        self._debug(GIL::python()).get()
    }

    /// Get reference to tokio remote handle
    pub fn remote(&self) -> &Remote {
        self._remote(GIL::python())
    }

    /// Get reference to tokio handle
    pub fn href(&self) -> &Handle {
        self.handle(GIL::python())
    }

    /// Clone tokio handle
    pub fn get_handle(&self) -> Handle {
        self.handle(GIL::python()).clone()
    }

    /// Stop with py exception
    pub fn stop_with_err(&self, py: Python, err: PyErr) {
        let runner = self._runner(py).borrow_mut().take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(Err(err));
            },
            None => (),
        }
    }

    /// set current executing task (for asyncio.Task.current_task api)
    pub fn set_current_task(&self, py: Python, task: PyObject) {
        *self._current_task(py).borrow_mut() = Some(task)
    }

    /// Run future to completion
    pub fn run_future(&self,
                      fut: Box<Future<Item=PyResult<PyObject>,
                                      Error=unsync::oneshot::Canceled>>) -> PyResult<PyObject> {
        let res = CORE.with(|cell| {
            match *cell.borrow_mut() {
                Some(ref mut core) => {
                    let rx = {
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // stop fut
                        let (tx, rx) = oneshot::channel();
                        *(self._runner(py)).borrow_mut() = Some(tx);

                        rx
                    };

                    // SIGINT
                    let ctrlc_f = tokio_signal::ctrl_c(&core.handle());
                    let ctrlc = core.run(ctrlc_f).unwrap().into_future();

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

                    // wait for completion
                    core.run(
                        fut.select2(sel).then(|res| {
                            match res {
                                Ok(future::Either::A((res, _))) => future::ok(
                                    RunStatus::PyRes(res)),
                                Ok(future::Either::B((res, _))) =>
                                    future::ok(res),
                                Err(err) => future::err(err),
                            }
                        }))
                }
                None => Ok(RunStatus::NoEventLoop),
            }
        });
        let gil = Python::acquire_gil();
        let py = gil.python();

        let _ = self.stop(py);

        match res {
            Ok(RunStatus::PyRes(res)) => res,
            Ok(RunStatus::NoEventLoop) => Err(PyErr::new::<exc::RuntimeError, _>(
                py, "There is event loop in current thread.")),
            Err(_) => Err(PyErr::new_err(py, &Classes.CancelledError, NoArgs)),
            _ => Ok(py.None())
        }
    }

    // Linux's socket.type is a bitmask that can include extra info
    // about socket, therefore we can't do simple
    // `sock_type == socket.SOCK_STREAM`.
    fn is_stream_socket(&self, py: Python, sock: &PyObject) -> PyResult<bool> {
        let stream = addrinfo::SocketType::Stream.to_int() as i32;
        let socktype: i32 = sock.getattr(py, "type")?.extract(py)?;
        Ok((socktype & stream) == stream)
    }

    fn is_uds_socket(&self, py: Python, sock: &PyObject) -> PyResult<bool> {
        if self.is_stream_socket(py, sock)? {
            let unix = addrinfo::Family::Unix.to_int() as i32;
            let family: i32 = sock.getattr(py, "family")?.extract(py)?;
            Ok((family & unix) == unix)
        } else {
            Ok(false)
        }
    }

    // Linux's socket.type is a bitmask that can include extra info
    // about socket, therefore we can't do simple
    // `sock_type == socket.SOCK_DGRAM`.
    fn _is_dgram_socket(&self, py: Python, sock: &PyObject) -> PyResult<bool> {
        let dgram = addrinfo::SocketType::DGram.to_int() as i32;
        let socktype: i32 = sock.getattr(py, "type")?.extract(py)?;
        Ok((socktype & dgram) == dgram)
    }

    // opened sockets only
    fn get_socket_fd(&self, py: Python, sock: &PyObject) -> PyResult<c_int> {
        let fileno: c_int = sock.call_method(py, "fileno", NoArgs, None)?.extract(py)?;
        if fileno == -1 {
            Err(PyErr::new::<exc::OSError, _>(py, "Bad file"))
        } else {
            Ok(fileno)
        }
    }

    // check if socket is blocked
    fn is_socket_nonblocking(&self, py: Python, sock: &PyObject) -> PyResult<()> {
        if self._debug(py).get() {
            let timeout = sock.call_method(py, "gettimeout", NoArgs, None)?;
            if let Ok(timeout) = timeout.extract::<c_int>(py) {
                if timeout == 0 {
                    return Ok(())
                }
            }
            Err(PyErr::new::<exc::ValueError, _>(py, "the socket must be non-blocking"))
        } else {
            return Ok(())
        }
    }

    /// Extract AddrInfo from python native socket object
    fn addr_from_socket(&self, py: Python, sock: PyObject) -> PyResult<addrinfo::AddrInfo> {
        let family: i32 = sock.getattr(py, "family")?.extract(py)?;
        let socktype: i32 = sock.getattr(py, "type")?.extract(py)?;
        let proto: i32 = sock.getattr(py, "proto")?.extract(py)?;

        let addr = PyTuple::downcast_from(
            py, sock.call_method(py, "getsockname", NoArgs, None)?)?;

        let sockaddr = if addr.len(py) == 2 {
            // parse INET
            let s = PyString::downcast_from(py, addr.get_item(py, 0))?;
            let ip = if let Ok(ip) = net::Ipv4Addr::from_str(
                s.to_string_lossy(py).as_ref()) {
                ip
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "Can not parse ip address"))
            };
            let port: u16 = addr.get_item(py, 1).extract(py)?;

            net::SocketAddr::V4(net::SocketAddrV4::new(ip, port))

        } else if addr.len(py) == 6 {
            // parse INET6
            let s = PyString::downcast_from(py, addr.get_item(py, 0))?;
            let ip = if let Ok(ip) = net::Ipv6Addr::from_str(
                s.to_string_lossy(py).as_ref()) {
                ip
            } else {
                return Err(PyErr::new::<exc::ValueError, _>(
                    py, "Can not parse ip address"))
            };
            let port: u16 = addr.get_item(py, 1).extract(py)?;
            let flowinfo: u32 = addr.get_item(py, 2).extract(py)?;
            let scope_id: u32 = addr.get_item(py, 3).extract(py)?;

            net::SocketAddr::V6(
                net::SocketAddrV6::new(ip, port, flowinfo, scope_id))

        } else {
            return Err(PyErr::new::<exc::ValueError, _>(
                py, "Unknown address type"))
        };

        Ok(addrinfo::AddrInfo::new(
            0, addrinfo::Family::from_int(family as libc::c_int),
            addrinfo::SocketType::from_int(socktype as libc::c_int),
            addrinfo::Protocol::from_int(proto as libc::c_int),
            sockaddr, None))
    }

    pub fn create_server_helper(&self, py: Python, protocol_factory: PyObject,
                                host: Option<PyString>, port: Option<u16>,
                                family: i32, flags: i32, sock: Option<PyObject>,
                                backlog: i32, ssl: Option<PyObject>,
                                reuse_address: bool, reuse_port: bool,
                                transport_factory: transport::TransportFactory)
                                -> PyResult<PyFuture> {

        if let (&None, &None) = (&host, &port) {
            if let Some(sock) = sock {
                // only stream sockets
                if ! self.is_stream_socket(py, &sock)? {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, format!("A Stream Socket was expected, got {:?}", sock)))
                }

                // check if socket is UNIX domain socket
                if self.is_uds_socket(py, &sock)? {
                    return self.create_unix_server(
                        py, protocol_factory, None, Some(sock), backlog, ssl);
                }

                // listen
                sock.call_method(py, "listen", (backlog,), None)?;

                // opened sockets only
                let fileno = self.get_socket_fd(py, &sock)?;
                let sockaddr = self.addr_from_socket(py, sock)?;

                // waiter future
                let fut = PyFuture::new(py, &self)?;
                let evloop = self.clone_ref(py);

                // create TcpListener object
                let listener = unsafe {
                    net::TcpListener::from_raw_fd(fileno as RawFd)
                };

                let res = server::create_sock_server(
                    py, self.clone_ref(py), listener, sockaddr,
                    ssl, protocol_factory, transport_factory);
                let _ = fut.set(py, res);

                return Ok(fut)
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

        // exctract hostname
        let host = host.map(|s| String::from(s.to_string_lossy(py)));

        // waiter future
        let fut = PyFuture::new(py, &self)?;
        let fut_srv = fut.clone_ref(py);
        let evloop = self.clone_ref(py);

        // resolve addresses and start listening
        let conn = addrinfo::lookup(&self._lookup(py),
                                    host, port.map(|p| p.to_string()),
                                    family, flags, addrinfo::SocketType::Stream)
            .map_err(|err| with_py(
                |py| io::Error::new(io::ErrorKind::Other, err.description()).to_pyerr(py)))
            .then(move |result| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                match result {
                    Err(err) => {
                        let _ = fut_srv.set(py, Err(err));
                    },
                    Ok(Err(err)) => {
                        let _ = fut_srv.set(py, Err(err.to_pyerr(py)));
                    },
                    Ok(Ok(addrs)) => {
                        if addrs.is_empty() {
                            let _ = fut_srv.set(
                                py, Err(PyErr::new::<exc::RuntimeError, _>(
                                    py, "getaddrinfo() returned empty list")));
                        } else {
                            let res = server::create_server(
                                py, evloop, addrs, backlog, ssl, reuse_address, reuse_port,
                                protocol_factory, transport_factory);
                            let _ = fut_srv.set(py, res);
                        }
                    }
                }
                future::ok(())
            });

        self.handle(py).spawn(conn);
        Ok(fut)
    }

    pub fn with<T, F>(&self, py: Python, message: &str, f: F)
        where F: FnOnce(Python) -> PyResult<T> {

        let gil = Python::acquire_gil();
        let py = gil.python();

        if let Err(err) = f(py) {
            self.log_exception(py, message, Some(err), None, None);
        }
    }

    pub fn log_error(&self, py: Python, err: PyErr, message: &str) -> PyErr {
        self.log_exception(py, message, Some(err.clone_ref(py)), None, None);
        err
    }

    pub fn log_exception(&self, py: Python, message: &str,
                         exception: Option<PyErr>,
                         source_traceback: Option<PyObject>,
                         kwargs: Option<&[(PyObject, PyObject)]>) {
        let _: PyResult<()> = {
            let context = PyDict::new(py);
            let _ = context.set_item(py, "message", "Future exception was never retrieved");
            source_traceback.map(
                |tb| context.set_item(py, "source_traceback", tb));
            exception.map(
                |mut exc| context.set_item(py, "exception", exc.instance(py)));

            if let Some(kwargs) = kwargs {
                for &(ref key, ref val) in kwargs {
                    let _ = context.set_item(py, key.clone_ref(py), val.clone_ref(py));
                }
            }
            let _ = self.call_exception_handler(py, context);

            Ok(())
        };
    }
}


impl PartialEq for TokioEventLoop {
    fn eq(&self, other: &TokioEventLoop) -> bool {
        let py = GIL::python();
        self.id(py) == other.id(py)
    }
}
