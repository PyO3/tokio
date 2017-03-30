// UNSAFE code!
use std::ops::Deref;
use std::clone::Clone;
use tokio_core::reactor;
use futures::unsync::{mpsc, oneshot};
use cpython::Python;


#[doc(hidden)]
pub struct GIL;

unsafe impl Send for GIL {}

impl GIL {

    /// Retrieves the marker type that proves that the GIL was acquired.
    #[inline]
    pub fn python<'p>() -> Python<'p> {
        unsafe { Python::assume_gil_acquired() }
    }

}


// tokio handle
#[doc(hidden)]
pub struct Handle {
    pub h: reactor::Handle,
}

unsafe impl Send for Handle {}

impl Handle {
    pub fn new(h: reactor::Handle) -> Handle {
        Handle{h: h}
    }
}

impl Clone for Handle {

    fn clone(&self) -> Handle {
        Handle {h: self.h.clone()}
    }
}

impl Deref for Handle {
    type Target = reactor::Handle;

    fn deref(&self) -> &reactor::Handle {
        &self.h
    }
}


#[doc(hidden)]
pub struct Sender<T> {
    pub s: mpsc::UnboundedSender<T>,
}

unsafe impl<T> Send for Sender<T> {}

impl<T> Sender<T> {

    pub fn new(sender: mpsc::UnboundedSender<T>) -> Self {
        Sender{s: sender}
    }

    pub fn send(&self, msg: T) -> Result<(), mpsc::SendError<T>> {
        self.s.send(msg)
    }

}

impl<T> Clone for Sender<T> {

    fn clone(&self) -> Self {
        Sender{s: self.s.clone()}
    }
}


#[doc(hidden)]
pub struct OneshotSender<T> (oneshot::Sender<T>);

unsafe impl<T> Send for OneshotSender<T> {}

impl<T> OneshotSender<T> {

    pub fn new(sender: oneshot::Sender<T>) -> Self {
        OneshotSender(sender)
    }

    pub fn send(self, msg: T) -> Result<(), T> {
        self.0.send(msg)
    }

}
