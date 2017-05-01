use std::cell::{Cell, RefCell};
use std::time::Duration;

use cpython::*;
use futures::future::{self, Future};
use futures::sync::oneshot;
use tokio_core::reactor::{Timeout, Remote};

use pyunsafe::Handle;
use utils::{with_py, PyLogger};


py_class!(pub class PyHandle |py| {
    data cancelled: Cell<bool>;

    def cancel(&self) -> PyResult<PyObject> {
        self.cancelled(py).set(true);
        Ok(py.None())
    }
});

py_class!(pub class PyTimerHandle |py| {
    data cancel_handle: RefCell<Option<oneshot::Sender<()>>>;

    def cancel(&self) -> PyResult<PyObject> {
        if let Some(tx) = self.cancel_handle(py).borrow_mut().take() {
            let _ = tx.send(());
        }
        Ok(py.None())
    }
});


pub fn call_soon(py: Python, h: &Handle,
                 callback: PyObject, args: PyTuple) -> PyResult<PyObject> {
    let handle = PyHandle::create_instance(py, Cell::new(false))?;
    let handle_ref = handle.clone_ref(py);

    // schedule work
    h.spawn_fn(move || {
        with_py(|py| {
            // check if cancelled
            if ! handle_ref.cancelled(py).get() {
                callback.call(py, args, None)
                    .into_log(py, "call_soon callback error");
            }
        });

        future::ok(())
    });

    Ok(handle.into_object())
}


pub fn call_soon_threadsafe(py: Python, h: &Remote,
                            callback: PyObject, args: PyTuple) -> PyResult<PyObject> {
    let handle = PyHandle::create_instance(py, Cell::new(false))?;
    let handle_ref = handle.clone_ref(py);

    // schedule work
    h.spawn(move |_| {
        with_py(|py| {
            // check if cancelled
            if ! handle_ref.cancelled(py).get() {
                callback.call(py, args, None).into_log(py, "call_soon_threadsafe callback error");
            }
        });

        future::ok(())
    });

    Ok(handle.into_object())
}


pub fn call_later(py: Python, h: &Handle, dur: Duration,
                  callback: PyObject, args: PyTuple) -> PyResult<PyObject> {

    // python TimerHandle
    let (cancel, rx) = oneshot::channel::<()>();

    let handle = PyTimerHandle::create_instance(py, RefCell::new(Some(cancel)))?;

    // we need to hold reference, otherwise python will release handle object
    let handle_ref = handle.clone_ref(py);

    // start timer
    let fut = Timeout::new(dur, &h).unwrap().select2(rx).then(move |res| {
        with_py(|py| {
            // drop ref to handle
            handle_ref.release_ref(py);

            if let Ok(future::Either::A(_)) = res {
                // timeout got fired, call callback
                callback.call(py, args, None)
                    .into_log(py, "call_later callback error");
            }
        });

        future::ok(())
    });
    h.spawn(fut);

    Ok(handle.into_object())
}
