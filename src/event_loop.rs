#![allow(unused_variables)]

use std::io;
use std::thread;
use std::net;
use std::error::Error;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use cpython::*;
use futures::{future, unsync, Future, Stream};
use futures::sync::{oneshot};
use tokio_core::reactor::{Core, CoreId, Remote};
use native_tls::TlsConnector;
use tokio_signal;

use ::{PyFuture, PyTask};
use addrinfo;
use client;
use handle;
use http;
use server;
use transport;
use utils::{self, with_py, ToPyErr};
use pyunsafe::{GIL, Handle};


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

        let evloop = TokioEventLoop::create_instance(
            py, core.id(),
            Handle::new(core.handle()),
            core.remote(),
            Instant::now(),
            addrinfo::start_workers(3),
            RefCell::new(None),
            RefCell::new(py.None()),
            Cell::new(false),
            RefCell::new(None),
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
            Some(ref id) => return id == id,
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
    Error
}


py_class!(pub class TokioEventLoop |py| {
    data id: CoreId;
    data handle: Handle;
    data _remote: Remote;
    data _instant: Instant;
    data _lookup: addrinfo::LookupWorkerSender;
    data _runner: RefCell<Option<oneshot::Sender<bool>>>;
    data _exception_handler: RefCell<PyObject>;
    data _debug: Cell<bool>;
    data _current_task: RefCell<Option<PyObject>>;

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
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("function takes at least {} arguments", 1)))
        } else {
            // get params
            let callback = args.get_item(py, 0);

            handle::call_soon(
                py, &self.handle(py),
                callback, PyTuple::new(py, &args.as_slice(py)[1..]))
        }
    }

    //
    // def call_soon_threadsafe(self, callback, *args):
    //
    // Like call_soon(), but thread-safe.
    //
    def call_soon_threadsafe(&self, *args, **kwargs) -> PyResult<PyObject> {
        if args.len(py) < 1 {
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("function takes at least {} arguments", 1)))
        } else {
            // get params
            let callback = args.get_item(py, 0);

            handle::call_soon_threadsafe(
                py, self._remote(py),
                callback, PyTuple::new(py, &args.as_slice(py)[1..]))
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

        if args.len(py) < 1 {
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("function takes at least {} arguments", 1)))
        } else {
            // get params
            let callback = args.get_item(py, 1);
            let delay = utils::parse_millis(py, "delay", args.get_item(py, 0))?;

            if delay == 0 {
                handle::call_soon(
                    py, &self.handle(py),
                    callback, PyTuple::new(py, &args.as_slice(py)[2..]))
            } else {
                let when = Duration::from_millis(delay);
                handle::call_later(
                    py, &self.handle(py),
                    when, callback, PyTuple::new(py, &args.as_slice(py)[2..]))
            }
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
            Err(PyErr::new::<exc::TypeError, _>(
                py, format!("function takes at least {} arguments", 2)))
        } else {
            // get params
            let callback = args.get_item(py, 1);

            // calculate delay
            if let Some(when) = utils::parse_seconds(py, "when", args.get_item(py, 0))? {
                let time = when - self._instant(py).elapsed();

                handle::call_later(
                    py, &self.handle(py), time, callback,
                    PyTuple::new(py, &args.as_slice(py)[2..]))
            } else {
                handle::call_soon(
                    py, &self.handle(py),
                    callback, PyTuple::new(py, &args.as_slice(py)[2..]))
            }
        }
    }

    //
    // Stop running the event loop.
    //
    def stop(&self) -> PyResult<PyBool> {
        let runner = self._runner(py).borrow_mut().take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(true);
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
        CORE.with(|cell| {
            match cell.try_borrow() {
                Ok(ref cell) => Ok(cell.is_some()),
                Err(_) => Ok(true)
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

        // drop CORE
        ID.with(|cell| {cell.borrow_mut().take()});
        CORE.with(|cell| {cell.borrow_mut().take()});

        Ok(py.None())
    }

    // return list of tuples
    // item = (family, type, proto, canonname, sockaddr)
    // sockaddr(IPV4) = (address, port)
    // sockaddr(IPV6) = (address, port, flow info, scope id)
    def getaddrinfo(&self, host: PyString, port: u16,
                    family: i32=0, socktype: i32 = 0,
                    proto: i32 = 0, flags: i32 = 0) -> PyResult<PyFuture> {
        // result future
        let res = PyFuture::new(py, &self)?;

        // create processing future
        let fut = res.clone_ref(py);
        let fut_err = res.clone_ref(py);

        // lookup process future
        let lookup = addrinfo::lookup(
            &self._lookup(py),
            String::from(host.to_string(py)?.as_ref()),
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
                    println!("addrinfo: {}", list.clone_ref(py).into_object());
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
                      host: Option<PyString>, port: Option<u16> = None,
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
                           host: Option<PyString>, port: Option<u16> = None,
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
                          server_hostname: Option<PyString> = None) -> PyResult<PyFuture> {
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

        // create_ssl context
        let ctx =
            if let Some(ssl) = ssl {
                match TlsConnector::builder() {
                    Err(err) => return Err(PyErr::new::<exc::OSError, _>(py, err.description())),
                    Ok(builder) => match builder.build() {
                        Err(err) => return Err(
                            PyErr::new::<exc::OSError, _>(py, err.description())),
                        Ok(ctx) => Some(ctx)
                    },
                }
            } else {
                None
            };

        match (&host, &port) {
            (&None, &None) => {
                if let Some(_) = sock {
                    Err(PyErr::new::<exc::ValueError, _>(
                        py, "sock is not supported yet"))
                } else {
                    Err(PyErr::new::<exc::ValueError, _>(
                        py, "host and port was not specified and no sock specified"))
                }
            },
            _ => {
                if let Some(_) = sock {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, "host/port and sock can not be specified at the same time"))
                }

                // exctract hostname
                let host = host.map(|s| String::from(s.to_string_lossy(py)))
                    .unwrap_or(String::new());

                // server hostname for ssl validation
                let server_hostname = match server_hostname {
                    Some(s) => String::from(s.to_string(py)?),
                    None => host.clone(),
                };

                let fut = PyFuture::new(py, &self)?;

                let evloop = self.clone_ref(py);
                let handle = self.handle(py).clone();
                let fut_err = fut.clone_ref(py);
                let fut_conn = fut.clone_ref(py);

                // resolve addresses and connect
                let conn = addrinfo::lookup(&self._lookup(py),
                                            host, port.unwrap_or(0),
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
                                        protocol_factory, evloop, addrs, ctx, server_hostname))
                            }
                        }
                    })
                    // set exception to future
                    .map_err(move |e| with_py(|py| {
                        fut_err.set(py, Err(e.to_pyerr(py)));}))
                    // set transport and protocol
                    .map(move |res| with_py(|py| {
                        fut_conn.set(py, Ok(res.to_py_tuple(py).into_object()));}));

                self.handle(py).spawn(conn);

                Ok(fut)
            }
        }
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
        if !handler.is_callable(py) {
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
            error!("Unhandled error in ecent loop, context: {}", context.into_object());
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
        let res = py.allow_threads(|| {
            CORE.with(|cell| {
                match *cell.borrow_mut() {
                    Some(ref mut core) => {
                        let rx = {
                            let gil = Python::acquire_gil();
                            let py = gil.python();

                            // set cancel sender
                            let (tx, rx) = oneshot::channel::<bool>();
                            *(self._runner(py)).borrow_mut() = Some(tx);
                            rx
                        };

                        // SIGINT
                        if stop_on_sigint {
                            let ctrlc_f = tokio_signal::ctrl_c(&core.handle());
                            let ctrlc = core.run(ctrlc_f).unwrap().into_future();

                            let fut = rx.select2(ctrlc).then(|res| {
                                match res {
                                    Ok(future::Either::A(_)) => future::ok(RunStatus::Stopped),
                                    Ok(future::Either::B(_)) => future::ok(RunStatus::CtrlC),
                                    Err(e) => future::err(()),
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
        let (completed, result) =
            // PyTask
            if let Ok(fut) = PyTask::downcast_from(py, fut.clone_ref(py)) {
                let fut2 = fut.clone_ref(py);
                (py.allow_threads(|| self.run_future(Box::new(fut2))), fut.get(py))
            // PyFuture
            } else if let Ok(fut) = PyFuture::downcast_from(py, fut.clone_ref(py)) {
                let fut2 = fut.clone_ref(py);
                (py.allow_threads(|| self.run_future(Box::new(fut2))), fut.get(py))
            // asyncio.Future
            } else if fut.hasattr(py, "_asyncio_future_blocking")? {
                let fut = PyFuture::from_fut(py, &self, fut)?;
                let fut2 = fut.clone_ref(py);
                (py.allow_threads(|| self.run_future(Box::new(fut2))), fut.get(py))
            } else {
                // TODO: add check for Generator object
                let fut = PyTask::new(py, fut.clone_ref(py), &self)?;
                let fut2 = fut.clone_ref(py);
                (py.allow_threads(|| self.run_future(Box::new(fut2))), fut.get(py))
            };

        if completed {
            // cleanup running state
            let _ = self.stop(py);

            result
        } else {
            Err(no_loop_exc(py))
        }
    }

    //
    // Event loop debug flag
    //
    def get_debug(&self) -> PyResult<bool> {
        Ok(self._debug(py).get())
    }

    //
    // Set event loop debug flag
    //
    def set_debug(&self, enabled: bool) -> PyResult<PyObject> {
        self._debug(py).set(enabled);
        Ok(py.None())
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

    /// set current executing task (for asyncio.Task.current_task api)
    pub fn set_current_task(&self, py: Python, task: PyObject) {
        *self._current_task(py).borrow_mut() = Some(task)
    }

    /// Run future to completion
    pub fn run_future(&self, fut: Box<Future<Item=PyResult<PyObject>,
                                             Error=unsync::oneshot::Canceled>>) -> bool {
        CORE.with(|cell| {
            match *cell.borrow_mut() {
                Some(ref mut core) => {
                    let rx = {
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // stop fut
                        let (tx, rx) = oneshot::channel::<bool>();
                        *(self._runner(py)).borrow_mut() = Some(tx);

                        rx
                    };

                    // SIGINT
                    let ctrlc_f = tokio_signal::ctrl_c(&core.handle());
                    let ctrlc = core.run(ctrlc_f).unwrap().into_future();

                    // wait for completion
                    let _ = core.run(rx.select2(fut).select2(ctrlc));

                    true
                }
                None => false,
            }
        })
    }

    pub fn create_server_helper(&self, py: Python, protocol_factory: PyObject,
                                host: Option<PyString>, port: Option<u16>,
                                family: i32, flags: i32, sock: Option<PyObject>,
                                backlog: i32, ssl: Option<PyObject>,
                                reuse_address: bool, reuse_port: bool,
                                transport_factory: transport::TransportFactory)
                                -> PyResult<PyFuture> {

        match (&host, &port) {
            (&None, &None) => {
                if let Some(_) = sock {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, "sock is not supported yet"))
                } else {
                    return Err(PyErr::new::<exc::ValueError, _>(
                        py, "Neither host/port nor sock were specified"))
                }
            },
            _ => ()
        }

        if let Some(ssl) = ssl {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, PyString::new(py, "ssl argument is not supported yet")));
        }

        // exctract hostname
        let host = host.map(|s| String::from(s.to_string_lossy(py)))
            .unwrap_or(String::new());

        // waiter future
        let fut = PyFuture::new(py, &self)?;
        let fut_srv = fut.clone_ref(py);
        let evloop = self.clone_ref(py);

        // resolve addresses and start listening
        let conn = addrinfo::lookup(&self._lookup(py),
                                    host, port.unwrap_or(0),
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
}
