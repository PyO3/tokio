// Copyright (c) 2017-present PyO3 Project and Contributors

use std;
use pyo3::*;
use futures::{future, unsync, Async, Poll};
use boxfnonce::SendBoxFnOnce;

use TokioEventLoop;
use utils::{Classes, PyLogger};
use pyunsafe::{GIL, OneshotSender, OneshotReceiver};
use pyfuture::{_PyFuture, PyFuture, Callback, State};


#[py::class(freelist=250)]
pub struct PyTask {
    fut: _PyFuture,
    waiter: Option<PyObject>,
    must_cancel: bool,
    blocking: bool,

    token: PyToken,
}

#[py::methods]
impl PyTask {

    ///
    /// Cancel the future and schedule callbacks.
    ///
    /// If the future is already done or cancelled, return False.  Otherwise,
    /// change the future's state to cancelled, schedule the callbacks and
    /// return True.
    ///
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

    ///
    /// Return True if the future was cancelled
    ///
    fn cancelled(&self, _py: Python) -> PyResult<bool> {
        Ok(self.fut.cancelled())
    }

    /// Return True if the future is done.
    ///
    /// Done means either that a result / exception are available, or that the
    /// future was cancelled.
    ///
    fn done(&self, _py: Python) -> PyResult<bool> {
        Ok(self.fut.done())
    }

    ///
    /// Return the result this future represents.
    ///
    /// If the future has been cancelled, raises CancelledError.  If the
    /// future's result isn't yet available, raises InvalidStateError.  If
    /// the future is done and has an exception set, this exception is raised.
    ///
    fn result(&self, py: Python) -> PyResult<PyObject> {
        self.fut.result(py, true)
    }

    ///
    /// asyncio.gather() uses attribute
    ///
    #[getter(_result)]
    fn get_result(&self) -> PyResult<PyObject> {
        self.fut.get_result(self.py())
    }

    /// Return the exception that was set on this future.
    ///
    /// The exception (or None if no exception was set) is returned only if
    /// the future is done.  If the future has been cancelled, raises
    /// CancelledError.  If the future isn't done yet, raises InvalidStateError.
    ///
    fn exception(&self, py: Python) -> PyResult<PyObject> {
        self.fut.exception(py)
    }

    //
    // asyncio.gather() uses attribute
    //
    #[getter(_exception)]
    fn get_exception(&self) -> PyResult<PyObject> {
        self.fut.get_exception(self.py())
    }

    /// Add a callback to be run when the future becomes done.
    ///
    /// The callback is called with a single argument - the future object. If
    /// the future is already done when this is called, the callback is
    /// scheduled with call_soon.
    ///
    fn add_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<PyObject> {
        let ob: PyObject = self.into();
        self.fut.add_done_callback(py, f, ob)
    }

    /// Remove all instances of a callback from the "call when done" list.
    ///
    /// Returns the number of callbacks removed.
    ///
    fn remove_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<u32> {
        self.fut.remove_done_callback(py, f)
    }

    /// Mark the future done and set its result.
    ///
    /// If the future is already done when this method is called, raises
    /// InvalidStateError.
    ///
    fn set_result(&mut self, py: Python, result: PyObject) -> PyResult<()> {
        let ob = self.into();
        self.fut.set_result(py, result, ob)
    }

    /// Mark the future done and set an exception.
    ///
    /// If the future is already done when this method is called, raises
    /// InvalidStateError.
    ///
    fn set_exception(&mut self, py: Python, exception: &PyObjectRef) -> PyResult<()> {
        let ob = self.into();
        self.fut.set_exception(py, exception, ob)
    }

    // compatibility
    #[getter(_loop)]
    fn get_loop(&self) -> PyResult<Py<TokioEventLoop>> {
        Ok(self.fut.evloop.clone_ref(self.py()))
    }

    #[getter(_fut_waiter)]
    fn get_fut_waiter(&self) -> PyResult<PyObject> {
        match self.waiter {
            Some(ref fut) => Ok(fut.clone_ref(self.py())),
            None => Ok(self.py().None())
        }
    }

    #[getter(_must_cancel)]
    fn get_must_cancel(&self) -> PyResult<bool> {
        Ok(self.must_cancel)
    }

    #[getter(_callbacks)]
    fn get_callbacks(&self) -> PyResult<PyObject> {
        if let Some(ref cb) = self.fut.callbacks {
            Ok(PyTuple::new(self.py(), cb.as_slice()).into())
        } else {
            Ok(self.py().None())
        }
    }

    #[getter(_source_traceback)]
    fn get_source_traceback(&self) -> PyResult<PyObject> {
        self.fut.extract_traceback(self.py())
    }

    #[getter(_log_destroy_pending)]
    fn get_log_destroy_pending(&self) -> PyResult<bool> {
        Ok(false)
    }
    #[setter(_log_destroy_pending)]
    fn set_log_destroy_pending(&self, _value: &PyObjectRef) -> PyResult<()> {
        Ok(())
    }

    //
    // isfuture support
    //
    #[getter(_asyncio_future_blocking)]
    fn get_asyncio_future_blocking(&self) -> PyResult<bool> {
        Ok(self.blocking)
    }
    #[setter(_asyncio_future_blocking)]
    fn set_asyncio_future_blocking(&mut self, value: bool) -> PyResult<()> {
        self.blocking = value;
        Ok(())
    }

    // generator support
    fn send(&mut self, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__()
    }

    fn throw(&mut self, py: Python, tp: &PyObjectRef,
             val: Option<PyObject>, _tb: Option<PyObject>) -> PyResult<Option<PyObject>>
    {
        if Classes.Exception.as_ref(py).is_instance(tp)? {
            let val = tp;
            let tp = val.get_type();
            PyErr::new_lazy_init(tp, Some(val.into())).restore(py);
        } else {
            if let Ok(tp) = PyType::downcast_from(tp) {
                PyErr::new_lazy_init(tp, val).restore(py);
            } else {
                PyErr::new::<exc::TypeError, _>(py, NoArgs).restore(py);
            }
        }

        self.__next__()
    }
}


impl PyTask {

    pub fn new(py: Python, coro: PyObject, evloop: &TokioEventLoop) -> PyResult<Py<PyTask>> {
        let task = py.init(|t| PyTask {
            fut:  _PyFuture::new(py, evloop.into()),
            waiter: None,
            must_cancel: false,
            blocking: false,
            token: t})?;

        // execute one step
        let fut = task.clone_ref(py);
        evloop.schedule_callback(SendBoxFnOnce::from(move || {
            let py = GIL::python();
            task_step(py, fut.as_mut(py), coro, None, 10);
        }));

        Ok(task)
    }

    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        self.fut.get(py)
    }

    /// Add future completion callback
    ///
    pub fn add_callback(&mut self, py: Python, cb: Callback) {
        self.fut.add_callback(py, cb);
    }

    // blocking
    //
    pub fn is_blocking(&self) -> bool {
        self.blocking
    }

    pub fn set_blocking(&mut self, value: bool) {
        self.blocking = value
    }

    // helpers methods
    //
    pub fn is_same_loop(&self, evloop: &TokioEventLoop) -> bool {
        self.fut.evloop.as_ptr() == evloop.as_ptr()
    }
}

/*#[py::proto]
impl PyGCProtocol for PyTask {
    //
    // Python GC support
    //
    fn __traverse__(&self, visit: PyVisit) -> Result<(), PyTraverseError> {
        if let Some(ref callbacks) = self.fut.callbacks {
            for callback in callbacks.iter() {
                let _ = visit.call(callback);
            }
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        if let Some(callbacks) = self.fut.callbacks.take() {
            for cb in callbacks {
                self.py().release(cb);
            }
        }
    }
}*/

#[py::proto]
impl PyObjectProtocol for PyTask {
    fn __repr__(&self) -> PyResult<PyObject> {
        let ob: PyObject = self.into();
        Ok(Classes.Helpers.as_ref(self.py()).call("future_repr", ("Task", ob,), None)?.into())
    }
}

#[py::proto]
impl PyAsyncProtocol for PyTask {

    fn __await__(&self) -> PyResult<PyObject> {
        Ok(self.into())
    }
}

#[py::proto]
impl PyIterProtocol for PyTask {

    fn __iter__(&mut self) -> PyResult<PyObject> {
        Ok(self.into())
    }

    fn __next__(&mut self) -> PyResult<Option<PyObject>> {
        let done = self.fut.done();

        if !done {
            self.blocking = true;
            Ok(Some(self.into()))
        } else {
            let res = self.fut.result(self.py(), true)?;
            Err(PyErr::new::<exc::StopIteration, _>(self.py(), (res,)))
        }
    }
}


pub struct PyTaskFut{
    fut: Py<PyTask>,
    rx: OneshotReceiver<PyResult<PyObject>>,
}

impl std::convert::From<Py<PyTask>> for PyTaskFut {
    fn from(ob: Py<PyTask>) -> Self {
        let (tx, rx) = unsync::oneshot::channel();

        let py = GIL::python();
        let tx = OneshotSender::new(tx);
        let _ = ob.as_mut(py).add_callback(py, SendBoxFnOnce::from(move |result| {
            let _ = tx.send(result);
        }));

        PyTaskFut{fut: ob, rx: OneshotReceiver::new(rx)}
    }
}
impl<'a> std::convert::From<&'a PyTask> for PyTaskFut {
    fn from(ob: &'a PyTask) -> Self {
        let (tx, rx) = unsync::oneshot::channel();

        let ob: Py<PyTask> = ob.into();
        let py = GIL::python();
        let tx = OneshotSender::new(tx);
        let _ = ob.as_mut(py).add_callback(py, SendBoxFnOnce::from(move |result| {
            let _ = tx.send(result);
        }));

        PyTaskFut{fut: ob, rx: OneshotReceiver::new(rx)}
    }
}

impl future::Future for PyTaskFut {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.rx.poll() {
            Ok(Async::Ready(result)) => {
                self.fut.as_mut(GIL::python()).fut.log_exc_tb.set(false);
                Ok(Async::Ready(result))
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err),
        }
    }
}


const INPLACE_RETRY: usize = 5;

///
/// wakeup task from future
///
fn wakeup_task(fut: Py<PyTask>, coro: PyObject, result: PyResult<PyObject>) {
    let py = GIL::python();
    match result {
        Ok(_) => task_step(py, fut.as_mut(py), coro, None, 0),
        Err(mut err) => task_step(py, fut.as_mut(py), coro, Some(err.instance(py)), 0),
    }
    py.release(fut);
}


///
/// execute task step
///
fn task_step(py: Python, task: &mut PyTask, coro: PyObject, exc: Option<PyObject>, retry: usize) {
    // cancel if needed
    let exc = if task.must_cancel {
        if let Some(exc) = exc {
            if Classes.CancelledError.as_ref(py).is_instance(exc.as_ref(py)).unwrap() {
                Some(exc)
            } else {
                Some(Classes.CancelledError.as_ref(py).call(NoArgs, None).unwrap().into())
            }
        } else {
            Some(Classes.CancelledError.as_ref(py).call(NoArgs, None).unwrap().into())
        }
    } else {
        exc
    };
    task.waiter = None;

    //let mut evloop = fut.evloop.as_mut(py);

    // set current task
    let task_ob = task.into();
    task.fut.evloop.as_mut(py).set_current_task(task_ob);

    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        None => coro.call_method(py, "send", (py.None(),), None),
        Some(exc) => coro.call_method(py, "throw", (exc,), None),
    };

    // handle coroutine result
    match res {
        Err(mut err) => {
            if err.matches(py, &Classes.StopIteration) {
                let ob = task.into();
                let _ = task.fut.set_result(
                    py, err.instance(py).getattr(py, "value").unwrap(), ob);
            }
            else if err.matches(py, &Classes.CancelledError) {
                let ob = task.into();
                let _ = task.fut.cancel(py, ob);
            }
            else if err.matches(py, &Classes.BaseException) {
                task.set_exception(py, err.instance(py).as_ref(py))
                    .into_log(py, "can not set task exception");
            }
            else {
                // log exception
                err.into_log(py, "error executing task step");
            }
        },
        Ok(res) => {
            let mut result = res.as_mut(py);
            if let Some(fut) = PyFuture::try_mut_exact_downcast_from(&mut result) {
                if !fut.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}", task, fut));
                    task_step(py, task, coro, Some(err.instance(py)), 0);
                    return
                }
                fut.set_blocking(false);

                // cancel if needed
                if task.must_cancel {
                    let _ = fut.cancel(py);
                    task.must_cancel = false;
                }

                // fast path
                if fut.state() != State::Pending && retry < INPLACE_RETRY {
                    let exc = match fut.get(py) {
                        Ok(_) => None,
                        Err(ref mut err) => Some(err.instance(py)),
                    };

                    task_step(py, task, coro, exc, retry+1);
                    return
                }

                // store ref to future
                task.waiter = Some(fut.into());

                // schedule wakeup on done
                let waiter_task = task.into();
                let _ = fut.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));
                return
            }

            if let Some(res) = PyTask::try_mut_exact_downcast_from(&mut result) {
                if !res.is_blocking() {
                    let mut err = PyErr::new::<exc::RuntimeError, _>(
                        py, format!("yield was used instead of yield from \
                                     in task {:?} with {:?}", task, res));
                    task_step(py, task, coro, Some(err.instance(py)), 0);
                    return
                }
                res.set_blocking(false);

                // store ref to future
                task.waiter = Some(res.into());

                // schedule wakeup on done
                let waiter_task = task.into();
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

            if let Ok(true) = result.hasattr("_asyncio_future_blocking") {
                if let Ok(blocking) = result.getattr("_asyncio_future_blocking") {
                    if !blocking.is_true().unwrap() {
                        let mut err = PyErr::new::<exc::RuntimeError, _>(
                            py, format!("yield was used instead of yield from \
                                         in task {:?} with {:?}", task, result));

                        // wakeup task
                        let waiter_task: Py<PyTask> = task.into();
                        task.fut.evloop.as_ref(py).schedule_callback(SendBoxFnOnce::from(move || {
                            let py = GIL::python();
                            task_step(py, waiter_task.as_mut(py),
                                      coro, Some(err.instance(py)), 0);
                        }));
                        return
                    }
                }

                // wrap into PyFuture, use unwrap because if it failes then whole
                // processes is hosed
                let fut = PyFuture::from_fut(
                    py, task.fut.evloop.clone_ref(py), result).unwrap();

                // store ref to future
                task.waiter = Some(fut.clone_ref(py).into());

                // schedule wakeup on done
                let waiter_task = task.into();
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

            if result.is_none() {
                // call soon
                let task2: Py<PyTask> = task.into();
                task.fut.evloop.as_ref(py).schedule_callback(SendBoxFnOnce::from(move || {
                    let py = GIL::python();
                    task_step(py, task2.as_mut(py), coro, None, 0);
                    py.release(task2);
                }));
                return
            }

            // Yielding something else is an error.
            let task2: Py<PyTask> = task.into();
            let result: PyObject = result.into();
            task.fut.evloop.as_ref(py).schedule_callback(SendBoxFnOnce::from(move || {
                let py = GIL::python();
                let mut exc = PyErr::new::<exc::RuntimeError, _>(
                    py, format!("Task got bad yield: {:?}", result));

                // wakeup task
                task_step(py, task2.as_mut(py), coro, Some(exc.instance(py)), 0);
                py.release(task2);
            }));
        },
    }
}
