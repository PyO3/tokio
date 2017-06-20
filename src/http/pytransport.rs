#![allow(unused_variables)]
#![allow(dead_code)]

use std::io;
use std::collections::VecDeque;

use pyo3::*;
use futures::unsync::mpsc;
use futures::{Async, Future, Poll};

use {TokioEventLoop, PyFuture, PyFuturePtr, PyTask, PyTaskPtr, pybytes};
use http::{self, pyreq, codec};
use http::pyreq::{PyRequest, PyRequestPtr, StreamReader, StreamReaderPtr};
use utils::{Classes, PyLogger, with_py};
use pyunsafe::{GIL, Sender};


pub enum PyHttpTransportMessage {
    Close(Option<PyErr>),
}

const CONCURENCY_LEVEL: usize = 1;

#[py::class]
pub struct PyHttpTransport {
    _loop: TokioEventLoopPtr,
    _connection_lost: PyObject,
    _data_received: PyObject,
    _request_handler: PyObject,
    _socket: PyObject,
    transport: Sender<PyHttpTransportMessage>,
    req: Option<pyreq::PyRequestPtr>,
    req_count: usize,

    inflight: usize,
    reqs: VecDeque<(http::Request, Sender<codec::EncoderMessage>)>,
    payloads: VecDeque<StreamReaderPtr>,

    token: PyToken,
}


#[py::methods]
impl PyHttpTransport {

    fn get_extra_info(&self, py: Python, _name: PyString,
                       default: Option<PyObject>) -> PyResult<PyObject> {
        Ok(self._socket(py).clone_ref(py))
    }

    //
    // write bytes to transport
    //
    fn write(&self, py: Python, data: PyBytes) -> PyResult<PyObject> {
        Err(PyErr::new::<exc::RuntimeError, _>(
            py, "write() method is not available, use PayloadWriter"))
    }

    //
    // send buffered data to socket
    //
    fn drain(&self, py: Python) -> PyResult<PyFuture> {
        Ok(PyFuture::done_fut(py, self._loop(py), py.None())?)
    }

    //
    // close transport
    //
    fn close(&self, py: Python) -> PyResult<PyObject> {
        let _ = self.transport(py).send(PyHttpTransportMessage::Close(None));
        Ok(py.None())
    }
}


impl PyHttpTransport {

    pub fn new(py: Python, evloop: &TokioEventLoop,
               sender: Sender<PyHttpTransportMessage>,
               proto: &PyObject, sock: PyObject) -> PyResult<PyHttpTransport> {
        // get protocol callbacks
        let connection_made = proto.getattr(py, "connection_made")?;
        let connection_lost = proto.getattr(py, "connection_lost")?;
        let data_received = proto.getattr(py, "data_received")?;
        let request_handler = proto.getattr(py, "handle_request")?;
        //let request_handler = proto.getattr(py, "_request_handler")?;

        let transport = PyHttpTransport::create_instance(
            py, evloop.clone_ref(py),
            connection_lost, data_received, request_handler, sock,
            sender,
            None, 0, 0,
            VecDeque::with_capacity(12),
            VecDeque::with_capacity(CONCURENCY_LEVEL))?;

        // connection made
        connection_made.call(
            py, PyTuple::new(
                py, &[transport.clone_ref(py).into_object()]), None)
            .log_error(py, "Protocol.connection_made error")?;

        Ok(transport)
    }

    pub fn connection_lost(&self) {
        trace!("Protocol.connection_lost(None)");
        with_py(|py| {
            self.reqs_mut(py).clear();

            self._connection_lost(py).call(py, PyTuple::new(py, &[py.None()]), None)
                .into_log(py, "connection_lost error");
        });
    }

    pub fn connection_error(&self, err: io::Error) {
        trace!("Protocol.connection_lost({:?})", err);
        with_py(|py| {
            self.reqs_mut(py).clear();

            match err.kind() {
                io::ErrorKind::TimedOut => {
                    trace!("socket.timeout");
                    with_py(|py| {
                        let e = Classes.SocketTimeout.call(
                            py, NoArgs, None).unwrap();

                        self._connection_lost(py).call(py, PyTuple::new(py, &[e]), None)
                            .into_log(py, "connection_lost error");
                    });
                },
                _ => {
                    trace!("Protocol.connection_lost(err): {:?}", err);
                    with_py(|py| {
                        let mut e = err.to_pyerr(py);
                        self._connection_lost(py).call(py,
                                                       PyTuple::new(py, &[e.instance(py)]), None)
                            .into_log(py, "connection_lost error");
                    });
                }
            }
        });
    }

    pub fn data_received(&self, msg: http::RequestMessage)
                         -> PyResult<Option<mpsc::UnboundedReceiver<codec::EncoderMessage>>> {
        match msg {
            http::RequestMessage::Message(msg) => {
                let (sender, recv) = mpsc::unbounded();

                with_py(|py| match pyreq::PyRequest::new(
                    py, msg, self._loop(py).clone(), Sender::new(sender)) {
                    Err(err) => {
                        error!("{:?}", err);
                        err.clone_ref(py).print(py);
                    },
                    Ok(req) => {
                        self.payloads_mut(py).push_back(req.content().clone_ref(py));
                        self._data_received(py).call(
                            py, PyTuple::new(py, &[req.into_object()]), None)
                            .into_log(py, "data_received error");
                    }
                });
                return Ok(Some(recv));

                /*let py = GIL::python();
                let count = self.req_count(py);
                count.set(count.get() + 1);

                let inflight = self.inflight(py);
                if inflight.get() < CONCURENCY_LEVEL {
                    inflight.set(inflight.get() + 1);

                    // start handler task
                    let tx = self.transport(py).clone();
                    let handler = RequestHandler::new(
                        self._loop(py).clone(), msg, Sender::new(sender),
                        self.clone_ref(py), self._request_handler(py).clone_ref(py))?;

                    self._loop(GIL::python()).spawn(handler.map_err(move |err| {
                        // close connection with error
                        let _ = tx.send(PyHttpTransportMessage::Close(Some(err)));
                    }));
                } else {
                    //println!("wait");
                    self.reqs(py).borrow_mut().push_back((msg, Sender::new(sender)));
                }
                return Ok(Some(recv));*/
            },
            http::RequestMessage::Body(chunk) => {
                with_py(|py| {
                    if let Some(payload) = self.payloads_mut(py).pop_front() {
                        match pybytes::PyBytes::new(py, chunk) {
                            Ok(bytes) => payload.feed_data(py, bytes),
                            Err(err) =>  {
                                // close connection with error
                                let _ = self.transport(py).send(
                                    PyHttpTransportMessage::Close(Some(err)));
                            }
                        }
                    }
                });
            },
            http::RequestMessage::Completed => {
                with_py(|py| {
                    if let Some(payload) = self.payloads_mut(py).pop_front() {
                        payload.feed_eof(py);
                    }
                });
            }
        };
        Ok(None)
    }
}


struct RequestHandler {
    evloop: TokioEventLoopPtr,
    tr: PyHttpTransportPtr,
    handler: PyObject,
    task: PyTaskPtr,
    inflight: PyRequestPtr,
}

impl RequestHandler {

    fn new(evloop: TokioEventLoopPtr, msg: http::Request, tx: Sender<codec::EncoderMessage>,
           tr: PyHttpTransportPtr, handler: PyObject) -> PyResult<RequestHandler> {

        let (task, req) = RequestHandler::start_task(&evloop, msg, tx, &handler)?;

        Ok(RequestHandler {
            evloop: evloop,
            tr: tr,
            handler: handler,
            task: task,
            inflight: req,
        })
    }

    pub fn start_task(evloop: &TokioEventLoopPtr, msg: http::Request,
                      sender: Sender<codec::EncoderMessage>,
                      handler: &PyObject) -> PyResult<(PyTaskPtr, PyRequestPtr)> {
        // start python task
        with_py(|py| {
            let req = pyreq::PyRequest::new(py, msg, &evloop, sender)?;
            req.content().feed_eof(py);

            let coro = handler.call(
                py, PyTuple::new(py, &[req.clone_ref(py).into_object()]), None)?;

            let task = PyTask::new(py, coro, evloop)?;
            Ok((task, req))
        })
    }
}


impl Future for RequestHandler {
    type Item = ();
    type Error = PyErr;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let result = self.task.poll();

        // process result from python task
        match result {
            Ok(Async::Ready(res)) => {
                // select next message
                //if self.tr.reqs(GIL::python()).borrow_mut().len() > 0 {
                //    println!("num: {}", self.tr.reqs(GIL::python()).borrow_mut().len());
                //}
                let tr = self.tr.as_mut(GIL::python());
                let (msg, sender) = match tr.reqs.pop_front() {
                    Some((msg, sender)) => (msg, sender),
                    None => {
                        // nothing to process, decrease number of inflight tasks and exit
                        tr.inflight = tr.inflight - 1;

                        //println!("no requests in queue");
                        return Ok(Async::Ready(()))
                    }
                };
                let (task, req) = RequestHandler::start_task(
                    &self.evloop, msg, sender, &self.handler)?;
                self.inflight = req;
                self.task = task;
                self.poll()
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => {
                // close connection with error
                Ok(Async::Ready(()))
            }
        }
    }
}
