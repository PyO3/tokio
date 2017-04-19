#![allow(unused_variables)]

use std::cell;
use cpython::*;
use futures::{future, unsync, Poll};
// use futures::task::{Task, park};
use boxfnonce::SendBoxFnOnce;

use utils::{Classes, PyLogger, with_py};
use pyunsafe::{GIL, Handle, OneshotReceiver, OneshotSender};


pub fn create_future(py: Python, h: Handle) -> PyResult<PyFuture> {
    let (tx, rx) = unsync::oneshot::channel();

    PyFuture::create_instance(
        py, h,
        cell::RefCell::new(Some(OneshotSender::new(tx))),
        cell::RefCell::new(Some(OneshotReceiver::new(rx))),
        cell::Cell::new(State::Pending),
        cell::RefCell::new(py.None()),
        cell::RefCell::new(None),
        cell::RefCell::new(Some(Vec::new())),
        cell::RefCell::new(None),
    )
}

pub fn done_future(py: Python, h: Handle, result: PyObject) -> PyResult<PyFuture> {
    PyFuture::create_instance(
        py, h,
        cell::RefCell::new(None),
        cell::RefCell::new(None),
        cell::Cell::new(State::Finished),
        cell::RefCell::new(result),
        cell::RefCell::new(None),
        cell::RefCell::new(None),
        cell::RefCell::new(None),
    )
}

#[derive(Copy, Clone, Debug)]
pub enum State {
    Pending,
    Cancelled,
    Finished,
}

type Callback = SendBoxFnOnce<(PyFuture,)>;


py_class!(pub class PyFuture |py| {
    data _loop: Handle;
    data _sender: cell::RefCell<Option<OneshotSender<PyResult<PyObject>>>>;
    data _receiver: cell::RefCell<Option<OneshotReceiver<PyResult<PyObject>>>>;
    data _state: cell::Cell<State>;
    data _result: cell::RefCell<PyObject>;
    data _exception: cell::RefCell<Option<PyObject>>;
    data _callbacks: cell::RefCell<Option<Vec<PyObject>>>;

    // rust callbacks
    data _rcallbacks: cell::RefCell<Option<Vec<Callback>>>;

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    def cancel(&self) -> PyResult<bool> {
        let state = self._state(py);

        match state.get() {
            State::Pending => {
                state.set(State::Cancelled);
                Ok(true)
            }
            _ => Ok(false)
        }
    }

    //
    // Return True if the future was cancelled
    //
    def cancelled(&self) -> PyResult<bool> {
        match self._state(py).get() {
            State::Cancelled => Ok(true),
            _ => Ok(false),
        }
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    def done(&self) -> PyResult<bool> {
        match self._state(py).get() {
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
        match self._state(py).get() {
            State::Pending =>
                Err(PyErr::new_lazy_init(
                    Classes.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Result is not ready.").into_object()))),
            State::Cancelled =>
                Err(PyErr::new_lazy_init(
                    Classes.CancelledError.clone_ref(py), None)),
            State::Finished => {
                match *self._exception(py).borrow() {
                    Some(ref err) => Err(PyErr::from_instance(py, err.clone_ref(py))),
                    None => Ok(self._result(py).borrow().clone_ref(py)),
                }
            }
        }
    }

    //
    // Return the exception that was set on this future.
    //
    // The exception (or None if no exception was set) is returned only if
    // the future is done.  If the future has been cancelled, raises
    // CancelledError.  If the future isn't done yet, raises
    // InvalidStateError.
    //
    def exception(&self) -> PyResult<PyObject> {
        match self._state(py).get() {
            State::Pending =>
                Err(PyErr::new_lazy_init(
                    Classes.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Exception is not set.").into_object()))),
            State::Cancelled =>
                Err(PyErr::new_lazy_init(
                    Classes.CancelledError.clone_ref(py), None)),
            State::Finished =>
                match *self._exception(py).borrow_mut() {
                    Some(ref err) => Ok(err.clone_ref(py)),
                    None => Ok(py.None()),
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

        match self._state(py).get() {
            State::Pending => {
                // add callback, create callbacks vector if needed
                let mut callbacks = self._callbacks(py).borrow_mut();
                if let None = *callbacks {
                    *callbacks = Some(Vec::new());
                }
                if let Some(ref mut cb) = *callbacks {
                    cb.push(f.clone_ref(py));
                }
            },
            _ => {
                //let fut = self.clone_ref(py);

                // schedule callback
                //self._loop(py).spawn_fn(move|| {
                    // call python callback
                    //with_py(|py| {
                cb.call(py, (self,).to_py_object(py), None)
                    .into_log(py, "future callback error");
                //});

                //future::ok(())
            },
        }

        Ok(py.None())
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    def remove_done_callback(&self, _f: PyObject) -> PyResult<u32> {
        Ok(0)
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_result(&self, result: PyObject) -> PyResult<PyObject> {
        //println!("set result {:?}", result);
        let state = self._state(py);

        match state.get() {
            State::Pending => {
                // complete oneshot channel
                if let Some(sender) = self._sender(py).borrow_mut().take() {
                    let _ = sender.send(Ok(result.clone_ref(py)));
                }

                // set result
                state.set(State::Finished);
                *self._result(py).borrow_mut() = result;

                // schedule callbacks
                let callbacks = self._callbacks(py).borrow_mut().take();
                let mut rcallbacks = self._rcallbacks(py).borrow_mut().take();

                if let Some(callbacks) = callbacks {
                    let fut = self.clone_ref(py);
                    self._loop(py).spawn_fn(move|| {
                        with_py(move|py| {
                            // call python callback
                            for cb in callbacks.iter() {
                                cb.call(py, (fut.clone_ref(py),).to_py_object(py), None)
                                    .into_log(py, "future done callback error");
                            }

                            // call task callback
                            if let Some(ref mut rcallbacks) = rcallbacks {
                                loop {
                                    match rcallbacks.pop() {
                                        Some(cb) => cb.call(fut.clone_ref(py)),
                                        None => break
                                    }
                                }
                            }
                        });

                        future::ok(())
                    });
                }

                //self._schedule_callbacks()
                Ok(py.None())
            },
            _ => Err(PyErr::new_lazy_init(Classes.InvalidStateError.clone_ref(py), None)),
        }
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_exception(&self, exception: PyObject) -> PyResult<PyObject> {
        let state = self._state(py);

        match state.get() {
            State::Pending => {
                // check if exception is a type object
                let exc =
                    if let Ok(exception) = PyType::downcast_borrow_from(py, &exception) {
                        Some(exception.call(py, NoArgs, None)?)
                    } else {
                        None
                    };
                let exc = if let Some(exc) = exc { exc } else { exception };

                // if type(exception) is StopIteration:
                //    raise TypeError("StopIteration interacts badly with generators "
                //                    "and cannot be raised into a Future")
                state.set(State::Finished);
                *self._exception(py).borrow_mut() = Some(exc.clone_ref(py));

                // complete oneshot channel
                if let Some(sender) = self._sender(py).borrow_mut().take() {
                    let _ = sender.send(Err(PyErr::from_instance(py, exc.clone_ref(py))));
                }

                // schedule callback
                let callbacks = self._callbacks(py).borrow_mut().take();

                if let Some(callbacks) = callbacks {
                    let fut = self.clone_ref(py);
                    self._loop(py).spawn_fn(move|| {
                        with_py(|py| {
                            // call python callback
                            for cb in callbacks.iter() {
                                cb.call(py, (fut.clone_ref(py),).to_py_object(py), None)
                                    .into_log(py, "future exception callback error");
                            }
                        });
                        future::ok(())
                    })
                }

                Ok(py.None())
            }
            _ => Err(PyErr::new_lazy_init(Classes.InvalidStateError.clone_ref(py), None)),
        }
    }

    //
    // awaitable
    //
    def __iter__(&self) -> PyResult<PyFutureIter> {
        PyFutureIter::create_instance(py, self.clone_ref(py))
    }

    def __await__(&self) -> PyResult<PyFutureIter> {
        PyFutureIter::create_instance(py, self.clone_ref(py))
    }

    //
    // Python GC support
    //
    def __traverse__(&self, visit) {
        if let Some(ref callbacks) = *self._callbacks(py).borrow() {
            for callback in callbacks.iter() {
                visit.call(callback)?;
            }
        }
        Ok(())
    }

    def __clear__(&self) {
        let _ = self._callbacks(py).borrow_mut().take();
    }

});


py_class!(pub class PyFutureIter |py| {
    data _fut: PyFuture;

    def __iter__(&self) -> PyResult<PyFutureIter> {
        Ok(self.clone_ref(py))
    }

    def __next__(&self) -> PyResult<Option<PyObject>> {
        let fut = self._fut(py);

        if let State::Pending = fut._state(py).get() {
            Ok(Some(fut.clone_ref(py).into_object()))
        } else {
            let res = fut.result(py)?;
            Err(PyErr::new_lazy_init(
                Classes.StopIteration.clone_ref(py),
                Some(PyTuple::new(py, &[res]).into_object())))
        }
    }

    def send(&self, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__(py)
    }

    def throw(&self, tp: PyObject, val: Option<PyObject> = None,
              _tb: Option<PyObject> = None) -> PyResult<Option<PyObject>> {

        if Classes.Exception.is_instance(py, &tp) {
            let val = tp;
            let tp = val.get_type(py);
            PyErr::new_lazy_init(tp, Some(val)).restore(py);
        } else {
            if let Ok(tp) = PyType::downcast_from(py, tp) {
                PyErr::new_lazy_init(tp, val).restore(py);
            } else {
                PyErr::new_lazy_init(Classes.TypeError.clone_ref(py), None).restore(py);
            }
        }

        self.__next__(py)
    }

});

impl PyFuture {

    //
    // Add future completion callback
    //
    pub fn add_callback(&self, py: Python, cb: Callback) {
        match self._state(py).get() {
            State::Pending => {
                // add coro, create tasks vector if needed
                let mut callbacks = self._rcallbacks(py).borrow_mut();
                if let None = *callbacks {
                    *callbacks = Some(Vec::new());
                }
                if let Some(ref mut callbacks) = *callbacks {
                    callbacks.push(cb);
                }
            },
            _ => {
                let rfut = self.clone_ref(py);

                // schedule callback
                self._loop(py).spawn_fn(move|| {
                    cb.call(rfut);
                    future::ok(())
                })
            },
        }
    }
}

impl future::Future for PyFuture {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut rx) = *self._receiver(GIL::python()).borrow_mut() {
            rx.poll()
        } else {
            Err(unsync::oneshot::Canceled)
        }
    }
}


pub fn create_task(py: Python, coro: PyObject, handle: Handle) -> PyResult<PyFuture> {
    let fut = create_future(py, handle.clone())?;
    let fut2 = fut.clone_ref(py);

    handle.spawn_fn(move|| {
        let gil = Python::acquire_gil();
        let py = gil.python();

        // execute one step
        task_step(py, fut2, coro, None);

        future::ok(())
    });

    Ok(fut)
}


//
// wakeup task from future
//
fn wakeup_task(fut: PyFuture, coro: PyObject, rfut: PyFuture) {
    let gil = Python::acquire_gil();
    let py = gil.python();

    match rfut.result(py) {
        Ok(_) => task_step(py, fut, coro, None),
        Err(mut err) => task_step(py, fut, coro, Some(err.instance(py))),
    }
}

//
// execute task step
//
fn task_step(py: Python, fut: PyFuture, coro: PyObject, exc: Option<PyObject>) {
    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        None => coro.call_method(py, "send", PyTuple::new(py, &[py.None()]), None),
        Some(exc) => coro.call_method(py, "throw", PyTuple::new(py, &[exc]), None),
    };

    // handle coroutine result
    match res {
        Err(mut err) => {
            if err.matches(py, &Classes.StopIteration) {
                fut.set_result(py, err.instance(py).getattr(py, "value").unwrap())
                    .into_log(py, "can not get StopIteration.value");
            }
            else if err.matches(py, &Classes.CancelledError) {
                fut.cancel(py).into_log(py, "can not cancel task");
            }
            else if err.matches(py, &Classes.BaseException) {
                fut.set_exception(py, err.instance(py))
                    .into_log(py, "can not set task exception");
            }
            else {
                // log exception
                err.into_log(py, "error executing task step");
            }
        },
        Ok(result) => {
            if result == py.None() {
                // call soon
                let fut2 = fut.clone_ref(py);
                fut._loop(py).spawn_fn(move|| {
                    // get python GIL
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // wakeup task
                    task_step(py, fut2, coro, None);

                    future::ok(())
                });
            }
            else if let Ok(res) = PyFuture::downcast_from(py, result) {
                // schedule wakeup on done
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |rfut| {
                    wakeup_task(fut, coro, rfut);
                }));

                //if *self._must_cancel(py).borrow() {
                //    result.call_method(py, "cancel", NoArgs, None);
                //    *self._must_cancel(py).borrow_mut() = false
                //}
            }
        },
    }
}


//
// Cancel the future and schedule callbacks.
//
// If the future is already done or cancelled, return False.  Otherwise,
// change the future's state to cancelled, schedule the callbacks and
// return True.
//
//fn cancel(py: Python) -> PyResult<bool> {
//    let mut state = self._state(py).borrow_mut();

//    match *self._state(py).borrow_mut() {
//        State::Pending => {
//            if let Some(ref waiter) = *self._waiter(py).borrow() {
//                if waiter.cancel(py)? {
//                    return Ok(true);
//                }
//            }
//            *self._must_cancel(py).borrow_mut() = true;
//            Ok(true)
//        }
//        _ => Ok(false)
//    }
//}
