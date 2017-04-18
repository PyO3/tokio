#![allow(unused_variables)]
#![allow(dead_code)]

use std::io;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use cpython::*;
use futures::sync::{oneshot};
use futures::{Async, Future, Poll};
use boxfnonce::SendBoxFnOnce;

use future::{create_task, done_future, TokioFuture};
use http::{self, pyreq};
use http::pyreq::PyRequest;
use utils::{Classes, PyLogger, ToPyErr, with_py};
use pyunsafe::{GIL, Handle, Sender};

pub enum PyHttpTransportMessage {
    Bytes(PyBytes),
    Close(Option<PyErr>),
}

const CONCURENCY_LEVEL: usize = 2;


py_class!(pub class PyHttpTransport |py| {
    data _loop: Handle;
    data _connection_lost: PyObject;
    data _data_received: PyObject;
    data _request_handler: PyObject;
    data transport: Sender<PyHttpTransportMessage>;
    data req: RefCell<Option<pyreq::PyRequest>>;
    data req_count: Cell<usize>;

    data inflight: Cell<usize>;
    data reqs: RefCell<VecDeque<http::Request>>;

    def get_extra_info(&self, _name: PyString,
                       default: Option<PyObject> = None ) -> PyResult<PyObject> {
        Ok(
            if let Some(ob) = default {
                ob
            } else {
                py.None()
            }
        )
    }

    //
    // write bytes to transport
    //
    def write(&self, data: PyBytes) -> PyResult<PyObject> {
        //println!("got bytes {:?}", data.clone_ref(py).into_object());
        //let bytes = Bytes::from(data.data(py));
        let _ = self.transport(py).send(PyHttpTransportMessage::Bytes(data));
        Ok(py.None())
    }

    //
    // send buffered data to socket
    //
    def drain(&self) -> PyResult<TokioFuture> {
        Ok(done_future(py, self._loop(py).clone(), py.None())?)
    }

    //
    // close transport
    //
    def close(&self) -> PyResult<PyObject> {
        let _ = self.transport(py).send(PyHttpTransportMessage::Close(None));
        Ok(py.None())
    }

});


impl PyHttpTransport {

    pub fn get_data_received(&self, py: Python) -> PyObject {
        self._data_received(py).clone_ref(py)
    }

    pub fn new(py: Python, h: Handle, sender: Sender<PyHttpTransportMessage>,
               factory: &PyObject) -> PyResult<PyHttpTransport> {
        // create protocol
        let proto = factory.call(py, NoArgs, None)
            .log_error(py, "Protocol factory error")?;

        // get protocol callbacks
        let connection_made = proto.getattr(py, "connection_made")?;
        let connection_lost = proto.getattr(py, "connection_lost")?;
        let data_received = proto.getattr(py, "data_received")?;
        let request_handler = proto.getattr(py, "handle_request")?;

        let transport = PyHttpTransport::create_instance(
            py, h, connection_lost, data_received, request_handler, sender,
            RefCell::new(None), Cell::new(0), Cell::new(0),
            RefCell::new(VecDeque::with_capacity(4)))?;

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
            self._connection_lost(py).call(py, PyTuple::new(py, &[py.None()]), None)
                .into_log(py, "connection_lost error");
        });
    }

    pub fn connection_error(&self, err: io::Error) {
        trace!("Protocol.connection_lost({:?})", err);
        with_py(|py| {
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

    pub fn data_received(&self, msg: http::RequestMessage) -> PyResult<()> {
        match msg {
            http::RequestMessage::Message(msg) => {
                let h = self._loop(GIL::python()).clone();
                let req = with_py(|py| {
                    if let Ok(req) = pyreq::PyRequest::new(py, msg, h) {
                        let _ = self._data_received(py).call(
                            py, PyTuple::new(
                                py, &[req.into_object()]), None);
                    }
                });
                return Ok(());

                let py = GIL::python();
                let count = self.req_count(py);
                count.set(count.get() + 1);

                let inflight = self.inflight(py);
                if inflight.get() < CONCURENCY_LEVEL {
                    println!("start task {}", inflight.get());
                    inflight.set(inflight.get() + 1);

                    // start handler task
                    let tx = self.transport(py).clone();
                    let handler = RequestHandler::new(
                        self._loop(py).clone(),
                        msg, self.clone_ref(py), self._request_handler(py).clone_ref(py))?;
                    self._loop(GIL::python()).spawn(handler.map_err(move |err| {
                        // close connection with error
                        let _ = tx.send(PyHttpTransportMessage::Close(Some(err)));
                    }));
                } else {
                    println!("wait");
                    self.reqs(py).borrow_mut().push_back(msg);
                }
            },
            http::RequestMessage::Body(chunk) => {

            },
            http::RequestMessage::Completed => {
                with_py(|py| {
                    let mut req = self.req(py).borrow_mut();
                    if let Some(ref req) = *req {
                        req.content().feed_eof(py);
                    }
                    *req = None;
                });
            }
        };
        Ok(())
    }
}


struct RequestHandler {
    h: Handle,
    tr: PyHttpTransport,
    handler: PyObject,
    task: Option<TokioFuture>,
    event: Option<oneshot::Receiver<PyResult<PyObject>>>,
    inflight: PyRequest,
}

impl RequestHandler {

    fn new(h: Handle, msg: http::Request,
           tr: PyHttpTransport, handler: PyObject) -> PyResult<RequestHandler> {

        let req = with_py(|py| pyreq::PyRequest::new(py, msg, h.clone()))?;

        Ok(RequestHandler {
            h: h,
            tr: tr,
            handler: handler,
            task: None,
            event: None,
            inflight: req,
        })
    }
}

impl Future for RequestHandler {
    type Item = ();
    type Error = PyErr;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let result = match self.event {
            None => {
                // first run
                let (tx, mut rx) = oneshot::channel::<PyResult<PyObject>>();

                // start python task
                let task = with_py(|py| {
                    let coro = match self.handler.call(
                        py, PyTuple::new(
                            py, &[self.inflight.clone_ref(py).into_object()]), None) {
                        Ok(coro) => coro,
                        Err(err) => return Err(err)
                    };

                    let task = try!(create_task(py, coro, self.h.clone()));
                    let _ = task.add_callback(py, SendBoxFnOnce::from(move |fut: TokioFuture| {
                        let res = fut.result(Python::acquire_gil().python());
                        let _ = tx.send(res);
                    }));
                    Ok(task)
                });

                match task {
                    Ok(task) => {
                        let res = rx.poll();
                        self.event = Some(rx);
                        self.task = Some(task);
                        res
                    },
                    Err(err) => return Err(err),
                }
            },
            Some(ref mut ev) => {
                ev.poll()
            }
        };

        // process result from python task
        match result {
            Ok(Async::Ready(res)) => {
                // select next message
                match self.tr.reqs(GIL::python()).borrow_mut().pop_front() {
                    Some(msg) => {
                        let res = with_py(|py| pyreq::PyRequest::new(py, msg, self.h.clone()));

                        match res {
                            Err(err) => return Err(err),
                            Ok(req) => {
                                // start new task
                                self.event = None;
                                self.inflight = req;
                            },
                        };
                    },
                    None => {
                        // nothing to process, decrease number of inflight tasks and exit
                        let inflight = self.tr.inflight(GIL::python());
                        inflight.set(inflight.get() - 1);

                        return Ok(Async::Ready(()))
                    }
                };
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
