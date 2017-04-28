#![allow(unused_variables)]

use std::cell;
use cpython::*;
use futures::{future, unsync, Poll};
use futures::unsync::oneshot;
// use futures::task::{Task, park};
use boxfnonce::SendBoxFnOnce;

use utils::{Classes, PyLogger, with_py};
use pyunsafe::{GIL, Handle};

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum State {
    Pending,
    Cancelled,
    Finished,
}

pub type Callback = SendBoxFnOnce<(PyResult<PyObject>,)>;

pub struct _PyFuture {
    h: Handle,
    sender: Option<oneshot::Sender<PyResult<PyObject>>>,
    receiver: Option<oneshot::Receiver<PyResult<PyObject>>>,
    state: State,
    result: Option<PyObject>,
    exception: Option<PyObject>,
    pub callbacks: Option<Vec<PyObject>>,

    // rust callbacks
    rcallbacks: Option<Vec<Callback>>,
}

unsafe impl Send for _PyFuture {}

impl _PyFuture {

    pub fn new(h: Handle) -> _PyFuture {
        let (tx, rx) = unsync::oneshot::channel();

        _PyFuture {
            h: h,
            sender: Some(tx),
            receiver: Some(rx),
            state: State::Pending,
            result: None,
            exception: None,
            callbacks: None,
            rcallbacks: None,
        }
    }

    pub fn done_fut(h: Handle, result: PyObject) -> _PyFuture {
        _PyFuture {
            h: h,
            sender: None,
            receiver: None,
            state: State::Finished,
            result: Some(result),
            exception: None,
            callbacks: None,
            rcallbacks: None,
        }
    }

    pub fn done_res(py: Python, h: Handle, result: PyResult<PyObject>) -> _PyFuture {
        match result {
            Ok(result) => _PyFuture::done_fut(h, result),
            Err(mut err) =>
                _PyFuture {
                    h: h,
                    sender: None,
                    receiver: None,
                    state: State::Finished,
                    result: None,
                    exception: Some(err.instance(py)),
                    callbacks: None,
                    rcallbacks: None,
                }
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
                self.schedule_callbacks(py, State::Cancelled, sender);
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
    pub fn result(&self, py: Python) -> PyResult<PyObject> {
        match self.state {
            State::Pending =>
                Err(PyErr::new_lazy_init(
                    Classes.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Result is not ready.").into_object()))),
            State::Cancelled =>
                Err(PyErr::new_lazy_init(Classes.CancelledError.clone_ref(py), None)),
            State::Finished => {
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
                Err(PyErr::new_lazy_init(
                    Classes.InvalidStateError.clone_ref(py),
                    Some(PyString::new(py, "Exception is not set.").into_object()))),
            State::Cancelled =>
                Err(PyErr::new_lazy_init(
                    Classes.CancelledError.clone_ref(py), None)),
            State::Finished =>
                match self.exception {
                    Some(ref err) => Ok(err.clone_ref(py)),
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
                f.call(py, (owner,).to_py_object(py), None)
                    .into_log(py, "future callback error");
            },
        }

        Ok(py.None())
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
                    Err(mut err) =>
                        self.exception = Some(err.instance(py))
                }
                self.schedule_callbacks(py, State::Finished, sender);

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
                      result: PyObject, sender: PyObject) -> PyResult<PyObject> {
        match self.state {
            State::Pending => {
                // set result
                self.result = Some(result);

                self.schedule_callbacks(py, State::Finished, sender);
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
    pub fn set_exception(&mut self, py: Python,
                         exception: PyObject, sender: PyObject) -> PyResult<PyObject> {
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

                // if type(exception) is StopIteration:
                //    raise TypeError("StopIteration interacts badly with generators "
                //                    "and cannot be raised into a Future")
                self.exception = Some(exc);

                self.schedule_callbacks(py, State::Finished, sender);
                Ok(py.None())
            }
            _ => Err(PyErr::new_lazy_init(Classes.InvalidStateError.clone_ref(py), None)),
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
                cb.call(self.result(py));
            },
        }
    }

    //
    //
    pub fn schedule_callbacks(&mut self, py: Python, state: State, owner: PyObject) {
        self.state = state;

        // complete oneshot channel
        if let Some(sender) = self.sender.take() {
            if state != State::Cancelled {
                let _ = sender.send(self.result(py));
            }
        }

        // schedule rust callbacks
        let result = self.result(py);
        let mut rcallbacks = self.rcallbacks.take();

        self.h.spawn_fn(move|| {
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
        });

        // schedule python callbacks
        match self.callbacks.take() {
            Some(callbacks) => {
                // call task callback
                let result = self.result(py);

                self.h.spawn_fn(move|| {
                    with_py(move |py| {
                        // call python callback
                        for cb in callbacks.iter() {
                            cb.call(py, (owner.clone_ref(py),).to_py_object(py), None)
                                .into_log(py, "future done callback error");
                        }

                    });
                    future::ok(())
                });
            },
            _ => (),
        }
    }
}

impl future::Future for _PyFuture {
    type Item = PyResult<PyObject>;
    type Error = unsync::oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut rx) = self.receiver {
            rx.poll()
        } else {
            Err(unsync::oneshot::Canceled)
        }
    }
}


py_class!(pub class PyFuture |py| {
    data _fut: cell::RefCell<_PyFuture>;

    //
    // Cancel the future and schedule callbacks.
    //
    // If the future is already done or cancelled, return False.  Otherwise,
    // change the future's state to cancelled, schedule the callbacks and
    // return True.
    //
    def cancel(&self) -> PyResult<bool> {
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
        self._fut(py).borrow().result(py)
    }

    //
    // some ugly api called incapsulaton
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
    // some ugly api called incapsulaton
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
        get(&slf) -> PyResult<PyBool> {
            Ok(py.False())
        }
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

});

impl PyFuture {

    pub fn new(py: Python, h: Handle) -> PyResult<PyFuture> {
        PyFuture::create_instance(py, cell::RefCell::new(_PyFuture::new(h)))
    }

    pub fn done_fut(py: Python, h: Handle, result: PyObject) -> PyResult<PyFuture> {
        PyFuture::create_instance(py, cell::RefCell::new(_PyFuture::done_fut(h, result)))
    }

    pub fn done_res(py: Python, h: Handle, result: PyResult<PyObject>) -> PyResult<PyFuture> {
        PyFuture::create_instance(
            py, cell::RefCell::new(_PyFuture::done_res(py, h, result)))
    }

    pub fn set(&self, py: Python, result: PyResult<PyObject>) -> bool {
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
