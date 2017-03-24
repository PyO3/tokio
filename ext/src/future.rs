use std::cell;
use cpython::*;
use futures::future::*;
use tokio_core::reactor::{Handle};

use utils::EXC;


pub fn create_future(py: Python, h: Handle) -> PyResult<Future> {
    Future::create_instance(
        py,
        _Handle{h: h},
        cell::RefCell::new(State::Pending),
        cell::RefCell::new(py.None()),
        cell::RefCell::new(None),
        cell::RefCell::new(Some(Vec::new())),
    )
}


pub enum State {
    Pending,
    Cancelled,
    Finished,
}


pub struct _Handle {
    h: Handle
}

unsafe impl Send for _Handle {}


py_class!(pub class Future |py| {
    data _loop: _Handle;
    data _state: cell::RefCell<State>;
    data _result: cell::RefCell<PyObject>;
    data _exception: cell::RefCell<Option<PyErr>>;
    data _callbacks: cell::RefCell<Option<Vec<PyObject>>>;

    def cancel(&self) -> PyResult<bool> {
        let mut state = self._state(py).borrow_mut();

        match *state {
            State::Pending => {
                *state = State::Cancelled;
                Ok(true)
            }
            _ => Ok(false)
        }
    }

    def cancelled(&self) -> PyResult<bool> {
        match *self._state(py).borrow() {
            State::Cancelled => Ok(true),
            _ => Ok(false),
        }
    }

    def done(&self) -> PyResult<bool> {
        match *self._state(py).borrow() {
            State::Pending => Ok(false),
            _ => Ok(true),
        }
    }

    //
    // Return the result this future represents.
    //
    // If the future has been cancelled, raises CancelledError.  If the
    // future's result isn't yet available, raises InvalidStateError.  If
    // the future is done and has an exception set, this exception is raised.
    //
    def result(&self) -> PyResult<PyObject> {
        match *self._state(py).borrow() {
            State::Pending =>
                Err(PyErr::new_lazy_init(
                    EXC.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Result is not ready.").into_object()))),
            State::Cancelled =>
                Err(PyErr::new_lazy_init(
                    EXC.CancelledError.clone_ref(py), None)),
            State::Finished => {
                match *self._exception(py).borrow() {
                    Some(ref err) => Err(err.clone_ref(py)),
                    None => Ok(self._result(py).borrow().clone_ref(py)),
                }
            }
        }
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    def add_done_callback(&self, f: &PyObject) -> PyResult<PyObject> {
        let cb = f.clone_ref(py);

        match *self._state(py).borrow() {
            State::Pending => {
                let mut callbacks = self._callbacks(py).borrow_mut();
                if let None = *callbacks {
                    *callbacks = Some(Vec::new());
                }
                if let Some(ref mut cb) = *callbacks {
                    cb.push(f.clone_ref(py));
                }
            },
            _ => {
                let fut = self.clone_ref(py);

                // schedule callback
                self._loop(py).h.spawn_fn(move|| {
                    // get python GIL
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // call python callback
                    let res = cb.call(py, (fut,).to_py_object(py), None);
                    match res {
                        Err(err) => println!("error {:?}", err),
                        _ => (),
                    }

                    ok(())
                })
            },
        }

        Ok(py.None())
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_result(&self, result: PyObject) -> PyResult<PyObject> {
        let mut state = self._state(py).borrow_mut();

        match *state {
            State::Pending => {
                *state = State::Finished;
                *self._result(py).borrow_mut() = result;

                let fut = self.clone_ref(py);
                let callbacks = self._callbacks(py).borrow_mut().take();

                // schedule callback
                if let Some(callbacks) = callbacks {
                    self._loop(py).h.spawn_fn(move|| {
                        // get python GIL
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // call python callback
                        for cb in callbacks.iter() {
                            let f = fut.clone_ref(py);
                            let res = cb.call(py, (f,).to_py_object(py), None);
                            match res {
                                Err(err) => println!("error {:?}", err),
                                _ => (),
                            }
                        }
                        ok(())
                    })
                }

                //self._schedule_callbacks()
                Ok(py.None())
            },
            _ =>
                Err(PyErr::new_lazy_init(
                    EXC.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Result is not ready.").into_object()))),
        }
    }

});
