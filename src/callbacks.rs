// Copyright (c) 2017-present PyO3 Project and Contributors

use std;
use std::collections::VecDeque;

use pyo3::Python;
use boxfnonce::SendBoxFnOnce;
use futures::{Async, Future, Poll, task};

pub type Callback = SendBoxFnOnce<()>;

pub struct Callbacks {
    callbacks: VecDeque<Callback>,
    callbacks2: Option<VecDeque<Callback>>,
    scheduled: bool,
    task: Option<task::Task>,
}

impl Callbacks {

    pub fn new() -> Callbacks {
        Callbacks{ callbacks: VecDeque::with_capacity(25),
                   callbacks2: Some(VecDeque::with_capacity(25)),
                   scheduled: true, task: None}
    }

    pub fn call_soon(&mut self, cb: Callback) {
        self.callbacks.push_back(cb);

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

        if !self.callbacks.is_empty() {
            let mut callbacks = std::mem::replace(
                &mut self.callbacks, self.callbacks2.take().unwrap());

            let _gil = Python::acquire_gil();
            loop {
                match callbacks.pop_front() {
                    Some(cb) => cb.call(),
                    None => break
                }
            }
            self.callbacks2 = Some(callbacks);
            if self.callbacks.len() < 5 {
                for _ in 0..5 {
                    if let Some(cb) = self.callbacks.pop_front() {
                        cb.call()
                    } else {
                        break
                    }
                }
            }
        }

        if !self.callbacks.is_empty() {
            if let Some(ref task) = self.task {
                task.notify();
            }
        } else {
            self.scheduled = false;
        }

        Ok(Async::NotReady)
    }
}
