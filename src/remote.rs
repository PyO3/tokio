use std::thread;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use cpython::*;
use futures::future;
use tokio_core::reactor;

use handle;
use unsafepy::GIL;
use event_loop::{TokioEventLoop, new_event_loop};


pub fn spawn_event_loop(py: Python, name: &PyString) -> PyResult<RemoteTokioEventLoop> {
    let (tx, rx) = mpsc::channel();

    // start worker thread
    let _ = thread::Builder::new().name(String::from(name.to_string_lossy(py))).spawn(
        move || {
            let gil = Python::acquire_gil();
            let py = gil.python();

            if let Ok(evloop) = new_event_loop(py) {
                let _ = tx.send((evloop.clone_ref(py), evloop.remote(py)));
                let _ = evloop.run_forever(py);
            }
        }
    );

    // temp gil release
    let result = py.allow_threads(move|| {
        if let Ok(res) = rx.recv() { Some(res) } else { None }
    });

    match result {
        Some((evloop, handle)) =>
            RemoteTokioEventLoop::create_instance(py, evloop, handle),
        None => Err(PyErr::new::<exc::RuntimeError, _>(
            py, "Can not start tokio evernt loop".to_py_object(py)))
    }
}


py_class!(pub class RemoteTokioEventLoop |py| {
    data evloop: TokioEventLoop;
    data handle: reactor::Remote;

    //
    // Return the time according to the event loop's clock.
    //
    // This is a float expressed in seconds since event loop creation.
    //
    def time(&self) -> PyResult<f64> {
        self.evloop(py).time(py)
    }

    //
    // Return the time according to the event loop's clock (milliseconds)
    //
    def millis(&self) -> PyResult<u64> {
        self.evloop(py).millis(py)
    }

    //
    // Schedule callback to call later
    //
    def call_later(&self, *args, **kwargs) -> PyResult<handle::TokioTimerHandle> {
        let args = args.clone_ref(py);
        let remote = self.handle(py);

        let handle = py.allow_threads(move|| {
            let py = GIL::python();
            let (tx, rx) = mpsc::channel();
            let evloop = self.evloop(py).clone_ref(py);

            remote.spawn(move |_| {
                let res = evloop.call_later(GIL::python(), &args, None);
                tx.send(res);
                future::ok(())
            });

            match rx.recv() {
                Ok(Ok(handle)) => Some(handle),
                _ => None,
            }
        });

        match handle {
            Some(handle) => Ok(handle),
            None => Err(
                PyErr::new::<exc::RuntimeError, _>(
                    py, "Can not connect to remote event loop")),
        }
    }
    
    //
    // Stop running the event loop (it is safe to call TokioEventLoop.stop())
    //
    def stop(&self) -> PyResult<PyBool> {
        self.evloop(py).stop(py)
    }

    //
    // It is not possible to close remote event loop.
    //
    def close(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }


});
