use std::mem;
use pyo3::*;
use futures::{future, unsync, Poll};
use boxfnonce::SendBoxFnOnce;

use ::{TokioEventLoop, TokioEventLoopPtr};
use utils::{Classes, PyLogger};
use pyunsafe::GIL;
use pyfuture::{_PyFuture, PyFuture, Callback, State};


#[py::class]
pub struct PyTask {
    fut: _PyFuture,
    waiter: Option<PyObject>,
    must_cancel: bool,
    blocking: bool,

    token: PyToken,
}

#[py::ptr(PyTask)]
pub struct PyTaskPtr(PyPtr);

#[py::methods]
impl PyTask {

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    fn cancel(&mut self, py: Python) -> PyResult<bool> {
        if !self.fut.done() {
            if let Some(ref waiter) = self.waiter {
                let _ = waiter.call_method(py, "cancel", NoArgs, None)?;
                return Ok(true);
            }
            self.must_cancel = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    //
    // Return True if the future was cancelled
    //
    fn cancelled(&self, _py: Python) -> PyResult<bool> {
        Ok(self.fut.cancelled())
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    fn done(&self, _py: Python) -> PyResult<bool> {
        Ok(self.fut.done())
    }

    //
    // Return the result this future represents.
    //
    // If the future has been cancelled, raises CancelledError.  If the
    // future's result isn't yet available, raises InvalidStateError.  If
    // the future is done and has an exception set, this exception is raised.
    //
    fn result(&self, py: Python) -> PyResult<PyObject> {
        self.fut.result(py, true)
    }

    //
    // asyncio.gather() uses attribute
    //
    #[getter(_result)]
    fn get_result(&self, py: Python) -> PyResult<PyObject> {
        self.fut.get_result(py)
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
        self.fut.exception(py)
    }

    //
    // asyncio.gather() uses attribute
    //
    #[getter(_exception)]
    fn get_exception(&self, py: Python) -> PyResult<PyObject> {
        self.fut.get_exception(py)
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    fn add_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<PyObject> {
        let ob = self.to_inst_ptr().into();
        self.fut.add_done_callback(py, f, ob)
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    fn remove_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<u32> {
        self.fut.remove_done_callback(py, f)
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    fn set_result(&mut self, py: Python, result: PyObject) -> PyResult<PyObject> {
        let ob = self.to_inst_ptr().into();
        self.fut.set_result(py, result, ob, false)
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    fn set_exception(&mut self, py: Python, exception: PyObject) -> PyResult<PyObject> {
        let ob = self.to_inst_ptr().into();
        self.fut.set_exception(py, exception, ob, false)
    }

    //
    // awaitable
    //
    fn __iter__(&self, py: Python) -> PyResult<PyTaskIterPtr> {
        py.init(|t| PyTaskIter{
            fut: self.to_inst_ptr(),
            token: t})
    }

    // compatibility
    #[getter(_loop)]
    fn get_loop(&self, py: Python) -> PyResult<TokioEventLoopPtr> {
        Ok(self.fut.evloop.clone_ref(py))
    }

    #[getter(_fut_waiter)]
    fn get_fut_waiter(&self, py: Python) -> PyResult<PyObject> {
        match self.waiter {
            Some(ref fut) => Ok(fut.clone_ref(py)),
            None => Ok(py.None())
        }
    }

    #[getter(_must_cancel)]
    fn get_must_cancel(&self, _py: Python) -> PyResult<bool> {
        Ok(self.must_cancel)
    }

    #[getter(_callbacks)]
    fn get_callbacks(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref cb) = self.fut.callbacks {
            Ok(PyTuple::new(py, cb.as_slice()).into())
        } else {
            Ok(py.None())
        }
    }

    #[getter(_source_traceback)]
    fn get_source_traceback(&self, py: Python) -> PyResult<PyObject> {
        self.fut.extract_traceback(py)
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
    fn get_asyncio_future_blocking(&self, _py: Python) -> PyResult<bool> {
        Ok(self.blocking)
    }
    #[setter(_asyncio_future_blocking)]
    fn set_asyncio_future_blocking(&mut self, _py: Python, value: bool) -> PyResult<()> {
        self.blocking = value;
        Ok(())
    }
}


impl PyTask {

    pub fn new(py: Python, coro: PyObject, evloop: &TokioEventLoop) -> PyResult<PyTaskPtr> {
        let task = py.init(|t| PyTask {
            fut:  _PyFuture::new(py, evloop.to_inst_ptr()),
            waiter: None,
            must_cancel: false,
            blocking: false,
            token: t})?;

        let fut = task.clone_ref(py);

        evloop.href().spawn_fn(move|| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            // execute one step
            task_step(py, fut.as_mut(py), coro, None, 0);

            future::ok(())
        });

        Ok(task)
    }

    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        self.fut.get(py)
    }

    //
    // Add future completion callback
    //
    pub fn add_callback(&mut self, py: Python, cb: Callback) {
        self.fut.add_callback(py, cb);
    }

    //
    // bloking
    //
    pub fn is_blocking(&self) -> bool {
        self.blocking
    }

    pub fn set_blocking(&mut self, value: bool) {
        self.blocking = value
    }

    //
    // helpers methods
    //
    pub fn is_same_loop(&self, evloop: &TokioEventLoop) -> bool {
        self.fut.evloop.as_ptr() == evloop.as_ptr()
    }
}

#[py::proto]
impl PyGCProtocol for PyTask {
    //
    // Python GC support
    //
    fn __traverse__(&self, _py: Python, visit: PyVisit) -> Result<(), PyTraverseError> {
        if let Some(ref callbacks) = self.fut.callbacks {
            for callback in callbacks.iter() {
                let _ = visit.call(callback);
            }
        }
        Ok(())
    }

    fn __clear__(&mut self, py: Python) {
        let callbacks = mem::replace(&mut self.fut.callbacks, None);
        if let Some(callbacks) = callbacks {
            for cb in callbacks {
                py.release(cb);
            }
        }
    }
}

#[py::proto]
impl PyObjectProtocol for PyTask {
    fn __repr__(&self, py: Python) -> PyResult<PyObject> {
        Classes.Helpers.call(py, "future_repr", ("Task", self.to_inst_ptr(),), None)
    }
}

#[py::proto]
impl PyAsyncProtocol for PyTask {

    fn __await__(&self, py: Python) -> PyResult<PyTaskIterPtr> {
        py.init(|t| PyTaskIter{fut: self.to_inst_ptr(), token: t})
    }
}

impl future::Future for PyTaskPtr {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.as_mut(GIL::python()).fut.poll()
    }
}

#[py::class]
pub struct PyTaskIter {
    fut: PyTaskPtr,
    token: PyToken,
}

#[py::ptr(PyTaskIter)]
pub struct PyTaskIterPtr(PyPtr);


#[py::methods]
impl PyTaskIter {

    fn send(&mut self, py: Python, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__(py)
    }

    fn throw(&mut self, py: Python, tp: PyObject, val: Option<PyObject>, _tb: Option<PyObject>)
             -> PyResult<Option<PyObject>>
    {
        if Classes.Exception.is_instance(py, &tp) {
            let val = tp;
            let tp = val.get_type(py);
            PyErr::new_lazy_init(tp, Some(val)).restore(py);
        } else {
            if let Ok(tp) = PyType::downcast_into(py, tp) {
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

    fn __iter__(&mut self, _py: Python) -> PyResult<PyTaskIterPtr> {
        Ok(self.to_inst_ptr())
    }

    fn __next__(&mut self, py: Python) -> PyResult<Option<PyObject>> {
        let fut = self.fut.as_mut(py);

        if !fut.fut.done() {
            fut.blocking = true;
            Ok(Some(self.fut.clone_ref(py).into()))
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
fn wakeup_task(fut: PyTaskPtr, coro: PyObject, result: PyResult<PyObject>) {
    let gil = Python::acquire_gil();
    let py = gil.python();

    match result {
        Ok(_) => task_step(py, fut.as_mut(py), coro, None, 0),
        Err(mut err) => task_step(py, fut.as_mut(py), coro, Some(err.instance(py)), 0),
    }

    py.release(fut);
}


//
// execute task step
//
fn task_step(py: Python, task: &mut PyTask, coro: PyObject, exc: Option<PyObject>, retry: usize) {
    // cancel if needed
    let mut exc = exc;
    if task.must_cancel {
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
    task.waiter = None;

    //let mut evloop = fut.evloop.as_mut(py);

    // set current task
    task.fut.evloop.as_mut(py).set_current_task(py, task.to_inst_ptr().into());

    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        None => coro.call_method(py, "send", (py.None(),), None),
        Some(exc) => coro.call_method(py, "throw", (exc,), None),
    };

    // handle coroutine result
    match res {
        Err(mut err) => {
            if err.matches(py, &Classes.StopIteration) {
                let ob = task.to_inst_ptr().into();
                let _ = task.fut.set_result(
                    py, err.instance(py).getattr(py, "value").unwrap(), ob, false);
            }
            else if err.matches(py, &Classes.CancelledError) {
                let ob = task.to_inst_ptr().into();
                let _ = task.fut.cancel(py, ob);
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
        Ok(mut result) => {
            if let Ok(true) = result.hasattr(py, "_asyncio_future_blocking") {
                if let Ok(blocking) = result.getattr(py, "_asyncio_future_blocking") {
                    if !blocking.is_true(py).unwrap() {
                        let mut err = PyErr::new::<exc::RuntimeError, _>(
                            py, format!("yield was used instead of yield from \
                                         in task {:?} with {:?}", task, &result));

                        let waiter_task = task.to_inst_ptr();
                        task.fut.evloop.as_mut(py).href().spawn_fn(move|| {
                            let gil = Python::acquire_gil();
                            let py = gil.python();

                            // wakeup task
                            task_step(py, waiter_task.as_mut(py),
                                      coro, Some(err.instance(py)), 0);

                            future::ok(())
                        });
                        return
                    }
                }
            }

            if let Ok(res) = PyFuture::downcast_mut_from(py, &mut result) {
                if !res.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}", task, res));
                    let task2 = task.to_inst_ptr();
                    task_step(py, task2.as_mut(py), coro, Some(err.instance(py)), 0);
                    py.release(task2);
                    return
                }
                res.set_blocking(false);

                // cancel if needed
                if task.must_cancel {
                    let _ = res.cancel(py);
                    task.must_cancel = false;
                }

                // fast path
                if res.state() != State::Pending && retry < INPLACE_RETRY {
                    let exc = match res.get(py) {
                        Ok(_) => None,
                        Err(ref mut err) => Some(err.instance(py)),
                    };

                    let task2 = task.to_inst_ptr();
                    task_step(py, task2.as_mut(py), coro, exc, retry+1);
                    py.release(task2);
                    return
                }

                // store ref to future
                task.waiter = Some(res.to_inst_ptr().into());

                // schedule wakeup on done
                let waiter_task = task.to_inst_ptr();
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));
                return
            }

            if let Ok(res) = PyTask::downcast_mut_from(py, &mut result) {
                if !res.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}", task, res));
                    let task2 = task.to_inst_ptr();
                    task_step(py, task2.as_mut(py), coro, Some(err.instance(py)), 0);
                    py.release(task2);
                    return
                }
                res.set_blocking(false);

                // store ref to future
                task.waiter = Some(res.to_inst_ptr().into());

                // schedule wakeup on done
                let waiter_task = task.to_inst_ptr();
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));

                // cancel if needed
                if task.must_cancel {
                    let _ = res.cancel(py);
                    task.must_cancel = false;
                }
                return
            }

            if result.hasattr(py, "_asyncio_future_blocking").unwrap() {
                // wrap into PyFuture, use unwrap because if it failes then whole
                // processes is hosed
                let fut = PyFuture::from_fut(py, task.fut.evloop.clone_ref(py), result).unwrap();

                // store ref to future
                task.waiter = Some(fut.clone_ref(py).into());

                // schedule wakeup on done
                let waiter_task = task.to_inst_ptr();
                let _ = fut.as_mut(py).add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));

                // cancel if needed
                if task.must_cancel {
                    let _ = fut.as_mut(py).cancel(py);
                    task.must_cancel = false;
                }
                return
            }

            if result.is_none(py) {
                // call soon
                let task2 = task.to_inst_ptr();
                task.fut.evloop.as_mut(py).href().spawn_fn(move|| {
                    let gil = Python::acquire_gil();
                    let py = gil.python();

                    // wakeup task
                    task_step(py, task2.as_mut(py), coro, None, 0);

                    future::ok(())
                });
                return
            }

            // Yielding something else is an error.
            let task2 = task.to_inst_ptr();
            task.fut.evloop.as_mut(py).href().spawn_fn(move|| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                let mut exc = PyErr::new::<exc::RuntimeError, _>(
                    py, format!("Task got bad yield: {:?}", &result));

                // wakeup task
                task_step(py, task2.as_mut(py), coro, Some(exc.instance(py)), 0);

                future::ok(())
            });
        },
    }
}
