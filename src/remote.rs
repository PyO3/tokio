#![allow(unused_variables)]

use std::thread;
use std::sync::mpsc;
use cpython::*;
use futures::future;
use tokio_core::reactor;

use addrinfo;
use handle;
use server;
use http;
use transport;
use ::{PyFuture, PyTask};
use pyunsafe::{GIL, Handle};
use event_loop::{TokioEventLoop, new_event_loop};


pub fn spawn_event_loop(py: Python, name: &PyString) -> PyResult<RemoteTokioEventLoop> {
    let (tx, rx) = mpsc::channel();

    // start worker thread
    let _ = thread::Builder::new().name(String::from(name.to_string_lossy(py))).spawn(
        move || {
            let gil = Python::acquire_gil();
            let py = gil.python();

            if let Ok(evloop) = new_event_loop(py) {
                let _ = tx.send((evloop.clone_ref(py), evloop.remote(py)));
                let _ = evloop.run_forever(py, false);
            }
        }
    );

    // temp gil release
    let result = py.allow_threads(move|| {
        if let Ok(res) = rx.recv() { Some(res) } else { None }
    });

    match result {
        Some((evloop, handle)) =>
            RemoteTokioEventLoop::create_instance(py, evloop, handle),
        None => Err(PyErr::new::<exc::RuntimeError, _>(
            py, "Can not start tokio evernt loop".to_py_object(py)))
    }
}


py_class!(pub class RemoteTokioEventLoop |py| {
    data evloop: TokioEventLoop;
    data handle: reactor::Remote;

    def create_future(&self) -> PyResult<PyFuture> {
        let res = self.execute_in_loop(py, move|py, h| {
            PyFuture::new(py, h)
        });
        match res {
            Some(Ok(srv)) => Ok(srv),
            _ => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Can not create tcp server")),
        }
    }

    def create_task(&self, coro: PyObject) -> PyResult<PyTask> {
        let res = self.execute_in_loop(py, move|py, h| {
            PyTask::new(py, coro, None, h)
        });
        match res {
            Some(Ok(srv)) => Ok(srv),
            _ => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Can not create tcp server")),
        }
    }

    //
    // Return the time according to the event loop's clock.
    //
    // This is a float expressed in seconds since event loop creation.
    //
    def time(&self) -> PyResult<f64> {
        self.evloop(py).time(py)
    }

    //
    // Return the time according to the event loop's clock (milliseconds)
    //
    def millis(&self) -> PyResult<u64> {
        self.evloop(py).millis(py)
    }

    //
    // Schedule callback to call later
    //
    def call_later(&self, *args, **_kwargs) -> PyResult<handle::TokioTimerHandle> {
        let args = args.clone_ref(py);
        let remote = self.handle(py);

        let handle = py.allow_threads(move|| {
            let py = GIL::python();
            let (tx, rx) = mpsc::channel();
            let evloop = self.evloop(py).clone_ref(py);

            remote.spawn(move |_| {
                let res = evloop.call_later(GIL::python(), &args, None);
                let _ = tx.send(res);
                future::ok(())
            });

            match rx.recv() {
                Ok(Ok(handle)) => Some(handle),
                _ => None,
            }
        });

        match handle {
            Some(handle) => Ok(handle),
            None => Err(
                PyErr::new::<exc::RuntimeError, _>(
                    py, "Can not connect to remote event loop")),
        }
    }


    //
    // Schedule callback at specific time
    //
    def call_at(&self, *args, **kwargs) -> PyResult<handle::TokioTimerHandle> {
        let args = args.clone_ref(py);
        let remote = self.handle(py);

        let handle = py.allow_threads(move|| {
            let py = GIL::python();
            let (tx, rx) = mpsc::channel();
            let evloop = self.evloop(py).clone_ref(py);

            remote.spawn(move |_| {
                let res = evloop.call_at(GIL::python(), &args, None);
                let _ = tx.send(res);
                future::ok(())
            });

            match rx.recv() {
                Ok(Ok(handle)) => Some(handle),
                _ => None,
            }
        });

        match handle {
            Some(handle) => Ok(handle),
            None => Err(
                PyErr::new::<exc::RuntimeError, _>(
                    py, "Can not connect to remote event loop")),
        }
    }

    //
    // Stop running the event loop (it is safe to call TokioEventLoop.stop())
    //
    def stop(&self) -> PyResult<PyBool> {
        self.evloop(py).stop(py)
    }

    //
    // It is not possible to close remote event loop.
    //
    def close(&self) -> PyResult<PyObject> {
        Ok(py.None())
    }


    //
    // Create a TCP server.
    //
    // The host parameter can be a string, in that case the TCP server is bound
    // to host and port.
    //
    // The host parameter can also be a sequence of strings and in that case
    // the TCP server is bound to all hosts of the sequence. If a host
    // appears multiple times (possibly indirectly e.g. when hostnames
    // resolve to the same IP address), the server is only bound once to that
    // host.
    //
    // Return a Server object which can be used to stop the service.
    //
    def create_server(&self, protocol_factory: PyObject,
                      host: Option<PyString>, port: Option<u16> = None,
                      family: i32 = 0,
                      flags: i32 = addrinfo::AI_PASSIVE,
                      sock: Option<PyObject> = None,
                      backlog: i32 = 100,
                      ssl: Option<PyObject> = None,
                      reuse_address: bool = true,
                      reuse_port: bool = true) -> PyResult<PyFuture> {

        if let Some(_) = ssl {
            return Err(
                PyErr::new::<exc::TypeError, _>(
                    py, "ssl argument is not supported yet"));
        }

        let res = self.execute_in_loop(py, move|py, h| {
            server::create_server(
                py, protocol_factory, h,
                Some(String::from(host.unwrap().to_string_lossy(py))), Some(port.unwrap_or(0)),
                family, flags, sock, backlog, ssl, reuse_address, reuse_port,
                transport::tcp_transport_factory)
        });

        match res {
            Some(Ok(srv)) => Ok(srv),
            _ => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Can not create tcp server")),
        }
    }

    //
    // Create a HTTP server.
    //
    // The host parameter can be a string, in that case the TCP server is bound
    // to host and port.
    //
    // The host parameter can also be a sequence of strings and in that case
    // the TCP server is bound to all hosts of the sequence. If a host
    // appears multiple times (possibly indirectly e.g. when hostnames
    // resolve to the same IP address), the server is only bound once to that
    // host.
    //
    // Return a Server object which can be used to stop the service.
    //
    def create_http_server(&self, protocol_factory: PyObject,
                           host: Option<PyString>, port: Option<u16> = None,
                           family: i32 = 0,
                           flags: i32 = addrinfo::AI_PASSIVE,
                           sock: Option<PyObject> = None,
                           backlog: i32 = 100,
                           ssl: Option<PyObject> = None,
                           reuse_address: bool = true,
                           reuse_port: bool = true) -> PyResult<PyFuture> {

        if let Some(_) = ssl {
            return Err(
                PyErr::new::<exc::TypeError, _>(
                    py, "ssl argument is not supported yet"));
        }

        let res = self.execute_in_loop(py, move|py, h| {
            server::create_server(
                py, protocol_factory, h,
                Some(String::from(host.unwrap().to_string_lossy(py))), Some(port.unwrap_or(0)),
                family, flags, sock, backlog, ssl, reuse_address, reuse_port,
                http::http_transport_factory)
        });

        match res {
            Some(Ok(srv)) => Ok(srv),
            _ => Err(
                PyErr::new::<exc::RuntimeError, _>(py, "Can not create tcp server")),
        }
    }

});


impl RemoteTokioEventLoop {

    pub fn remote(&self, py: Python) -> reactor::Remote {
        self.handle(py).clone()
    }

    pub fn execute_in_loop<T, F>(&self, py: Python, f: F) -> Option<T>
        where T : Send + 'static, F : FnOnce(Python, Handle) -> T + Send + 'static
    {
        let remote = self.handle(py);

        py.allow_threads(move|| {
            let (tx, rx) = mpsc::channel();

            remote.spawn(move |h| {
                let gil = Python::acquire_gil();
                let py = gil.python();

                let result = f(py, Handle::new(h.clone()));
                let _ = tx.send(result);
                future::ok(())
            });

            match rx.recv() {
                Ok(handle) => Some(handle),
                _ => None,
            }
        })
    }

}
