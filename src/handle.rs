// Copyright (c) 2017-present PyO3 Project and Contributors

use std::time::Duration;

use pyo3::*;
use futures::future::{self, Future};
use futures::sync::oneshot;
use tokio_core::reactor::Timeout;
use boxfnonce::SendBoxFnOnce;

use {TokioEventLoop, Classes};
use pyunsafe::GIL;

#[py::class(freelist=250)]
pub struct PyHandle {
    evloop: Py<TokioEventLoop>,
    cancelled: bool,
    cancel_handle: Option<oneshot::Sender<()>>,
    callback: PyObject,
    args: Py<PyTuple>,
    source_traceback: Option<PyObject>,
    token: PyToken,
}

pub struct PyHandlePtr(Py<PyHandle>);


#[py::methods]
impl PyHandle {

    fn cancel(&mut self) -> PyResult<()> {
        self.cancelled = true;

        if let Some(tx) = self.cancel_handle.take() {
            let _ = tx.send(());
        }

        Ok(())
    }

    #[getter(_cancelled)]
    fn get_cancelled(&self) -> PyResult<bool> {
        Ok(self.cancelled)
    }
}


impl PyHandle {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               callback: PyObject, args: Py<PyTuple>) -> PyResult<PyHandlePtr> {

        let tb = if evloop.is_debug() {
            let frame = Classes.Sys.as_ref(py).call("_getframe", (0,), None)?;
            Some(Classes.ExtractStack.as_ref(py).call((frame,), None)?.into())
        } else {
            None
        };

        Ok(PyHandlePtr(py.init(|t| PyHandle{
            evloop: evloop.into(),
            cancelled: false,
            cancel_handle: None,
            callback: callback,
            args: args,
            source_traceback: tb,
            token: t})?))
    }

    pub fn run(&self, py: Python) {
        // check if cancelled
        if self.cancelled {
            return
        }

        let result = self.callback.call(py, self.args.clone_ref(py), None);

        // handle python exception
        if let Err(err) = result {
            if err.matches(py, &Classes.Exception) {
                let context = PyDict::new(py);
                let _ = context.set_item(
                    "message", format!("Exception in callback {:?} {:?}",
                                       self.callback, self.args));
                let _ = context.set_item("handle", format!("{:?}", self));
                let _ = context.set_item("exception", err.clone_ref(py).instance(py));

                if let Some(ref tb) = self.source_traceback {
                    let _ = context.set_item("source_traceback", tb.clone_ref(py));
                }
                let _ = self.evloop.as_ref(py).call_exception_handler(py, context);
            } else {
                // escalate to event loop
                self.evloop.as_mut(py).stop_with_err(err);
            }
        }
    }
}

impl PyHandlePtr {

    pub fn into(self) -> PyObject {
        self.0.into()
    }

    pub fn run(&self) {
        self.0.with(|py, h| h.run(py));
    }

    pub fn call_soon(&self, py: Python, evloop: &TokioEventLoop) {
        let h = self.0.clone_ref(py);

        // schedule work
        evloop.schedule_callback(SendBoxFnOnce::from(move || {
            let py = GIL::python();
            h.as_ref(py).run(py);
            py.release(h);
        }));
    }

    pub fn call_soon_threadsafe(&self, py: Python, evloop: &TokioEventLoop) {
        let h = self.0.clone_ref(py);

        // schedule work
        evloop.remote().spawn(move |_| {
            h.into_py(|py, h| h.run(py));
            future::ok(())
        });
    }

    pub fn call_later(&mut self, py: Python, evloop: &TokioEventLoop, when: Duration) {
        // cancel onshot
        let (cancel, rx) = oneshot::channel::<()>();
        self.0.as_mut(py).cancel_handle = Some(cancel);

        // we need to hold reference, otherwise python will release handle object
        let h = self.0.clone_ref(py);

        // start timer
        let fut = Timeout::new(when, evloop.href()).unwrap().select2(rx)
            .then(move |res| {
                if let Ok(future::Either::A(_)) = res {
                    // timeout got fired, call callback
                    h.into_py(|py, h| h.run(py));
                }
                future::ok(())
            });
        evloop.href().spawn(fut);
    }
}
