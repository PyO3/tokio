use std::mem;
use pyo3::*;
use futures::{future, unsync, Poll};
use boxfnonce::SendBoxFnOnce;

use ::TokioEventLoop;
use utils::{Classes, PyLogger};
use pyunsafe::GIL;
use pyfuture::{_PyFuture, PyFuture, Callback, State};


#[py::class]
pub struct PyTask {
    _fut: _PyFuture,
    _waiter: Option<PyObject>,
    _must_cancel: bool,
    _blocking: bool,
}

#[py::methods]
impl PyTask {

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    fn cancel(&self, py: Python) -> PyResult<bool> {
        if !self._fut(py).done() {
            if let Some(ref waiter) = *self._waiter(py) {
                let _ = waiter.call_method(py, "cancel", NoArgs, None)?;
                return Ok(true);
            }
            *self._must_cancel_mut(py) = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    //
    // Return True if the future was cancelled
    //
    fn cancelled(&self, py: Python) -> PyResult<bool> {
        Ok(self._fut(py).cancelled())
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    fn done(&self, py: Python) -> PyResult<bool> {
        Ok(self._fut(py).done())
    }

    //
    // Return the result this future represents.
    //
    // If the future has been cancelled, raises CancelledError.  If the
    // future's result isn't yet available, raises InvalidStateError.  If
    // the future is done and has an exception set, this exception is raised.
    //
    fn result(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).result(py, true)
    }

    //
    // asyncio.gather() uses attribute
    //
    #[getter(_result)]
    fn get_result(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).get_result(py)
    }

    //
    // Return the exception that was set on this future.
    //
    // The exception (or None if no exception was set) is returned only if
    // the future is done.  If the future has been cancelled, raises
    // CancelledError.  If the future isn't done yet, raises
    // InvalidStateError.
    //
    fn exception(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).exception(py)
    }

    //
    // asyncio.gather() uses attribute
    //
    #[getter(_exception)]
    fn get_exception(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).get_exception(py)
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    fn add_done_callback(&self, py: Python, f: PyObject) -> PyResult<PyObject> {
        self._fut_mut(py).add_done_callback(
            py, f, self.clone_ref(py).into_object())
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    fn remove_done_callback(&self, py: Python, f: PyObject) -> PyResult<u32> {
        self._fut_mut(py).remove_done_callback(py, f)
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    fn set_result(&self, py: Python, result: PyObject) -> PyResult<PyObject> {
        self._fut_mut(py).set_result(
            py, result, self.clone_ref(py).into_object(), false)
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    fn set_exception(&self, py: Python, exception: PyObject) -> PyResult<PyObject> {
        self._fut_mut(py).set_exception(
            py, exception, self.clone_ref(py).into_object(), false)
    }

    //
    // awaitable
    //
    fn __iter__(&self, py: Python) -> PyResult<PyTaskIter> {
        PyTaskIter::create_instance(py, self.clone_ref(py))
    }

    // compatibility
    #[getter(_loop)]
    fn get_loop(&self, py: Python) -> PyResult<TokioEventLoop> {
        Ok(self._fut(py).evloop.clone_ref(py))
    }

    #[getter(_fut_waiter)]
    fn get_fut_waiter(&self, py: Python) -> PyResult<PyObject> {
        match *self._waiter(py) {
            Some(ref fut) => Ok(fut.clone_ref(py)),
            None => Ok(py.None())
        }
    }

    #[getter(_must_cancel)]
    fn get_must_cancel(&self, py: Python) -> PyResult<bool> {
        Ok(*self._must_cancel(py))
    }

    #[getter(_callbacks)]
    fn get_callbacks(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref cb) = self._fut(py).callbacks {
            Ok(PyTuple::new(py, cb.as_slice()).into_object())
        } else {
            Ok(py.None())
        }
    }

    #[getter(_source_traceback)]
    fn get_source_traceback(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).extract_traceback(py)
    }

    #[getter(_log_destroy_pending)]
    fn get_log_destroy_pending(&self, py: Python) -> PyResult<PyBool> {
        Ok(py.False())
    }
    #[setter(_log_destroy_pending)]
    fn set_log_destroy_pending(&self, _py: Python, _value: &PyObject) -> PyResult<()> {
        Ok(())
    }

    //
    // isfuture support
    //
    #[getter(_asyncio_future_blocking)]
    fn get_asyncio_future_blocking(&self, py: Python) -> PyResult<bool> {
        Ok(*self._blocking(py))
    }
    #[setter(_asyncio_future_blocking)]
    fn set_asyncio_future_blocking(&self, py: Python, value: bool) -> PyResult<()> {
        *self._blocking_mut(py) = value;
        Ok(())
    }
}


impl PyTask {

    pub fn new(py: Python, coro: PyObject, evloop: &TokioEventLoop) -> PyResult<PyTask> {
        let task = PyTask::create_instance(
            py,
            _PyFuture::new(evloop.clone_ref(py)), None, false, false)?;

        let fut = task.clone_ref(py);

        evloop.href().spawn_fn(move|| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            // execute one step
            task_step(py, fut, coro, None, 0);

            future::ok(())
        });

        Ok(task)
    }

    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).get(py)
    }

    //
    // Add future completion callback
    //
    pub fn add_callback(&self, py: Python, cb: Callback) {
        self._fut_mut(py).add_callback(py, cb);
    }

    //
    // bloking
    //
    pub fn is_blocking(&self) -> bool {
        *self._blocking(GIL::python())
    }

    pub fn set_blocking(&self, value: bool) {
        *self._blocking_mut(GIL::python()) = value
    }

    //
    // helpers methods
    //
    pub fn is_same_loop(&self, py: Python, evloop: &TokioEventLoop) -> bool {
        &self._fut(py).evloop == evloop
    }
}

#[py::proto]
impl PyGCProtocol for PyTask {
    //
    // Python GC support
    //
    fn __traverse__(&self, py: Python, visit: PyVisit) -> Result<(), PyTraverseError> {
        let fut = self._fut(py);
        if let Some(ref callbacks) = fut.callbacks {
            for callback in callbacks.iter() {
                let _ = visit.call(callback);
            }
        }
        Ok(())
    }

    fn __clear__(&self, py: Python) {
        let callbacks = mem::replace(&mut self._fut_mut(py).callbacks, None);
        if let Some(callbacks) = callbacks {
            for cb in callbacks {
                cb.release_ref(py);
            }
        }
    }
}

#[py::proto]
impl PyObjectProtocol for PyTask {
    fn __repr__(&self, py: Python) -> PyResult<PyObject> {
        Classes.Helpers.call(py, "future_repr", ("Task", &self,), None)
    }
}

#[py::proto]
impl PyAsyncProtocol for PyTask {

    fn __await__(&self, py: Python) -> PyResult<PyTaskIter> {
        PyTaskIter::create_instance(py, self.clone_ref(py))
    }
}

impl future::Future for PyTask {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self._fut_mut(GIL::python()).poll()
    }
}

#[py::class]
pub struct PyTaskIter {
    _fut: PyTask
}

#[py::methods]
impl PyTaskIter {

    fn send(&self, py: Python, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__(py)
    }

    fn throw(&self, py: Python, tp: PyObject, val: Option<PyObject>, _tb: Option<PyObject>) -> PyResult<Option<PyObject>> {

        if Classes.Exception.is_instance(py, &tp) {
            let val = tp;
            let tp = val.get_type(py);
            PyErr::new_lazy_init(tp, Some(val)).restore(py);
        } else {
            if let Ok(tp) = PyType::downcast_from(py, tp) {
                PyErr::new_lazy_init(tp, val).restore(py);
            } else {
                PyErr::new::<exc::TypeError, _>(py, NoArgs).restore(py);
            }
        }

        self.__next__(py)
    }
}

#[py::proto]
impl PyIterProtocol for PyTaskIter {

    fn __iter__(&self, py: Python) -> PyResult<PyTaskIter> {
        Ok(self.clone_ref(py))
    }

    fn __next__(&self, py: Python) -> PyResult<Option<PyObject>> {
        let fut = self._fut(py);

        if !fut._fut(py).done() {
            *fut._blocking_mut(py) = true;
            Ok(Some(fut.clone_ref(py).into_object()))
        } else {
            let res = fut.result(py)?;
            Err(PyErr::new::<exc::StopIteration, _>(py, (res,)))
        }
    }
}


const INPLACE_RETRY: usize = 5;

//
// wakeup task from future
//
fn wakeup_task(fut: PyTask, coro: PyObject, result: PyResult<PyObject>) {
    let gil = Python::acquire_gil();
    let py = gil.python();

    match result {
        Ok(_) => task_step(py, fut, coro, None, 0),
        Err(mut err) => task_step(py, fut, coro, Some(err.instance(py)), 0),
    }
}


//
// execute task step
//
fn task_step(py: Python, task: PyTask, coro: PyObject, exc: Option<PyObject>, retry: usize) {
    // cancel if needed
    let mut exc = exc;
    if *task._must_cancel(py) {
        exc = if let Some(exc) = exc {
            if Classes.CancelledError.is_instance(py, &exc) {
                Some(exc)
            } else {
                Some(Classes.CancelledError.call(py, NoArgs, None).unwrap())
            }
        } else {
            Some(Classes.CancelledError.call(py, NoArgs, None).unwrap())
        };
    }
    *task._waiter_mut(py) = None;

    // set current task
    task._fut(py).evloop.set_current_task(py, task.clone_ref(py).into_object());

    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        None => coro.call_method(py, "send", (py.None(),), None),
        Some(exc) => coro.call_method(py, "throw", (exc,), None),
    };

    // handle coroutine result
    match res {
        Err(mut err) => {
            if err.matches(py, &Classes.StopIteration) {
                let _ = task._fut_mut(py).set_result(
                    py, err.instance(py).getattr(py, "value").unwrap(),
                    task.clone_ref(py).into_object(), false);
            }
            else if err.matches(py, &Classes.CancelledError) {
                let _ = task._fut_mut(py).cancel(py, task.clone_ref(py).into_object());
            }
            else if err.matches(py, &Classes.BaseException) {
                task.set_exception(py, err.instance(py))
                    .into_log(py, "can not set task exception");
            }
            else {
                // log exception
                err.into_log(py, "error executing task step");
            }
        },
        Ok(result) => {
            if let Ok(true) = result.hasattr(py, "_asyncio_future_blocking") {
                if let Ok(blocking) = result.getattr(py, "_asyncio_future_blocking") {
                    if !blocking.is_true(py).unwrap() {
                        let mut err = PyErr::new::<exc::RuntimeError, _>(
                            py, format!("yield was used instead of yield from \
                                         in task {:?} with {:?}",
                                        task.clone_ref(py).into_object(), &result));

                        let waiter_task = task.clone_ref(py);
                        task._fut(py).evloop.href().spawn_fn(move|| {
                            let gil = Python::acquire_gil();
                            let py = gil.python();

                            // wakeup task
                            task_step(py, waiter_task, coro, Some(err.instance(py)), 0);

                            future::ok(())
                        });
                        return
                    }
                }
            }

            if let Ok(res) = PyFuture::downcast_from(py, result.clone_ref(py)) {
                if !res.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}",
                                    task.clone_ref(py).into_object(), &result));
                    task_step(py, task.clone_ref(py), coro, Some(err.instance(py)), 0);
                    return
                }
                res.set_blocking(false);

                // cancel if needed
                if *task._must_cancel(py) {
                    let _ = res.cancel(py);
                    *task._must_cancel_mut(py) = false
                }

                // fast path
                if res.state(py) != State::Pending && retry < INPLACE_RETRY {
                    let exc = match res.get(py) {
                        Ok(_) => None,
                        Err(ref mut err) => Some(err.instance(py)),
                    };

                    task_step(py, task, coro, exc, retry+1);
                    return
                }

                // store ref to future
                *task._waiter_mut(py) = Some(res.clone_ref(py).into_object());

                // schedule wakeup on done
                let waiter_task = task.clone_ref(py);
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));
            }
            else if let Ok(res) = PyTask::downcast_from(py, result.clone_ref(py)) {
                if !res.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}",
                                    task.clone_ref(py).into_object(), &result));
                    task_step(py, task.clone_ref(py), coro, Some(err.instance(py)), 0);
                    return
                }
                res.set_blocking(false);

                // store ref to future
                *task._waiter_mut(py) = Some(res.clone_ref(py).into_object());

                // schedule wakeup on done
                let waiter_task = task.clone_ref(py);
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));

                // cancel if needed
                if *task._must_cancel(py) {
                    let _ = res.cancel(py);
                    *task._must_cancel_mut(py) = false
                }
            }
            else if result.hasattr(py, "_asyncio_future_blocking").unwrap() {
                // wrap into PyFuture, use unwrap because if it failes then whole
                // processes is hosed
                let fut = PyFuture::from_fut(py, &task._fut(py).evloop, result).unwrap();

                // store ref to future
                *task._waiter_mut(py) = Some(fut.clone_ref(py).into_object());

                // schedule wakeup on done
                let waiter_task = task.clone_ref(py);
                let _ = fut.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));

                // cancel if needed
                if *task._must_cancel(py) {
                    let _ = fut.cancel(py);
                    *task._must_cancel_mut(py) = false
                }
            }
            else if result == py.None() {
                // call soon
                let task2 = task.clone_ref(py);
                task._fut(py).evloop.href().spawn_fn(move|| {
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // wakeup task
                    task_step(py, task2, coro, None, 0);

                    future::ok(())
                });
            }
            else {
                // Yielding something else is an error.
                let task2 = task.clone_ref(py);
                task._fut(py).evloop.href().spawn_fn(move|| {
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    let mut exc = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("Task got bad yield: {:?}", result));

                    // wakeup task
                    task_step(py, task2, coro, Some(exc.instance(py)), 0);

                    future::ok(())
                });
            }
        },
    }
}
