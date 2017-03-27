use std::cell;
use cpython::*;
use futures::future::*;

use utils::{Classes, Handle};


pub fn create_future(py: Python, h: Handle) -> PyResult<TokioFuture> {
    TokioFuture::create_instance(
        py, h,
        cell::RefCell::new(State::Pending),
        cell::RefCell::new(py.None()),
        cell::RefCell::new(None),
        cell::RefCell::new(Some(Vec::new())),
        cell::RefCell::new(None),
    )
}


pub enum State {
    Pending,
    Cancelled,
    Finished,
}


py_class!(pub class TokioFuture |py| {
    data _loop: Handle;
    data _state: cell::RefCell<State>;
    data _result: cell::RefCell<PyObject>;
    data _exception: cell::RefCell<Option<PyObject>>;
    data _callbacks: cell::RefCell<Option<Vec<PyObject>>>;

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
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

    //
    // Return True if the future was cancelled
    //
    def cancelled(&self) -> PyResult<bool> {
        match *self._state(py).borrow() {
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
        match *self._state(py).borrow() {
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

        match *self._state(py).borrow() {
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
                let fut = self.clone_ref(py);

                // schedule callback
                self._loop(py).spawn_fn(move|| {
                    // get python GIL
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // call python callback
                    let res = cb.call(py, (fut,).to_py_object(py), None);
                    match res {
                        Err(err) => {
                            println!("future callback error {:?}", &err);
                            err.print(py);
                        }
                        _ => (),
                    }

                    ok(())
                })
            },
        }

        Ok(py.None())
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    def remove_done_callback(&self, f: PyObject) -> PyResult<u32> {
        Ok(0)
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

                // schedule callbacks
                let callbacks = self._callbacks(py).borrow_mut().take();
                let coroutines = self._coroutines(py).borrow_mut().take();

                if let Some(callbacks) = callbacks {
                    let fut = self.clone_ref(py);
                    self._loop(py).spawn_fn(move|| {
                        // get python GIL
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // call python callback
                        for cb in callbacks.iter() {
                            let f = fut.clone_ref(py);
                            let res = cb.call(py, (f,).to_py_object(py), None);
                            match res {
                                Err(err) => {
                                    println!("future done {:?}", &err);
                                    err.print(py);
                                }
                                _ => (),
                            }
                        }

                        // call task callback
                        if let Some(mut coroutines) = coroutines {
                            loop {
                                match coroutines.pop() {
                                    Some((task, coro)) =>
                                        wakeup_task(py, task, coro, fut.clone_ref(py)),
                                    None => break
                                }
                            }
                        }

                        ok(())
                    })
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
        let mut state = self._state(py).borrow_mut();

        match *state {
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
                *state = State::Finished;
                *self._exception(py).borrow_mut() = Some(exc);

                // schedule callback
                let callbacks = self._callbacks(py).borrow_mut().take();

                if let Some(callbacks) = callbacks {
                    let fut = self.clone_ref(py);
                    self._loop(py).spawn_fn(move|| {
                        // get python GIL
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // call python callback
                        for cb in callbacks.iter() {
                            let f = fut.clone_ref(py);
                            let res = cb.call(py, (f,).to_py_object(py), None);
                            match res {
                                Err(err) => {
                                    println!("future exception {:?}", &err);
                                    err.print(py);
                                }
                                _ => (),
                            }
                        }
                        ok(())
                    })
                }

                Ok(py.None())
            }
            _ => Err(PyErr::new_lazy_init(Classes.InvalidStateError.clone_ref(py), None)),
        }
    }

    def __iter__(&self) -> PyResult<PyObject> {
        if let State::Pending = *self._state(py).borrow() {
            Ok(self.clone_ref(py).into_object())
        } else {
            self.result(py)
        }
    }

    def __next__(&self) -> PyResult<Option<PyObject>> {
        if let State::Pending = *self._state(py).borrow() {
            Ok(Some(self.clone_ref(py).into_object()))
        } else {
            Ok(None)
        }
    }

    def __await__(&self) -> PyResult<PyObject> {
        if let State::Pending = *self._state(py).borrow() {
            Ok(self.clone_ref(py).into_object())
        } else {
            self.result(py)
        }
    }

    //
    //  task scheduling support
    //

    // coroutines related to tasks
    data _coroutines: cell::RefCell<Option<Vec<(TokioFuture, PyObject)>>>;

    def add_task_callback(&self, fut: TokioFuture, coro: PyObject) -> PyResult<PyObject> {
        let co = coro.clone_ref(py);

        match *self._state(py).borrow() {
            State::Pending => {
                // add coro, create tasks vector if needed
                let mut coroutines = self._coroutines(py).borrow_mut();
                if let None = *coroutines {
                    *coroutines = Some(Vec::new());
                }
                if let Some(ref mut coroutines) = *coroutines {
                    coroutines.push((fut, co));
                }
            },
            _ => {
                let rfut = self.clone_ref(py);

                // schedule callback
                self._loop(py).spawn_fn(move|| {
                    // get python GIL
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // wakeup task
                    wakeup_task(py, fut, coro, rfut);

                    ok(())
                })
            },
        }

        Ok(py.None())
    }

});


pub fn create_task(py: Python, coro: PyObject, handle: Handle) -> PyResult<TokioFuture> {
    let fut = create_future(py, handle.clone())?;

    // schedule task
    let fut2 = fut.clone_ref(py);

    handle.spawn_fn(move|| {
        // get python GIL
        let gil = Python::acquire_gil();
        let py = gil.python();

        // call python callback
        task_step(py, fut2, coro, None);

        ok(())
    });

    Ok(fut)
}


fn wakeup_task(py: Python, fut: TokioFuture, coro: PyObject, rfut: TokioFuture) {
    if let Err(ref mut err) = rfut.result(py) {
        task_step(py, fut, coro, Some(err.instance(py)))
    } else {
        task_step(py, fut, coro, None)
    }
}

//
// execute task step
//
fn task_step(py: Python, fut: TokioFuture, coro: PyObject, exc: Option<PyObject>) {
    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        Some(exc) => coro.call_method(
            py, "throw", PyTuple::new(py, &[exc]), None),
        None => coro.call_method(
            py, "send", PyTuple::new(py, &[py.None()]), None),
    };

    // handle coroutine result
    match res {
        Err(mut err) => {
            let exc = err.instance(py);

            if Classes.StopIteration.is_instance(py, &exc) {
                if let Err(err) = fut.set_result(py, exc.getattr(py, "value").unwrap()) {
                    // log exception
                    println!("can not get StopIteration.value {:?}", &err);
                    err.print(py);
                }
            }
            else if Classes.CancelledError.is_instance(py, &exc) {
                if let Err(err) = fut.cancel(py) {
                    // log exception
                    println!("can not cancel task {:?}", &err);
                    err.print(py);
                }
            }
            else if Classes.BaseException.is_instance(py, &exc) {
                if let Err(err) = fut.set_exception(py, exc.clone_ref(py)) {
                    // log exception
                    println!("can not set task exception {:?}", &err);
                    err.print(py);
                }
            }
            // log exception
            println!("unknown task error {:?}", &err);
            err.print(py);
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

                    ok(())
                });
            }
            else if let Ok(res) = TokioFuture::downcast_from(py, result) {
                // schedule wakeup on done
                let _ = res.add_task_callback(py, fut, coro);

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
