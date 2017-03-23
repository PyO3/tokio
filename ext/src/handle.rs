use std::cell::RefCell;
use std::time::Duration;
use cpython::*;

use futures::future::*;
use futures::sync::oneshot;
use tokio_core::reactor::{Remote, Timeout};


py_class!(pub class Handle |py| {
    data cancelled: RefCell<bool>;

    def cancel(&self) -> PyResult<PyObject> {
        *self.cancelled(py).borrow_mut() = true;
        Ok(py.None())
    }
});


pub fn create_handle(py: Python, remote: &Remote,
                     callback: PyObject, args: PyTuple) -> PyResult<Handle> {
    let handle = Handle::create_instance(py, RefCell::new(false))?;
    let handle_ref = handle.clone_ref(py);

    // schedule work
    remote.spawn(move |_| {
        let fut = result::<(), ()>(Ok(())).then(move |_| {
            // get python GIL
            let gil = Python::acquire_gil();
            let py = gil.python();

            // check if cancelled
            if ! *handle_ref.cancelled(py).borrow() {
                // call python callback
                let res = callback.call(py, args, None);
                match res {
                    Err(err) => println!("error {:?}", err),
                    _ => (),
                }
            }

            // drop ref to handle
            handle_ref.release_ref(py);

            ok(())
        });

        fut
    });

    Ok(handle)
}


py_class!(pub class TimerHandle |py| {
    data cancel_handle: RefCell<Option<oneshot::Sender<()>>>;

    def cancel(&self) -> PyResult<PyObject> {
        if let Some(tx) = self.cancel_handle(py).borrow_mut().take() {
            let _ = tx.send(());
        }
        Ok(py.None())
    }
});


pub fn create_timer(py: Python, remote: &Remote, dur: Duration,
                    callback: PyObject, args: PyTuple) -> PyResult<TimerHandle> {

    // python TimerHandle
    let (cancel, rx) = oneshot::channel::<()>();

    let handle = TimerHandle::create_instance(py, RefCell::new(Some(cancel)))?;
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
                Ok(Either::A(_)) => {
                    // call python callback
                    let res = callback.call(py, args, None);
                    match res {
                        Err(err) => println!("error {:?}", err),
                        _ => (),
                    }
                },
                _ => ()
            };

            ok(())
        });

        fut
    });

    Ok(handle)
}
