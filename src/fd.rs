use std::io;
use std::os::raw::c_int;
use std::os::unix::io::RawFd;
use mio::event::Evented;
use mio::unix::EventedFd;
use mio::{self, Ready, PollOpt, Token};
use futures::unsync::oneshot;
use futures::{stream, Async, Future, Poll};
use tokio_core::reactor::{Handle, PollEvented};

use fut::Until;
use handle::PyHandlePtr;


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
    reader: Option<PyHandlePtr>,
    writer: Option<PyHandlePtr>,
}

impl PyFdHandle {
    pub fn reader(fd: c_int, handle: &Handle, reader: PyHandlePtr)
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

    pub fn writer(fd: c_int, handle: &Handle, writer: PyHandlePtr)
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
                        reader.run();
                        self.ev.need_read();
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
                        writer.run();
                        self.ev.need_write();
                    }
                    poll = true;
                },
                Some(Async::NotReady) => {
                    poll = false;
                },
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


/// Stream of read readyness for file descriptor
pub struct PyFdReadable {
    io: Option<PollEvented<PyFd>>,
    marked_ready: bool,
    h: Handle,
}

impl PyFdReadable {
    pub fn new(fd: c_int, handle: &Handle) -> io::Result<PyFdReadable> {
        Ok(PyFdReadable{
            io: Some(PollEvented::new(PyFd::new(fd), handle)?),
            marked_ready: false,
            h: handle.clone(),
        })
    }
}

impl Drop for PyFdReadable {
    fn drop(&mut self) {
        if let Some(io) = self.io.take() {
            let _ = io.deregister(&self.h);
        }
    }
}

impl stream::Stream for PyFdReadable {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(ref io) = self.io {
            if self.marked_ready {
                io.need_read();
                self.marked_ready = false;
            }

            match io.poll_read() {
                Async::Ready(_) => {
                    self.marked_ready = true;
                    Ok(Async::Ready(Some(())))
                },
                Async::NotReady =>
                    Ok(Async::NotReady)
            }
        } else {
            Ok(Async::Ready(Some(())))
        }
    }
}

impl Until for PyFdReadable {}


/// Stream of write readyness for file descriptor
pub struct PyFdWritable {
    io: Option<PollEvented<PyFd>>,
    marked_ready: bool,
    h: Handle,
}

impl PyFdWritable {
    pub fn new(fd: c_int, handle: &Handle) -> io::Result<PyFdWritable> {
        Ok(PyFdWritable{
            io: Some(PollEvented::new(PyFd::new(fd), handle)?),
            marked_ready: false,
            h: handle.clone(),
        })
    }
}

impl Drop for PyFdWritable {
    fn drop(&mut self) {
        if let Some(io) = self.io.take() {
            let _ = io.deregister(&self.h);
        }
    }
}

impl stream::Stream for PyFdWritable {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(ref io) = self.io {
            if self.marked_ready {
                io.need_write();
                self.marked_ready = false;
            }

            match io.poll_write() {
                Async::Ready(_) => {
                    self.marked_ready = true;
                    Ok(Async::Ready(Some(())))
                },
                Async::NotReady =>
                    Ok(Async::NotReady)
            }
        } else {
            Ok(Async::Ready(Some(())))
        }
    }
}

impl Until for PyFdWritable {}
