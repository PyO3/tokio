#![allow(unused_variables)]

use std::cell;
use std::mem;
use cpython::*;
use futures::{future, unsync, Async, Poll};
use futures::unsync::oneshot;
use boxfnonce::SendBoxFnOnce;

use ::TokioEventLoop;
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
    pub evloop: TokioEventLoop,
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

    pub fn new(ev: TokioEventLoop) -> _PyFuture {
        let tb = _PyFuture::extract_tb(&ev);
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

    pub fn done_fut(ev: TokioEventLoop, result: PyObject) -> _PyFuture {
        let tb = _PyFuture::extract_tb(&ev);

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

    pub fn done_res(py: Python, ev: TokioEventLoop, result: PyResult<PyObject>) -> _PyFuture {
        match result {
            Ok(result) => _PyFuture::done_fut(ev, result),
            Err(mut err) => {
                let tb = _PyFuture::extract_tb(&ev);

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

    fn extract_tb(ev: &TokioEventLoop) -> Option<PyObject> {
        if ev.is_debug() {
            match Classes.ExtractStack.call(GIL::python(), NoArgs, None) {
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
            _ => false
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
                Err(PyErr::new_err(py, &Classes.InvalidStateError, ("Result is not ready.",))),
            State::Cancelled =>
                Err(PyErr::new_err(py, &Classes.CancelledError, NoArgs)),
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
                Err(PyErr::new_err(
                    py, &Classes.InvalidStateError, "Exception is not set.")),
            State::Cancelled =>
                Err(PyErr::new_err(py, &Classes.CancelledError, NoArgs)),
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
                self.evloop.href().spawn_fn(move || with_py(|py| {
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
                Err(PyErr::new_err(py, &Classes.InvalidStateError, ("Result is not ready.",))),
            State::Cancelled =>
                Err(PyErr::new_err(py, &Classes.CancelledError, NoArgs)),
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
                      result: PyObject, sender: PyObject, inplace: bool) -> PyResult<PyObject> {
        match self.state {
            State::Pending => {
                // set result
                self.result = Some(result);

                self.schedule_callbacks(py, State::Finished, sender, inplace);
                Ok(py.None())
            },
            _ => Err(PyErr::new_err(py, &Classes.InvalidStateError, NoArgs)),
        }
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    pub fn set_exception(&mut self, py: Python, exception: PyObject,
                         sender: PyObject, inplace: bool) -> PyResult<PyObject> {
        match self.state {
            State::Pending => {
                // check if exception is a type object
                let exc =
                    if let Ok(exception) = PyType::downcast_borrow_from(py, &exception) {
                        Some(exception.call(py, NoArgs, None)?)
                    } else {
                        None
                    };
                let exc = if let Some(exc) = exc { exc } else { exception };

                // StopIteration cannot be raised into a Future - CPython issue26221
                if Classes.StopIteration.is_instance(py, &exc) {
                    return Err(PyErr::new::<exc::TypeError, _>(
                        py, "StopIteration interacts badly with generators \
                             and cannot be raised into a Future"));
                }

                self.exception = Some(exc);
                self.log_exc_tb.set(true);

                self.schedule_callbacks(py, State::Finished, sender, inplace);
                Ok(py.None())
            }
            _ => Err(PyErr::new_err(py, &Classes.InvalidStateError, NoArgs)),
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
                              state: State, owner: PyObject, inplace: bool) {
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
            self.evloop.href().spawn_fn(|| send_rresults());
        }

        // schedule python callbacks
        match self.callbacks.take() {
            Some(callbacks) => {
                // call task callback
                let send_callbacks = move|| {
                    with_py(move |py| {
                        // call python callback
                        for cb in callbacks.iter() {
                            cb.call(py, (owner.clone_ref(py),), None)
                                .into_log(py, "future done callback error");
                        }
                    });
                    future::ok(())
                };

                if inplace {
                    let _ = send_callbacks();
                } else {
                    self.evloop.href().spawn_fn(|| send_callbacks());
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
        if self.log_exc_tb.get() {
            let _: PyResult<()> = with_py(|py| {
                let context = PyDict::new(py);
                context.set_item(py, "message", "Future exception was never retrieved")?;
                context.set_item(py, "future", "PyFuture")?;
                if let Some(tb) = self.source_tb.take() {
                    context.set_item(py, "source_traceback", tb)?;
                }
                if let Some(ref exc) = self.exception {
                    context.set_item(py, "exception", exc.clone_ref(py))?;
                }
                self.evloop.call_exception_handler(py, context)?;
                Ok(())
            });
        }
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
            Err(unsync::oneshot::Canceled)
        }
    }
}

py_class!(pub class PyFuture |py| {
    data _fut: cell::RefCell<_PyFuture>;
    data _blocking: cell::Cell<bool>;

    // reference to asyncio.Future if any
    data _pyfut: cell::RefCell<Option<PyObject>>;

    def __repr__(&self) -> PyResult<PyString> {
        let repr = Classes.Helpers.call(py, "future_repr", ("Future", &self,), None)?;
        Ok(PyString::downcast_from(py, repr)?)
    }

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    def cancel(&self) -> PyResult<bool> {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self._pyfut(py).borrow_mut().take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "cancel", NoArgs, None);
        }

        Ok(self._fut(py).borrow_mut().cancel(py, self.clone_ref(py).into_object()))
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
        self._fut(py).borrow().result(py, true)
    }

    //
    // asyncio.gather() uses protected attribute
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
    // asyncio.gather() uses protected attribute
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
        // handle wrapped asyncio.Future object
        if let Some(fut) = self._pyfut(py).borrow_mut().take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "set_result", (result.clone_ref(py),), None);
        }
        self._fut(py).borrow_mut().set_result(
            py, result, self.clone_ref(py).into_object(), false)
    }

    //
    // Mark the future done and set an exception.
    //
    // If the future is already done when this method is called, raises
    // InvalidStateError.
    //
    def set_exception(&self, exception: PyObject) -> PyResult<PyObject> {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self._pyfut(py).borrow_mut().take() {
            // TODO: add logging for exceptions
            let _ = fut.call_method(py, "set_exception", (exception.clone_ref(py),), None);
        }
        self._fut(py).borrow_mut().set_exception(
            py, exception, self.clone_ref(py).into_object(), false)
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
    // isfuture support
    //
    property _asyncio_future_blocking {
        get(&slf) -> PyResult<bool> {
            Ok(slf._blocking(py).get())
        }
        set(&slf, value: bool) -> PyResult<()> {
            slf._blocking(py).set(value);
            Ok(())
        }
    }

    //
    // Python GC support
    //
    def __traverse__(&self, visit) {
        if let Some(ref callbacks) = self._fut(py).borrow().callbacks {
            for callback in callbacks.iter() {
                visit.call(callback)?;
            }
        }
        Ok(())
    }

    def __clear__(&self) {
        let callbacks = mem::replace(&mut (*self._fut(py).borrow_mut()).callbacks, None);
        if let Some(callbacks) = callbacks {
            for cb in callbacks {
                cb.release_ref(py);
            }
        }
    }

    // handler for asyncio.Future completion
    def _fut_done(&self, fut: PyObject) -> PyResult<PyObject> {
        // drop reference to wrapped asyncio.Future
        // if it is None, then self initiated _pyfut completion
        if let None = self._pyfut(py).borrow_mut().take() {
            return Ok(py.None())
        }

        // check fut is cancelled
        if let Ok(cancelled) = fut.call_method(py, "cancelled", NoArgs, None) {
            if cancelled.is_true(py)? {
                let _ = self._fut(py).borrow_mut().cancel(
                    py, self.clone_ref(py).into_object());
                return Ok(py.None())
            }
        }

        // if fut completed with exception
        if let Ok(exc) = fut.call_method(py, "exception", NoArgs, None) {
            if exc != py.None() {
                let _ = self._fut(py).borrow_mut().set_exception(
                    py, exc, self.clone_ref(py).into_object(), true);
                return Ok(py.None())
            }
        }

        // if fut completed with normal result
        if let Ok(result) = fut.call_method(py, "result", NoArgs, None) {
            let _ = self._fut(py).borrow_mut().set_result(
                py, result, self.clone_ref(py).into_object(), true);
            return Ok(py.None())
        }

        unreachable!();
    }

    // compatibility
    property _loop {
        get(&slf) -> PyResult<TokioEventLoop> {
            Ok(slf._fut(py).borrow().evloop.clone_ref(py))
        }
    }

    property _callbacks {
        get(&slf) -> PyResult<PyObject> {
            if let Some(ref cb) = slf._fut(py).borrow().callbacks {
                Ok(PyTuple::new(py, cb.as_slice()).into_object())
            } else {
                Ok(py.None())
            }
        }
    }

    property _source_traceback {
        get(&slf) -> PyResult<PyObject> {
            slf._fut(py).borrow().extract_traceback(py)
        }
    }
});

impl PyFuture {

    pub fn new(py: Python, evloop: &TokioEventLoop) -> PyResult<PyFuture> {
        PyFuture::create_instance(
            py,
            cell::RefCell::new(_PyFuture::new(evloop.clone_ref(py))),
            cell::Cell::new(false),
            cell::RefCell::new(None))
    }

    pub fn done_fut(py: Python, evloop: &TokioEventLoop, result: PyObject) -> PyResult<PyFuture> {
        PyFuture::create_instance(
            py,
            cell::RefCell::new(_PyFuture::done_fut(evloop.clone_ref(py), result)),
            cell::Cell::new(false),
            cell::RefCell::new(None))
    }

    pub fn done_res(py: Python, evloop: &TokioEventLoop, result: PyResult<PyObject>) -> PyResult<PyFuture> {
        PyFuture::create_instance(
            py,
            cell::RefCell::new(_PyFuture::done_res(py, evloop.clone_ref(py), result)),
            cell::Cell::new(false),
            cell::RefCell::new(None))
    }

    /// wrap asyncio.Future into PyFuture
    /// this method does not check if fut object is actually async.Future object
    pub fn from_fut(py: Python, evloop: &TokioEventLoop, fut: PyObject) -> PyResult<PyFuture> {
        let f = PyFuture::create_instance(
            py,
            cell::RefCell::new(_PyFuture::new(evloop.clone_ref(py))),
            cell::Cell::new(false),
            cell::RefCell::new(Some(fut.clone_ref(py))))?;

        // add done callback to fut
        let f_obj = f.clone_ref(py).into_object();
        let meth = f_obj.getattr(py, "_fut_done")?;
        fut.call_method(py, "add_done_callback", (meth,), None)?;

        Ok(f)
    }

    pub fn get(&self, py: Python) -> PyResult<PyObject> {
        self._fut(py).borrow().get(py)
    }

    pub fn set(&self, py: Python, result: PyResult<PyObject>) -> bool {
        // handle wrapped asyncio.Future object
        if let Some(fut) = self._pyfut(py).borrow_mut().take() {
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

        self._fut(py).borrow_mut().set(py, result, self.clone_ref(py).into_object())
    }

    pub fn state(&self, py: Python) -> State {
        self._fut(py).borrow().state()
    }

    //
    // Add future completion callback
    //
    pub fn add_callback(&self, py: Python, cb: Callback) {
        self._fut(py).borrow_mut().add_callback(py, cb);
    }

    //
    // bloking
    //
    pub fn is_blocking(&self) -> bool {
        self._blocking(GIL::python()).get()
    }

    pub fn set_blocking(&self, value: bool) {
        self._blocking(GIL::python()).set(value)
    }

    //
    // helpers methods
    //
    pub fn is_same_loop(&self, py: Python, evloop: &TokioEventLoop) -> bool {
        &self._fut(py).borrow().evloop == evloop
    }

    pub fn is_done(&self, py: Python) -> bool {
        self._fut(py).borrow().done()
    }

    pub fn is_cancelled(&self, py: Python) -> bool {
        self._fut(py).borrow().cancelled()
    }
}

impl future::Future for PyFuture {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self._fut(GIL::python()).borrow_mut().poll()
    }
}


py_class!(pub class PyFutureIter |py| {
    data _fut: PyFuture;

    def __iter__(&self) -> PyResult<PyFutureIter> {
        Ok(self.clone_ref(py))
    }

    def __next__(&self) -> PyResult<Option<PyObject>> {
        let fut = self._fut(py);

        if !fut._fut(py).borrow().done() {
            fut._blocking(py).set(true);
            Ok(Some(fut.clone_ref(py).into_object()))
        } else {
            let res = fut.result(py)?;
            Err(PyErr::new::<exc::StopIteration, _>(py, (res,)))
        }
    }

    def send(&self, _unused: PyObject) -> PyResult<Option<PyObject>> {
        self.__next__(py)
    }

    def throw(&self, tp: PyObject, val: Option<PyObject> = None,
              _tb: Option<PyObject> = None) -> PyResult<Option<PyObject>> {

        if Classes.Exception.is_instance(py, &tp) {
            PyErr::from_instance(py, tp).restore(py);
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
