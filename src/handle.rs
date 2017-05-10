use std::cell::{Cell, RefCell};
use std::time::Duration;

use cpython::*;
use futures::future::{self, Future};
use futures::sync::oneshot;
use tokio_core::reactor::Timeout;

use ::{TokioEventLoop, Classes, with_py};


py_class!(pub class PyHandle |py| {
    data _loop: TokioEventLoop;
    data _cancelled: Cell<bool>;
    data _cancel_handle: RefCell<Option<oneshot::Sender<()>>>;
    data _callback: PyObject;
    data _args: PyTuple;
    data _source_traceback: Option<PyObject>;

    def cancel(&self) -> PyResult<PyObject> {
        self._cancelled(py).set(true);

        if let Some(tx) = self._cancel_handle(py).borrow_mut().take() {
            let _ = tx.send(());
        }

        Ok(py.None())
    }

    property _cancelled {
        get(&slf) -> PyResult<bool> {
            Ok(slf._cancelled(py).get())
        }
    }
});


impl PyHandle {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               callback: PyObject, args: PyTuple) -> PyResult<PyHandle> {

        let tb = if evloop.is_debug() {
            let frame = Classes.Sys.call(py, "_getframe", (0,), None)?;
            Some(Classes.ExtractStack.call(py, (frame,), None)?)
        } else {
            None
        };

        PyHandle::create_instance(
            py, evloop.clone_ref(py), Cell::new(false), RefCell::new(None), callback, args, tb)
    }

    pub fn call_soon(&self, py: Python) {
        let h = self.clone_ref(py);

        // schedule work
        self._loop(py).get_handle().spawn_fn(move || {
            h.run();
            future::ok(())
        });
    }

    pub fn call_soon_threadsafe(&self, py: Python) {
        let h = self.clone_ref(py);

        // schedule work
        self._loop(py).remote().spawn(move |_| {
            h.run();
            future::ok(())
        });
    }

    pub fn call_later(&self, py: Python, when: Duration) {
        // cancel onshot
        let (cancel, rx) = oneshot::channel::<()>();
        *self._cancel_handle(py).borrow_mut() = Some(cancel);

        // we need to hold reference, otherwise python will release handle object
        let h = self.clone_ref(py);

        // start timer
        let fut = Timeout::new(when, self._loop(py).href()).unwrap().select2(rx)
            .then(move |res| {
                if let Ok(future::Either::A(_)) = res {
                    // timeout got fired, call callback
                    h.run();
                }
                future::ok(())
            });
        self._loop(py).href().spawn(fut);
    }

    pub fn run(&self) {
        let _: PyResult<()> = with_py(|py| {
            // check if cancelled
            if self._cancelled(py).get() {
                return Ok(())
            }

            let result = self._callback(py).call(py, self._args(py).clone_ref(py), None);

            // handle python exception
            if let Err(err) = result {
                if err.matches(py, &Classes.Exception) {
                    let context = PyDict::new(py);
                    context.set_item(py, "message",
                                     format!("Exception in callback {:?} {:?}",
                                             self._callback(py),
                                             self._args(py).clone_ref(py).into_object()))?;
                    context.set_item(py, "handle",
                                     format!("{:?}", self.clone_ref(py).into_object()))?;
                    context.set_item(py, "exception", err.clone_ref(py).instance(py))?;

                    if let Some(ref tb) = *self._source_traceback(py) {
                        context.set_item(py, "source_traceback", tb.clone_ref(py))?;
                    }
                    self._loop(py).call_exception_handler(py, context)?;
                } else {
                    // escalate to event loop
                    self._loop(py).stop_with_err(py, err);
                }
            }
            Ok(())
        });
    }
}
