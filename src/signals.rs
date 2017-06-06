use std::os::raw::c_int;
use std::collections::HashMap;

use futures::sync::mpsc;
use futures::{Async, Future, Poll, Stream};
use tokio_signal::unix::Signal;
use tokio_core::reactor::Handle;

use handle::PyHandlePtr;


pub enum SignalsMessage {
    Add(c_int, Signal, PyHandlePtr),
    Remove(c_int),
}


pub struct Signals {
    rx: mpsc::UnboundedReceiver<SignalsMessage>,
    signals: HashMap<c_int, (Signal, PyHandlePtr)>,
}

impl Signals {

    pub fn new(handle: &Handle) -> mpsc::UnboundedSender<SignalsMessage> {
        let (tx, rx) = mpsc::unbounded();

        handle.spawn(
            Signals {
                rx: rx,
                signals: HashMap::new(),
            });

        tx
    }

    fn add_signal_handler(&mut self, sig: c_int, signal: Signal, handler: PyHandlePtr) {
        self.signals.insert(sig, (signal, handler));
    }

    fn remove_signal_handler(&mut self, sig: c_int) {
        let _ = self.signals.remove(&sig);
    }
}


impl Future for Signals {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        match self.rx.poll() {
            Ok(Async::Ready(Some(msg))) => {
                match msg {
                    SignalsMessage::Add(sig, signal, handler) =>
                        self.add_signal_handler(sig, signal, handler),
                    SignalsMessage::Remove(sig) => self.remove_signal_handler(sig),
                }
                return self.poll()
            },
            Err(_) => return Err(()),
            _ => (),
        }

        let mut run = false;
        for &mut (ref mut sig, ref handler) in self.signals.values_mut() {
            let res = sig.poll();
            if let Ok(Async::Ready(Some(_))) = res {
                handler.run();
                run = true;
            }
        }

        if run {
            self.poll()
        } else {
            Ok(Async::NotReady)
        }
    }
}
