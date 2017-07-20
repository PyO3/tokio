// Copyright (c) 2017-present PyO3 Project and Contributors

use std;
use pyo3::Python;
use boxfnonce::SendBoxFnOnce;
use futures::{Async, Future, Poll, task};

pub type Callback = SendBoxFnOnce<()>;

pub struct Callbacks {
    callbacks: Vec<Callback>,
    callbacks2: Option<Vec<Callback>>,
    scheduled: bool,
    task: Option<task::Task>,
}

impl Callbacks {

    pub fn new() -> Callbacks {
        Callbacks{ callbacks: Vec::with_capacity(100),
                   callbacks2: Some(Vec::with_capacity(100)),
                   scheduled: false, task: None}
    }

    pub fn call_soon(&mut self, cb: Callback) {
        self.callbacks.push(cb);

        if !self.scheduled {
            self.scheduled = true;
            if let Some(ref task) = self.task {
                task.notify();
            }
        }
    }
}

impl Future for Callbacks {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.task.is_none() {
            self.task = Some(task::current());
        }

        if self.scheduled {
            self.scheduled = false;

            if !self.callbacks.is_empty() {
                let mut callbacks = std::mem::replace(
                    &mut self.callbacks, self.callbacks2.take().unwrap());

                let _gil = Python::acquire_gil();
                loop {
                    match callbacks.pop() {
                        Some(cb) => cb.call(),
                        None => break
                    }
                }

                unsafe {callbacks.set_len(0)};
                self.callbacks2 = Some(callbacks);
            }
        }

        if self.scheduled {
            if let Some(ref task) = self.task {
                task.notify();
            }
        }

        Ok(Async::NotReady)
    }
}
