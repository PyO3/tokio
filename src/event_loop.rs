#![allow(unused_variables)]

use std::thread;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use cpython::*;
use boxfnonce::SendBoxFnOnce;
use futures::{future, Future, Stream};
use futures::sync::{oneshot};
use tokio_core::reactor::{Core, CoreId, Remote};
use tokio_signal;

use addrinfo;
use handle;
use http;
use future::{TokioFuture, create_future, create_task};
use server;
use transport;
use utils;
use pyunsafe::Handle;


thread_local!(
    pub static CORE: RefCell<Option<Core>> = RefCell::new(None);
);


pub fn no_loop_exc(py: Python) -> PyErr {
    let cur = thread::current();
    PyErr::new::<exc::RuntimeError, _>(
        py,
        format!("There is no current event loop in thread {}.",
                cur.name().unwrap_or("unknown")).to_py_object(py))
}


pub fn new_event_loop(py: Python) -> PyResult<TokioEventLoop> {
    CORE.with(|cell| {
        let core = Core::new().unwrap();

        let evloop = TokioEventLoop::create_instance(
            py, core.id(),
            Handle::new(core.handle()), Instant::now(), RefCell::new(None), Cell::new(false));

        *cell.borrow_mut() = Some(core);
        evloop
    })
}


pub fn thread_safe_check(py: Python, id: &CoreId) -> Option<PyErr> {
    let check = CORE.with(|cell| {
        match *cell.borrow() {
            None => false,
            Some(ref core) => return core.id() == *id,
        }
    });

    if !check {
        Some(PyErr::new::<exc::RuntimeError, _>(
            py, PyString::new(
                py, "Non-thread-safe operation invoked on an event loop \
                     other than the current one")))
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
    data instant: Instant;
    data runner: RefCell<Option<oneshot::Sender<bool>>>;
    data debug: Cell<bool>;

    //
    // Create a Future object attached to the loop.
    //
    def create_future(&self) -> PyResult<TokioFuture> {
        if self.debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        create_future(py, self.handle(py).clone())
    }

    //
    // Schedule a coroutine object.
    //
    // Return a task object.
    //
    def create_task(&self, coro: PyObject) -> PyResult<TokioFuture> {
        if self.debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        create_task(py, coro, self.handle(py).clone())
    }

    //
    // Return the time according to the event loop's clock.
    //
    // This is a float expressed in seconds since event loop creation.
    //
    def time(&self) -> PyResult<f64> {
        let time = self.instant(py).elapsed();
        Ok(time.as_secs() as f64 + (time.subsec_nanos() as f64 / 1_000_000.0))
    }

    //
    // Return the time according to the event loop's clock (milliseconds)
    //
    def millis(&self) -> PyResult<u64> {
        let time = self.instant(py).elapsed();
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
    def call_soon(&self, *args, **kwargs) -> PyResult<handle::TokioHandle> {
        if self.debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        let _ = utils::check_min_length(py, args, 1)?;

        // get params
        let callback = args.get_item(py, 0);

        handle::call_soon(
            py, &self.handle(py),
            callback, PyTuple::new(py, &args.as_slice(py)[1..]))
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
    def call_later(&self, *args, **kwargs) -> PyResult<handle::TokioTimerHandle> {
        if self.debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        let _ = utils::check_min_length(py, args, 2)?;

        // get params
        let callback = args.get_item(py, 1);
        let delay = utils::parse_millis(py, "delay", args.get_item(py, 0))?;
        let when = Duration::from_millis(delay);

        handle::call_later(
            py, &self.handle(py),
            when, callback, PyTuple::new(py, &args.as_slice(py)[2..]))
    }

    //
    // def call_at(self, when, callback, *args):
    //
    // Like call_later(), but uses an absolute time.
    //
    // Absolute time corresponds to the event loop's time() method.
    //
    def call_at(&self, *args, **kwargs) -> PyResult<handle::TokioTimerHandle> {
        if self.debug(py).get() {
            if let Some(err) = thread_safe_check(py, &self.id(py)) {
                return Err(err)
            }
        }

        let _ = utils::check_min_length(py, args, 2)?;

        // get params
        let callback = args.get_item(py, 1);

        // calculate delay
        let when = utils::parse_seconds(py, "when", args.get_item(py, 0))?;
        let time = when - self.instant(py).elapsed();

        handle::call_later(
            py, &self.handle(py), time, callback, PyTuple::new(py, &args.as_slice(py)[2..]))
    }

    //
    // Stop running the event loop.
    //
    def stop(&self) -> PyResult<PyBool> {
        let runner = self.runner(py).borrow_mut().take();

        match runner  {
            Some(tx) => {
                let _ = tx.send(true);
                Ok(py.True())
            },
            None => Ok(py.False()),
        }
    }

    def is_running(&self) -> PyResult<bool> {
        Ok(match *self.runner(py).borrow() {
            Some(_) => true,
            None => false,
        })
    }

    def is_closed(&self) -> PyResult<bool> {
        CORE.with(|cell| {
            if let None = *cell.borrow() {
                Ok(true)
            } else {
                Ok(false)
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

        CORE.with(|cell| {
            cell.borrow_mut().take()
        });

        Ok(py.None())
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
                      reuse_port: bool = true) -> PyResult<server::TokioServer> {

        if let Some(ssl) = ssl {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, PyString::new(py, "ssl argument is not supported yet")));
        }

        server::create_server(
            py, protocol_factory, self.handle(py).clone(),
            Some(String::from(host.unwrap().to_string_lossy(py))), Some(port.unwrap_or(0)),
            family, flags, sock, backlog, ssl, reuse_address, reuse_port,
            transport::tcp_transport_factory)
    }

    def create_http_server(&self, protocol_factory: PyObject,
                           host: Option<PyString>, port: Option<u16> = None,
                           family: i32 = 0,
                           flags: i32 = addrinfo::AI_PASSIVE,
                           sock: Option<PyObject> = None,
                           backlog: i32 = 100,
                           ssl: Option<PyObject> = None,
                           reuse_address: bool = true,
                           reuse_port: bool = true) -> PyResult<server::TokioServer> {
        if let Some(ssl) = ssl {
            return Err(PyErr::new::<exc::TypeError, _>(
                py, PyString::new(py, "ssl argument is not supported yet")));
        }

        server::create_server(
            py, protocol_factory, self.handle(py).clone(),
            Some(String::from(host.unwrap().to_string_lossy(py))), Some(port.unwrap_or(0)),
            family, flags, sock, backlog, ssl, reuse_address, reuse_port,
            http::http_transport_factory)
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
                            *(self.runner(py)).borrow_mut() = Some(tx);
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
    def run_until_complete(&self, future: PyObject) -> PyResult<PyObject> {
        match TokioFuture::downcast_from(py, future) {
            Ok(fut) => {
                let res = py.allow_threads(|| {
                    CORE.with(|cell| {
                        match *cell.borrow_mut() {
                            Some(ref mut core) => {
                                let (rx, done_rx) = {
                                    let gil = Python::acquire_gil();
                                    let py = gil.python();

                                    // wait for future completion
                                    let (done, done_rx) = oneshot::channel::<bool>();
                                    fut.add_callback(py, SendBoxFnOnce::from(move |fut| {
                                        let _ = done.send(true);
                                    }));

                                    // stop fut
                                    let (tx, rx) = oneshot::channel::<bool>();
                                    *(self.runner(py)).borrow_mut() = Some(tx);

                                    (rx, done_rx)
                                };

                                // SIGINT
                                let ctrlc_f = tokio_signal::ctrl_c(&core.handle());
                                let ctrlc = core.run(ctrlc_f).unwrap().into_future();

                                // wait for completion
                                let _ = core.run(rx.select2(done_rx).select2(ctrlc));

                                true
                            }
                            None => false,
                        }
                    })
                });

                if res {
                    // cleanup running state
                    let _ = self.stop(py);

                    fut.result(py)
                } else {
                    Err(no_loop_exc(py))
                }
            },
            Err(_) => Err(PyErr::new::<exc::RuntimeError, _>(py, "Only TokioFuture is supported")),
        }
    }


    //
    // Event loop debug flag
    //
    def get_debug(&self) -> PyResult<bool> {
        Ok(self.debug(py).get())
    }

    //
    // Set event loop debug flag
    //
    def set_debug(&self, enabled: bool) -> PyResult<PyObject> {
        self.debug(py).set(enabled);
        Ok(py.None())
    }

});


impl TokioEventLoop {

    pub fn remote(&self, py: Python) -> Remote {
        self.handle(py).remote().clone()
    }

}
