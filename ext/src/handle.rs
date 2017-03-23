use std::cell::RefCell;
use std::time::Instant;
use cpython::*;

use futures::future::*;
use futures::sync::oneshot;
use tokio_core::reactor::{Remote, Timeout};


py_class!(pub class TimerHandle |py| {
    data cancel_handle: RefCell<Option<oneshot::Sender<()>>>;

    def cancel(&self) -> PyResult<PyObject> {
        if let Some(tx) = self.cancel_handle(py).borrow_mut().take() {
            let _ = tx.send(());
        }
        Ok(py.None())
    }
});


pub fn create_timer(py: Python, remote: &Remote, when: Instant,
                    callback: PyObject, args: PyTuple) -> PyResult<TimerHandle> {

    // python TimerHandle
    let (cancel, rx) = oneshot::channel::<()>();

    let handle = TimerHandle::create_instance(py, RefCell::new(Some(cancel)))?;
    let handle_ref = handle.clone_ref(py);

    // start timer
    remote.spawn(move |h| {
        let fut = Timeout::new_at(when, &h).unwrap().select2(rx).then(move |res| {
            // get python GIL
            let gil = Python::acquire_gil();
            let py = gil.python();

            // drop ref to handle
            handle_ref.release_ref(py);

            match res {
                Ok(Either::A(_)) => {
                    // call python callback
                    let _ = callback.call(py, args, None);
                },
                _ => ()
            };

            ok(())
        });

        fut
    });

    Ok(handle)
}
