use std::io;
use std::os::raw::c_int;
use std::os::unix::io::RawFd;
use mio::event::Evented;
use mio::unix::EventedFd;
use mio::{self, Ready, PollOpt, Token};
use futures::unsync::oneshot;
use futures::{Async, Future, Poll};
use tokio_core::reactor::{Handle, PollEvented};

use handle::PyHandle;


pub struct PyFd (RawFd);

impl PyFd {
    pub fn new(fd: c_int) -> PyFd {
        PyFd (fd as RawFd)
    }
}

impl Evented for PyFd {
    fn register(&self, poll: &mio::Poll, token: Token,
                interest: Ready, opts: PollOpt) -> io::Result<()> {
        EventedFd(&self.0).register(poll, token, interest, opts)
    }

    fn reregister(&self, poll: &mio::Poll, token: Token,
                  interest: Ready, opts: PollOpt) -> io::Result<()> {
        EventedFd(&self.0).reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        EventedFd(&self.0).deregister(poll)
    }
}


pub struct PyFdHandle {
    ev: PollEvented<PyFd>,
    rx: oneshot::Receiver<()>,
    reader: Option<PyHandle>,
    writer: Option<PyHandle>,
}

impl PyFdHandle {
    pub fn reader(fd: c_int, handle: &Handle, reader: PyHandle)
                  -> io::Result<oneshot::Sender<()>> {
        let (tx, rx) = oneshot::channel();
        let ev = PollEvented::new(PyFd::new(fd), handle)?;

        handle.spawn(PyFdHandle {
            ev: ev,
            rx: rx,
            reader: Some(reader),
            writer: None,
        });

        Ok(tx)
    }

    pub fn writer(fd: c_int, handle: &Handle, writer: PyHandle)
                  -> io::Result<oneshot::Sender<()>> {
        let (tx, rx) = oneshot::channel();
        let ev = PollEvented::new(PyFd::new(fd), handle)?;

        handle.spawn(PyFdHandle {
            ev: ev,
            rx: rx,
            reader: None,
            writer: Some(writer),
        });

        Ok(tx)
    }
}

impl Future for PyFdHandle {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut poll = false;

        loop {
            // reader
            let result = if let Some(_) = self.reader {
                Some(self.ev.poll_read())
            } else {
                None
            };
            match result {
                Some(Async::Ready(_)) => {
                    if let Some(ref reader) = self.reader {
                        reader.run()
                    }
                    poll = true;
                },
                Some(Async::NotReady) => {
                    poll = false;
                },
                None => (),
            }

            // writer
            let result = if let Some(_) = self.writer {
                Some(self.ev.poll_write())
            } else {
                None
            };
            match result {
                Some(Async::Ready(_)) => {
                    if let Some(ref writer) = self.writer {
                        writer.run()
                    }
                    poll = true;
                },
                Some(Async::NotReady) => {
                    poll = false;
                },
                //Some(Err(_)) => {
                //    let _ = self.writer.take();
                //},
                None => (),
            }

            if let (&None, &None) = (&self.reader, &self.writer) {
                return Ok(Async::Ready(()))
            }

            match self.rx.poll() {
                Ok(Async::Ready(_)) | Err(_) => {
                    return Ok(Async::Ready(()))
                },
                _ => (),
            }

            if ! poll {
                return Ok(Async::NotReady)
            }
        }
    }
}

/*
"""
    def _add_reader(self, fd, callback, *args):
        self._check_closed()
        handle = events.Handle(callback, args, self)
        try:
            key = self._selector.get_key(fd)
        except KeyError:
            self._selector.register(fd, selectors.EVENT_READ,
                                    (handle, None))
        else:
            mask, (reader, writer) = key.events, key.data
            self._selector.modify(fd, mask | selectors.EVENT_READ,
                                  (handle, writer))
            if reader is not None:
                reader.cancel()

    def _remove_reader(self, fd):
        if self.is_closed():
            return False
        try:
            key = self._selector.get_key(fd)
        except KeyError:
            return False
        else:
            mask, (reader, writer) = key.events, key.data
            mask &= ~selectors.EVENT_READ
            if not mask:
                self._selector.unregister(fd)
            else:
                self._selector.modify(fd, mask, (None, writer))

            if reader is not None:
                reader.cancel()
                return True
            else:
                return False

"""
*/
