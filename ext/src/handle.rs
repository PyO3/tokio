use std::cell::RefCell;
use std::time::Duration;
use cpython::*;

use futures::future::{self, Future};
use futures::sync::oneshot;
use tokio_core::reactor::{Remote, Timeout};


py_class!(pub class TokioHandle |py| {
    data cancelled: RefCell<bool>;

    def cancel(&self) -> PyResult<PyObject> {
        *self.cancelled(py).borrow_mut() = true;
        Ok(py.None())
    }
});


py_class!(pub class TokioTimerHandle |py| {
    data cancel_handle: RefCell<Option<oneshot::Sender<()>>>;

    def cancel(&self) -> PyResult<PyObject> {
        if let Some(tx) = self.cancel_handle(py).borrow_mut().take() {
            let _ = tx.send(());
        }
        Ok(py.None())
    }
});


pub fn call_soon(py: Python, remote: &Remote,
                 callback: PyObject, args: PyTuple) -> PyResult<TokioHandle> {
    let handle = TokioHandle::create_instance(py, RefCell::new(false))?;
    let handle_ref = handle.clone_ref(py);

    // schedule work
    remote.spawn(move |_| {
        future::lazy(move || {
            // get python GIL
            let gil = Python::acquire_gil();
            let py = gil.python();

            // check if cancelled
            if ! *handle_ref.cancelled(py).borrow() {
                // call python callback
                let res = callback.call(py, args, None);
                match res {
                    Err(err) => {
                        println!("call_soon {:?}", err);
                        err.print(py);
                    },
                    _ => (),
                }
            }

            // drop ref to handle
            handle_ref.release_ref(py);

            future::ok(())
        })
    });

    Ok(handle)
}


pub fn call_later(py: Python, remote: &Remote, dur: Duration,
                  callback: PyObject, args: PyTuple) -> PyResult<TokioTimerHandle> {

    // python TimerHandle
    let (cancel, rx) = oneshot::channel::<()>();

    let handle = TokioTimerHandle::create_instance(py, RefCell::new(Some(cancel)))?;
    let handle_ref = handle.clone_ref(py);

    // start timer
    remote.spawn(move |h| {
        let fut = Timeout::new(dur, &h).unwrap().select2(rx).then(move |res| {
            // get python GIL
            let gil = Python::acquire_gil();
            let py = gil.python();

            // drop ref to handle
            handle_ref.release_ref(py);

            match res {
                Ok(future::Either::A(_)) => {
                    // call python callback
                    let res = callback.call(py, args, None);
                    match res {
                        Err(err) => {
                            println!("call_later error {:?}", err);
                            err.print(py);
                        },
                        _ => (),
                    }
                },
                _ => ()
            };

            future::ok(())
        });

        fut
    });

    Ok(handle)
}
