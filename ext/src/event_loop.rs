use std::cell::RefCell;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use cpython::*;

use futures::sync::{oneshot};
use tokio_core::reactor::{Core, Remote};

use handle;
use utils;


thread_local!(
    pub static CORE: RefCell<Option<Core>> = RefCell::new(None);
);


pub fn close_event_loop() {
    CORE.with(|cell| {
        cell.borrow_mut().take()
    });
}


pub fn spawn_worker(py: Python, name: &PyString) -> PyResult<TokioEventLoop> {
    let (tx, rx) = mpsc::channel();
    let (tx_stop, rx_stop) = oneshot::channel::<bool>();

    // start router
    let _ = thread::Builder::new().name(String::from(name.to_string_lossy(py))).spawn(
        move || {
            CORE.with(|cell| {
                // create tokio core
                *cell.borrow_mut() = Some(Core::new().unwrap());

                if let Some(ref mut core) = *cell.borrow_mut() {
                    // send 'remote' to callee for TokioEventLoop
                    let _ = tx.send(core.remote());

                    // run loop
                    let _ = core.run(rx_stop);
                }
            });
        }
    );

    match rx.recv() {
        Ok(remote) => {
            TokioEventLoop::create_instance(
                py, remote, Instant::now(), RefCell::new(Some(tx_stop)))
        },
        Err(_) =>
            Err(PyErr::new::<exc::RuntimeError, _>(
                py, "Can not create TokioEventLoop".to_py_object(py)))
    }
}


pub fn run_event_loop(py: Python, event_loop: &TokioEventLoop) -> PyResult<PyObject> {
    CORE.with(|cell| {
        match *cell.borrow_mut() {
            Some(ref mut core) => {
                println!("Core: {:?}", core);

                let (tx, rx) = oneshot::channel::<bool>();

                *(event_loop.runner(py)).borrow_mut() = Some(tx);

                let _ = core.run(rx);
            }
            None => ()
        };
        Ok(py.None())
    })
}


pub fn new_event_loop(py: Python) -> PyResult<TokioEventLoop> {
    CORE.with(|cell| {
        let core = Core::new().unwrap();

        let evloop = TokioEventLoop::create_instance(
            py, core.remote(), Instant::now(), RefCell::new(None));

        *cell.borrow_mut() = Some(core);
        evloop
    })
}


py_class!(pub class TokioEventLoop |py| {
    data remote: Remote;
    data instant: Instant;
    data runner: RefCell<Option<oneshot::Sender<bool>>>;

    def time(&self) -> PyResult<f64> {
        let time = self.instant(py).elapsed();
        Ok(time.as_secs() as f64 + (time.subsec_nanos() as f64 / 1_000_000.0))
    }

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
    def call_soon(&self, *args, **kwargs) -> PyResult<handle::Handle> {
        let _ = utils::check_min_length(py, args, 1)?;

        // get params
        let callback = args.get_item(py, 0);

        handle::create_handle(
            py, &self.remote(py),
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
    def call_later(&self, *args, **kwargs) -> PyResult<handle::TimerHandle> {
        let _ = utils::check_min_length(py, args, 2)?;

        // get params
        let callback = args.get_item(py, 1);
        let delay = utils::parse_millis(py, "delay", args.get_item(py, 0))?;
        let when = Duration::from_millis(delay);

        handle::create_timer(
            py, &self.remote(py),
            when, callback, PyTuple::new(py, &args.as_slice(py)[2..]))
    }

    //
    // def call_at(self, when, callback, *args):
    //
    // Like call_later(), but uses an absolute time.
    //
    // Absolute time corresponds to the event loop's time() method.
    //
    def call_at(&self, *args, **kwargs) -> PyResult<handle::TimerHandle> {
        let _ = utils::check_min_length(py, args, 2)?;

        // get params
        let callback = args.get_item(py, 1);

        // calculate delay
        let when = utils::parse_seconds(py, "when", args.get_item(py, 0))?;
        let time = when - self.instant(py).elapsed();

        handle::create_timer(
            py, &self.remote(py), time, callback, PyTuple::new(py, &args.as_slice(py)[2..]))
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

    def is_closed(&self) -> PyResult<PyBool> {
        Ok(py.True())
    }

    //
    // Close the event loop. The event loop must not be running.
    //
    def close(&self) -> PyResult<PyObject> {
        if let Ok(running) = self.is_running(py) {
            if running {
                return Err(PyErr::new::<exc::RuntimeError, _>(
                    py, PyString::new(py, "Cannot close a running event loop")));
            }
        }
        debug!("Close {:?}", self.remote(py).id());

        close_event_loop();
        Ok(py.None())
    }

    //
    // Run until stop() is called
    //
    def run_forever(&self) -> PyResult<PyObject> {
        run_event_loop(py, self)
    }

    def run_until_complete(&self) -> PyResult<PyObject> {
        run_event_loop(py, self)
    }

});
