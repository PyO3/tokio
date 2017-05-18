// UNSAFE code!
use std::ops::Deref;
use std::clone::Clone;
use tokio_core::reactor;
use futures::{Future, Poll};
use futures::unsync::{mpsc, oneshot};
use pyo3::Python;


#[doc(hidden)]
pub struct GIL;

unsafe impl Send for GIL {}

impl GIL {

    /// Retrieves the marker type that proves that the GIL was acquired.
    /// This method does not acquire GIL, you have to be sure that your
    /// interfere with real python gil
    #[inline]
    pub fn python<'p>() -> Python<'p> {
        unsafe { Python::assume_gil_acquired() }
    }

}


// tokio handle
#[doc(hidden)]
pub struct Core (pub reactor::Core);

unsafe impl Send for Core {}

impl Core {
    pub fn new(core: reactor::Core) -> Core {
        Core(core)
    }
    pub fn into(self) -> reactor::Core {
        self.0
    }
}

impl Deref for Core {
    type Target = reactor::Core;

    fn deref(&self) -> &reactor::Core {
        &self.0
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
    pub fn into(self) -> reactor::Handle {
        self.h
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
pub struct Sender<T> (mpsc::UnboundedSender<T>);

unsafe impl<T> Send for Sender<T> {}

impl<T> Clone for Sender<T> {

    fn clone(&self) -> Self {
        Sender(self.0.clone())
    }
}

impl<T> Sender<T> {

    pub fn new(sender: mpsc::UnboundedSender<T>) -> Self {
        Sender(sender)
    }

    pub fn send(&self, msg: T) -> Result<(), mpsc::SendError<T>> {
        self.0.send(msg)
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


#[doc(hidden)]
pub struct OneshotReceiver<T> (oneshot::Receiver<T>);

unsafe impl<T> Send for OneshotReceiver<T> {}

impl<T> OneshotReceiver<T> {

    pub fn new(receiver: oneshot::Receiver<T>) -> Self {
        OneshotReceiver(receiver)
    }

}

impl<T> Future for OneshotReceiver<T> {
    type Item = T;
    type Error = oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}
