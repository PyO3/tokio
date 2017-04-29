#![allow(unused_variables)]

use std::cell;
use cpython::*;
use futures::{future, unsync, Poll};
// use futures::task::{Task, park};
use boxfnonce::SendBoxFnOnce;

use ::TokioEventLoop;
use utils::{Classes, PyLogger};
use pyunsafe::{GIL, Handle};
use pyfuture::{_PyFuture, PyFuture, Callback, State};


py_class!(pub class PyTask |py| {
    data _loop: Option<TokioEventLoop>;
    data _handle: Handle;
    data _fut: cell::RefCell<_PyFuture>;
    data _waiter: cell::RefCell<Option<PyObject>>;
    data _must_cancel: cell::Cell<bool>;

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    def cancel(&self) -> PyResult<bool> {
        if !self._fut(py).borrow().done() {
            if let Some(ref waiter) = *self._waiter(py).borrow() {
                let _ = waiter.call_method(py, "cancel", NoArgs, None)?;
                return Ok(true);
            }
            self._must_cancel(py).set(true);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    //
    // Return True if the future was cancelled
    //
    def cancelled(&self) -> PyResult<bool> {
        Ok(self._fut(py).borrow().cancelled())
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    def done(&self) -> PyResult<bool> {
        Ok(self._fut(py).borrow().done())
    }

    //
    // Return the result this future represents.
    //
    // If the future has been cancelled, raises CancelledError.  If the
    // future's result isn't yet available, raises InvalidStateError.  If
    // the future is done and has an exception set, this exception is raised.
    //
    def result(&self) -> PyResult<PyObject> {
        self._fut(py).borrow().result(py)
    }

    //
    // asyncio.gather() uses attribute
    //
    property _result {
        get(&slf) -> PyResult<PyObject> {
            slf._fut(py).borrow().get_result(py)
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
        self._fut(py).borrow().exception(py)
    }

    //
    // asyncio.gather() uses attribute
    //
    property _exception {
        get(&slf) -> PyResult<PyObject> {
            slf._fut(py).borrow().get_exception(py)
        }
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    def add_done_callback(&self, f: PyObject) -> PyResult<PyObject> {
        self._fut(py).borrow_mut().add_done_callback(
            py, f, self.clone_ref(py).into_object())
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    def remove_done_callback(&self, f: PyObject) -> PyResult<u32> {
        self._fut(py).borrow_mut().remove_done_callback(py, f)
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_result(&self, result: PyObject) -> PyResult<PyObject> {
        self._fut(py).borrow_mut().set_result(
            py, result, self.clone_ref(py).into_object())
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_exception(&self, exception: PyObject) -> PyResult<PyObject> {
        self._fut(py).borrow_mut().set_exception(
            py, exception, self.clone_ref(py).into_object())
    }

    //
    // awaitable
    //
    def __iter__(&self) -> PyResult<PyTaskIter> {
        PyTaskIter::create_instance(py, self.clone_ref(py))
    }

    def __await__(&self) -> PyResult<PyTaskIter> {
        PyTaskIter::create_instance(py, self.clone_ref(py))
    }

    //
    // Python GC support
    //
    def __traverse__(&self, visit) {
        if let Some(callbacks) = self._fut(py).borrow_mut().callbacks.take() {
            for callback in callbacks.iter() {
                visit.call(callback)?;
            }
        }
        Ok(())
    }

    def __clear__(&self) {
        let _ = self._fut(py).borrow_mut().callbacks.take();
    }

    // compatibility
    property _source_traceback {
        get(&slf) -> PyResult<PyObject> {
            Ok(py.None())
        }
    }

    property _log_destroy_pending {
        get(&slf) -> PyResult<PyBool> {
            Ok(py.False())
        }
        set(&slf, value: PyObject) -> PyResult<()> {
            Ok(())
        }
    }

    property _asyncio_future_blocking {
        get(&slf) -> PyResult<PyBool> {
            Ok(py.False())
        }
    }

});


impl PyTask {

    pub fn new(py: Python, coro: PyObject,
               evloop: Option<TokioEventLoop>, handle: Handle) -> PyResult<PyTask> {
        let task = PyTask::create_instance(
            py, evloop, handle.clone(), cell::RefCell::new(_PyFuture::new(handle.clone())),
            cell::RefCell::new(None),
            cell::Cell::new(false),
        )?;

        let fut = task.clone_ref(py);

        handle.spawn_fn(move|| {
            let gil = Python::acquire_gil();
            let py = gil.python();

            // execute one step
            task_step(py, fut, coro, None, 0);

            future::ok(())
        });

        Ok(task)
    }

    //
    // Add future completion callback
    //
    pub fn add_callback(&self, py: Python, cb: Callback) {
        self._fut(py).borrow_mut().add_callback(py, cb);
    }
}

impl future::Future for PyTask {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self._fut(GIL::python()).borrow_mut().poll()
    }
}

py_class!(pub class PyTaskIter |py| {
    data _fut: PyTask;

    def __iter__(&self) -> PyResult<PyTaskIter> {
        Ok(self.clone_ref(py))
    }

    def __next__(&self) -> PyResult<Option<PyObject>> {
        let fut = self._fut(py);

        if !fut._fut(py).borrow().done() {
            Ok(Some(fut.clone_ref(py).into_object()))
        } else {
            let res = fut.result(py)?;
            Err(PyErr::new::<exc::StopIteration, _>(py, res))
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
                PyErr::new::<exc::TypeError, _>(py, NoArgs).restore(py);
            }
        }

        self.__next__(py)
    }
});


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
    if task._must_cancel(py).get() {
        exc = Some(Classes.CancelledError.call(py, NoArgs, None).unwrap())
    }
    *task._waiter(py).borrow_mut() = None;

    // set current task
    if let Some(ref evloop) = *task._loop(py) {
        evloop.set_current_task(py, task.clone_ref(py).into_object());
    }

    // call either coro.throw(exc) or coro.send(None).
    let res = match exc {
        None => coro.call_method(py, "send", (py.None(),), None),
        Some(exc) => coro.call_method(py, "throw", (exc,), None),
    };

    // println!("result: {:?}", res);
    // handle coroutine result
    match res {
        Err(mut err) => {
            if err.matches(py, &Classes.StopIteration) {
                task.set_result(py, err.instance(py).getattr(py, "value").unwrap())
                    .into_log(py, "can not get StopIteration.value");
            }
            else if err.matches(py, &Classes.CancelledError) {
                task.cancel(py).into_log(py, "can not cancel task");
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
            if let Ok(res) = PyFuture::downcast_from(py, result.clone_ref(py)) {
                // cancel if needed
                if task._must_cancel(py).get() {
                    let _ = res.cancel(py);
                    task._must_cancel(py).set(false)
                }

                // fast path
                if res.state(py) != State::Pending && retry < INPLACE_RETRY {
                    let exc = match res.result(py) {
                        Ok(_) => None,
                        Err(ref mut err) => Some(err.instance(py)),
                    };

                    task_step(py, task, coro, exc, retry+1);
                    return
                }

                // store ref to future
                *task._waiter(py).borrow_mut() = Some(res.clone_ref(py).into_object());

                // schedule wakeup on done
                let waiter_task = task.clone_ref(py);
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));
            }
            else if let Ok(res) = PyTask::downcast_from(py, result.clone_ref(py)) {
                // store ref to future
                *task._waiter(py).borrow_mut() = Some(res.clone_ref(py).into_object());

                // schedule wakeup on done
                let waiter_task = task.clone_ref(py);
                let _ = res.add_callback(py, SendBoxFnOnce::from(move |result| {
                    wakeup_task(waiter_task, coro, result);
                }));

                // cancel if needed
                if task._must_cancel(py).get() {
                    let _ = res.cancel(py);
                    task._must_cancel(py).set(false)
                }
            }
            else if result == py.None() {
                // call soon
                let task2 = task.clone_ref(py);
                if retry < INPLACE_RETRY {
                    task_step(py, task2, coro, None, retry+1);
                } else {
                    task._handle(py).spawn_fn(move|| {
                        let gil = Python::acquire_gil();
                        let py = gil.python();

                        // wakeup task
                        task_step(py, task2, coro, None, 0);

                        future::ok(())
                    });
                }
            }
        },
    }
}
