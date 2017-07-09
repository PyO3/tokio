// Copyright (c) 2017-present PyO3 Project and Contributors

use std;
use std::cell;
use pyo3::*;
use futures::{future, unsync, Async, Poll};
use futures::unsync::oneshot;
use boxfnonce::SendBoxFnOnce;

use TokioEventLoop;
use utils::{Classes, PyLogger, with_py};
use pyunsafe::GIL;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum State {
    Pending,
    Cancelled,
    Finished,
}

pub type Callback = SendBoxFnOnce<(PyResult<PyObject>,)>;

pub struct _PyFuture {
    pub evloop: Py<TokioEventLoop>,
    sender: Option<oneshot::Sender<PyResult<PyObject>>>,
    receiver: Option<oneshot::Receiver<PyResult<PyObject>>>,
    state: State,
    result: Option<PyObject>,
    exception: Option<PyObject>,
    log_exc_tb: cell::Cell<bool>,
    source_tb: Option<PyObject>,
    pub callbacks: Option<Vec<PyObject>>,

    // rust callbacks
    rcallbacks: Option<Vec<Callback>>,
}

unsafe impl Send for _PyFuture {}

impl _PyFuture {

    pub fn new(py: Python, ev: Py<TokioEventLoop>) -> _PyFuture {
        let tb = _PyFuture::extract_tb(py, &ev);
        let (tx, rx) = unsync::oneshot::channel();

        _PyFuture {
            evloop: ev,
            sender: Some(tx),
            receiver: Some(rx),
            state: State::Pending,
            result: None,
            exception: None,
            log_exc_tb: cell::Cell::new(false),
            source_tb: tb,
            callbacks: None,
            rcallbacks: None,
        }
    }

    pub fn done_fut(py: Python, ev: Py<TokioEventLoop>, result: PyObject) -> _PyFuture {
        let tb = _PyFuture::extract_tb(py, &ev);

        _PyFuture {
            evloop: ev,
            sender: None,
            receiver: None,
            state: State::Finished,
            result: Some(result),
            exception: None,
            log_exc_tb: cell::Cell::new(false),
            source_tb: tb,
            callbacks: None,
            rcallbacks: None,
        }
    }

    pub fn done_res(py: Python, ev: Py<TokioEventLoop>, result: PyResult<PyObject>) -> _PyFuture
    {
        match result {
            Ok(result) => _PyFuture::done_fut(py, ev, result),
            Err(mut err) => {
                let tb = _PyFuture::extract_tb(py, &ev);

                _PyFuture {
                    evloop: ev,
                    sender: None,
                    receiver: None,
                    state: State::Finished,
                    result: None,
                    exception: Some(err.instance(py)),
                    log_exc_tb: cell::Cell::new(false),
                    source_tb: tb,
                    callbacks: None,
                    rcallbacks: None,
                }
            }
        }
    }

    fn extract_tb(py: Python, ev: &Py<TokioEventLoop>) -> Option<PyObject> {
        if ev.as_ref(py).is_debug() {
            match Classes.ExtractStack.call(py, NoArgs, None) {
                Ok(tb) => Some(tb),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    #[inline]
    pub fn state(&self) -> State {
        self.state
    }

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    pub fn cancel(&mut self, py: Python, sender: PyObject) -> bool {
        match self.state {
            State::Pending => {
                self.schedule_callbacks(py, State::Cancelled, sender, false);
                true
            }
            _ => {
                py.release(sender);
                false
            }
        }
    }

    //
    // Return True if the future was cancelled
    //
    pub fn cancelled(&self) -> bool {
        self.state == State::Cancelled
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    pub fn done(&self) -> bool {
        self.state != State::Pending
    }

    //
    // Return the result this future represents.
    //
    // If the future has been cancelled, raises CancelledError.  If the
    // future's result isn't yet available, raises InvalidStateError.  If
    // the future is done and has an exception set, this exception is raised.
    //
    pub fn result(&self, py: Python, reset_log: bool) -> PyResult<PyObject> {
        match self.state {
            State::Pending =>
                Err(PyErr::new_err(py, Classes.InvalidStateError.as_ref(py), ("Result is not ready.",))),
            State::Cancelled =>
                Err(PyErr::new_err(py, Classes.CancelledError.as_ref(py), NoArgs)),
            State::Finished => {
                if reset_log {
                    self.log_exc_tb.set(false);
                }

                match self.exception {
                    Some(ref err) => Err(PyErr::from_instance(py, err.clone_ref(py))),
                    None => match self.result {
                        Some(ref res) => Ok(res.clone_ref(py)),
                        None => Err(
                            PyErr::new::<exc::RuntimeError, _>(py, "Future result is not set"))
                    }
                }
            }
        }
    }

    pub fn get_result(&self, py: Python) -> PyResult<PyObject> {
        match self.result {
            Some(ref res) => Ok(res.clone_ref(py)),
            None => Ok(py.None())
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
    pub fn exception(&self, py: Python) -> PyResult<PyObject> {
        match self.state {
            State::Pending =>
                Err(PyErr::new_err(py, Classes.InvalidStateError.as_ref(py), "Exception is not set.")),
            State::Cancelled =>
                Err(PyErr::new_err(py, Classes.CancelledError.as_ref(py), NoArgs)),
            State::Finished =>
                match self.exception {
                    Some(ref err) => {
                        self.log_exc_tb.set(false);
                        Ok(err.clone_ref(py))
                    },
                    None => Ok(py.None()),
                }
        }
    }

    pub fn get_exception(&self, py: Python) -> PyResult<PyObject> {
        match self.exception {
            Some(ref exc) => Ok(exc.clone_ref(py)),
            None => Ok(py.None())
        }
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    pub fn add_done_callback(&mut self, py: Python,
                             f: PyObject, owner: PyObject) -> PyResult<PyObject> {
        match self.state {
            State::Pending => {
                // add callback, create callbacks vector if needed
                if let Some(ref mut callbacks) = self.callbacks {
                    callbacks.push(f);
                } else {
                    self.callbacks = Some(vec![f]);
                }
            },
            _ => {
                self.evloop.as_ref(py).href().spawn_fn(move || with_py(|py| {
                    f.call(py, (owner,), None).into_log(py, "future callback error");
                    future::ok(())
                }));
            },
        }
        Ok(py.None())
    }

    //
    // Remove all instances of a callback from the "call when done" list.
    //
    // Returns the number of callbacks removed.
    //
    pub fn remove_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<u32> {
        let (callbacks, removed) =
            if let Some(callbacks) = self.callbacks.take() {
                let mut removed = 0;
                let mut new = Vec::new();

                for cb in callbacks {
                    if cb != f {
                        new.push(cb.clone_ref(py));
                    } else {
                        removed += 1;
                    }
                }
                (new, removed)
            } else {
                return Ok(0)
            };

        if !callbacks.is_empty() {
            self.callbacks = Some(callbacks)
        }

        Ok(removed)
    }

    //
    // Return result or exception
    //
    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        match self.state {
            State::Pending =>
                Err(PyErr::new_err(py, Classes.InvalidStateError.as_ref(py), ("Result is not ready.",))),
            State::Cancelled =>
                Err(PyErr::new_err(py, Classes.CancelledError.as_ref(py), NoArgs)),
            State::Finished => {
                if let Some(ref exc) = self.exception {
                    self.log_exc_tb.set(false);
                    Err(PyErr::from_instance(py, exc.clone_ref(py)))
                } else {
                    if let Some(ref result) = self.result {
                        Ok(result.clone_ref(py))
                    } else {
                        Ok(py.None())
                    }
                }
            }
        }
    }
    
    //
    // Mark the future done and set its result.
    //
    pub fn set(&mut self, py: Python, result: PyResult<PyObject>, sender: PyObject) -> bool {
        match self.state {
            State::Pending => {
                match result {
                    Ok(result) =>
                        self.result = Some(result),
                    Err(mut err) => {
                        self.exception = Some(err.instance(py));
                        self.log_exc_tb.set(true);
                    }
                }
                self.schedule_callbacks(py, State::Finished, sender, false);
                true
            },
            _ => false
        }
    }

    //
    // Mark the future done and set its result.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    pub fn set_result(&mut self, py: Python,
                      result: PyObject, sender: PyObject, inplace: bool) -> PyResult<()> {
        let res = match self.state {
            State::Pending => {
                // set result
                self.result = Some(result);

                self.schedule_callbacks(py, State::Finished, sender, inplace);
                Ok(())
            },
            _ => {
                py.release(result);
                py.release(sender);
                Err(PyErr::new_err(py, Classes.InvalidStateError.as_ref(py), NoArgs))
            }
        };
        res
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    pub fn set_exception(&mut self, py: Python, exception: &PyObjectRef,
                         sender: PyObject, inplace: bool) -> PyResult<()>
    {
        match self.state {
            State::Pending => {
                // check if exception is a type object
                let exc =
                    if let Ok(exception) = PyType::downcast_from(exception) {
                        Some(exception.call(NoArgs, None)?)
                    } else {
                        None
                    };
                let exc = if let Some(exc) = exc { exc } else { exception };

                // StopIteration cannot be raised into a Future - CPython issue26221
                if Classes.StopIteration.as_ref(py).is_instance(exc)? {
                    return Err(PyErr::new::<exc::TypeError, _>(
                        py, "StopIteration interacts badly with generators \
                             and cannot be raised into a Future"));
                }

                self.exception = Some(exc.into());
                self.log_exc_tb.set(true);

                self.schedule_callbacks(py, State::Finished, sender, inplace);
                Ok(())
            }
            _ => {
                py.release(exception);
                py.release(sender);
                Err(PyErr::new_err(py, Classes.InvalidStateError.as_ref(py), NoArgs))
            }
        }
    }

    //
    // Add future completion callback
    //
    pub fn add_callback(&mut self, py: Python, cb: Callback) {
        match self.state {
            State::Pending => {
                // add coro, create tasks vector if needed
                if let Some(ref mut callbacks) = self.rcallbacks {
                    callbacks.push(cb);
                } else {
                    self.rcallbacks = Some(vec![cb]);
                }
            },
            _ => {
                // schedule callback
                cb.call(self.result(py, false));
            },
        }
    }

    //
    //
    pub fn schedule_callbacks(&mut self, py: Python,
                              state: State, owner: PyObject, inplace: bool)
    {
        let evloop = self.evloop.as_ref(py);

        self.state = state;

        // complete oneshot channel
        if let Some(sender) = self.sender.take() {
            if state != State::Cancelled {
                let _ = sender.send(self.result(py, false));
            }
        }

        // schedule rust callbacks
        let result = self.result(py, false);
        let mut rcallbacks = self.rcallbacks.take();
        let send_rresults = move || {
            if let Some(ref mut rcallbacks) = rcallbacks {
                with_py(move |py| {
                    loop {
                        match rcallbacks.pop() {
                            Some(cb) => {
                                match result {
                                    Ok(ref res) => cb.call(Ok(res.clone_ref(py))),
                                    Err(ref err) => cb.call(Err(err.clone_ref(py))),
                                }

                            }
                            None => break
                        };
                    }
                });
            }
            future::ok(())
        };
        if inplace {
            let _ = send_rresults();
        } else {
            evloop.href().spawn_fn(|| send_rresults());
        }

        // schedule python callbacks
        match self.callbacks.take() {
            Some(callbacks) => {
                if inplace {
                    // call python callback
                    for cb in callbacks.iter() {
                        cb.call(py, (owner.clone_ref(py),), None)
                            .into_log(py, "future done callback error");
                    }
                    py.release(owner);
                } else {
                    // call task callback
                    evloop.href().spawn_fn(move|| {
                        with_py(move |py| {
                            // call python callback
                            for cb in callbacks.iter() {
                                cb.call(py, (owner.clone_ref(py),), None)
                                    .into_log(py, "future done callback error");
                            }
                            py.release(owner);
                        });
                        future::ok(())
                    });
                }
            },
            _ => (),
        }
    }

    pub fn extract_traceback(&self, py: Python) -> PyResult<PyObject> {
        if let Some(ref tb) = self.source_tb {
            Ok(tb.clone_ref(py))
        } else {
            Ok(py.None())
        }
    }
}

impl Drop for _PyFuture {
    fn drop(&mut self) {
        let py = GIL::python();
        if self.log_exc_tb.get() {
            let context = PyDict::new(py);
            let _ = context.set_item("message", "Future exception was never retrieved");
            let _ = context.set_item("future", "PyFuture");
            if let Some(tb) = self.source_tb.take() {
                let _ = context.set_item("source_traceback", tb);
            }
            if let Some(ref exc) = self.exception {
                let _ = context.set_item("exception", exc.clone_ref(py));
            }
            let _ = self.evloop.as_ref(py).call_exception_handler(py, context);
        };
        py.release(self.result.take());
        py.release(self.exception.take());
        py.release(self.source_tb.take());
    }
}

impl future::Future for _PyFuture {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut rx) = self.receiver {
            match rx.poll() {
                Ok(Async::Ready(result)) => {
                    self.log_exc_tb.set(false);
                    Ok(Async::Ready(result))
                },
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => Err(err),
            }
        } else {
            Ok(Async::Ready(with_py(|py| self.get(py))))
        }
    }
}

#[py::class(freelist=500)]
pub struct PyFuture {
    fut: _PyFuture,
    blocking: bool,

    // reference to asyncio.Future if any
    pyfut: Option<PyObject>,

    token: PyToken,
}


#[py::methods]
impl PyFuture {

    fn __repr__(&self, py: Python) -> PyResult<PyObject> {
        let f: Py<PyFuture> = self.into();
        let repr = Classes.Helpers.as_ref(py).call(
            "future_repr", ("Future", f,), None)?;
        Ok(repr.into())
    }

    ///
    /// Cancel the future and schedule callbacks.
    ///
    /// If the future is already done or cancelled, return False.  Otherwise,
    /// change the future's state to cancelled, schedule the callbacks and
    /// return True.
    ///
    pub fn cancel(&mut self, py: Python) -> PyResult<bool> {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self.pyfut.take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "cancel", NoArgs, None);
            py.release(fut);
        }

        let ob = self.to_object(py);
        Ok(self.fut.cancel(py, ob))
    }

    //
    // Return True if the future was cancelled
    //
    pub fn cancelled(&self) -> PyResult<bool> {
        Ok(self.fut.cancelled())
    }

    // Return True if the future is done.
    //
    // Done means either that a result / exception are available, or that the
    // future was cancelled.
    //
    fn done(&self) -> PyResult<bool> {
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
    // asyncio.gather() uses protected attribute
    //
    #[getter(_result)]
    fn get_result(&self) -> PyResult<PyObject> {
        self.fut.get_result(self.token())
    }

    //
    // Return the exception that was set on this future.
    //
    // The exception (or None if no exception was set) is returned only if
    // the future is done.  If the future has been cancelled, raises
    // CancelledError.  If the future isn't done yet, raises
    // InvalidStateError.
    //
    fn exception(&self) -> PyResult<PyObject> {
        self.fut.exception(self.token())
    }

    //
    // asyncio.gather() uses protected attribute
    //
    #[getter(_exception)]
    fn get_exception(&self) -> PyResult<PyObject> {
        self.fut.get_exception(self.token())
    }

    //
    // Add a callback to be run when the future becomes done.
    //
    // The callback is called with a single argument - the future object. If
    // the future is already done when this is called, the callback is
    // scheduled with call_soon.
    //
    fn add_done_callback(&mut self, py: Python, f: PyObject) -> PyResult<PyObject> {
        let ob = self.to_object(py);
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

    ///
    /// Mark the future done and set its result.
    ///
    /// If the future is already done when this method is called, raises
    /// InvalidStateError.
    ///
    pub fn set_result(&mut self, py: Python, result: PyObject) -> PyResult<()> {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self.pyfut.take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "set_result", (result.clone_ref(py),), None);
            py.release(fut);
        }
        let ob = self.into();
        self.fut.set_result(py, result, ob, false)
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    fn set_exception(&mut self, py: Python, exception: PyObject) -> PyResult<()> {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self.pyfut.take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "set_exception", (exception.clone_ref(py),), None);
            py.release(fut);
        }
        let ob = self.to_object(py);
        self.fut.set_exception(py, exception.as_ref(py), ob, false)
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

    /// handler for asyncio.Future completion
    fn _fut_done(&mut self, py: Python, fut: PyObject) -> PyResult<()> {
        // drop reference to wrapped asyncio.Future
        // if it is None, then self initiated _pyfut completion
        if let None = self.pyfut.take() {
            return Ok(())
        }

        // check fut is cancelled
        if let Ok(cancelled) = fut.call_method(py, "cancelled", NoArgs, None) {
            if cancelled.is_true(py)? {
                let ob = self.to_object(py);
                let _ = self.fut.cancel(py, ob);
                return Ok(())
            }
        }

        // if fut completed with exception
        if let Ok(exc) = fut.call_method(py, "exception", NoArgs, None) {
            if !exc.is_none() {
                let ob = self.to_object(py);
                return self.fut.set_exception(py, exc.as_ref(py), ob, true)
            }
        }

        // if fut completed with normal result
        if let Ok(result) = fut.call_method(py, "result", NoArgs, None) {
            let ob = self.to_object(py);
            return self.fut.set_result(py, result, ob, true);
        }

        unreachable!();
    }

    // compatibility
    #[getter(_loop)]
    fn get_loop(&self) -> PyResult<Py<TokioEventLoop>> {
        Ok(self.fut.evloop.clone_ref(self.token()))
    }

    #[getter(_callbacks)]
    fn get_callbacks(&self) -> PyResult<PyObject> {
        if let Some(ref cb) = self.fut.callbacks {
            Ok(PyTuple::new(self.token(), cb.as_slice()).into_object(self.token()))
        } else {
            Ok(self.token().None())
        }
    }

    #[getter(_source_traceback)]
    fn get_source_traceback(&self) -> PyResult<PyObject> {
        self.fut.extract_traceback(self.token())
    }
}

/*#[py::proto]
impl PyGCProtocol for PyFuture {
    //
    // Python GC support
    //
    fn __traverse__(&self, _py: Python, visit: PyVisit) -> Result<(), PyTraverseError> {
        if let Some(ref callbacks) = self.fut.callbacks {
            for callback in callbacks.iter() {
                visit.call(callback)?;
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
}*/

#[py::proto]
impl PyAsyncProtocol for PyFuture {

    fn __await__(&self) -> PyResult<Py<PyFutureIter>> {
        let evloop = self.into();
        self.token().init(|t| PyFutureIter {fut: evloop, token: t})
    }
}

#[py::proto]
impl PyIterProtocol for PyFuture {

    fn __iter__(&mut self) -> PyResult<Py<PyFutureIter>> {
        let evloop = self.into();
        self.token().init(|t| PyFutureIter {fut: evloop, token: t})
    }
}

impl PyFuture {

    pub fn new(py: Python, evloop: Py<TokioEventLoop>) -> PyResult<Py<PyFuture>> {
        py.init(|t| PyFuture { fut: _PyFuture::new(py, evloop),
                               blocking: false,
                               pyfut: None,
                               token: t})
    }

    pub fn done_fut(py: Python, evloop: Py<TokioEventLoop>, result: PyObject)
                    -> PyResult<Py<PyFuture>>
    {
        py.init(|t| PyFuture { fut: _PyFuture::done_fut(py, evloop.clone_ref(py), result),
                               blocking: false,
                               pyfut: None,
                               token: t})
    }

    pub fn done_res(py: Python, evloop: Py<TokioEventLoop>, result: PyResult<PyObject>)
                    -> PyResult<Py<PyFuture>>
    {
        py.init(|t| PyFuture { fut: _PyFuture::done_res(py, evloop.clone_ref(py), result),
                               blocking: false,
                               pyfut: None,
                               token: t})
    }

    /// wrap asyncio.Future into PyFuture
    /// this method does not check if fut object is actually async.Future object
    pub fn from_fut(py: Python, evloop: Py<TokioEventLoop>, fut: &PyObjectRef)
                    -> PyResult<Py<PyFuture>>
    {
        let f = py.init(|t| PyFuture {
            fut: _PyFuture::new(py, evloop),
            blocking: false,
            pyfut: Some(fut.into()),
            token: t})?;

        // add done callback to fut
        let f_obj: PyObject = f.clone_ref(py).into();
        let meth = f_obj.getattr(py, "_fut_done")?;
        fut.call_method("add_done_callback", (meth,), None)?;

        Ok(f)
    }
    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        self.fut.get(py)
    }

    pub fn set(&mut self, py: Python, result: PyResult<PyObject>) {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self.pyfut.take() {
            // TODO: add logging for exceptions
            match result {
                Ok(ref res) => {
                    let _ = fut.call_method(py, "set_result", (res.clone_ref(py),), None);
                },
                Err(ref exc) => {
                    let _ = fut.call_method(
                        py, "set_exception", (exc.clone_ref(py).instance(py),), None);
                }
            }
        }

        let ob = self.to_object(py);
        self.fut.set(py, result, ob);
    }

    pub fn state(&self) -> State {
        self.fut.state()
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

    pub fn is_done(&self) -> bool {
        self.fut.done()
    }

    pub fn is_cancelled(&self) -> bool {
        self.fut.cancelled()
    }

}

#[derive(Debug)]
pub struct PyFut(Py<PyFuture>);

impl PyFut {
    #[inline]
    fn as_mut(&self) -> &mut PyFuture {
        return self.0.as_mut(GIL::python())
    }
}

impl std::convert::From<Py<PyFuture>> for PyFut {
    fn from(ob: Py<PyFuture>) -> Self {
        PyFut(ob)
    }
}

impl<'a> std::convert::From<&'a PyFuture> for PyFut {
    fn from(ob: &'a PyFuture) -> Self {
        PyFut(ob.into())
    }
}

impl future::Future for PyFut {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.as_mut().fut.poll()
    }
}

#[py::class]
pub struct PyFutureIter {
    fut: Py<PyFuture>,
    token: PyToken,
}


#[py::methods]
impl PyFutureIter {

    fn send(&mut self, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__()
    }

    fn throw(&mut self, tp: &PyObjectRef, val: Option<PyObject>,
             _tb: Option<PyObject>) -> PyResult<Option<PyObject>>
    {
        {
            let py = self.token();
            if Classes.Exception.as_ref(py).is_instance(tp)? {
                PyErr::from_instance(py, tp).restore(py);
            } else {
                if let Ok(tp) = PyType::downcast_from(tp) {
                    PyErr::new_lazy_init(tp, val).restore(py);
                } else {
                    PyErr::new::<exc::TypeError, _>(py, NoArgs).restore(py);
                }
            }
        }

        self.__next__()
    }
}

#[py::proto]
impl PyIterProtocol for PyFutureIter {

    fn __iter__(&mut self) -> PyResult<Py<PyFutureIter>> {
        Ok(self.into())
    }

    fn __next__(&mut self) -> PyResult<Option<PyObject>> {
        let py = self.token();
        let fut = self.fut.as_mut(py);
        if !fut.fut.done() {
            fut.blocking = true;
            Ok(Some(self.fut.to_object(py)))
        } else {
            let res = fut.result(py)?;
            Err(PyErr::new::<exc::StopIteration, _>(py, (res,)))
        }
    }
}
