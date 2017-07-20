// Copyright (c) 2017-present PyO3 Project and Contributors

use boxfnonce::SendBoxFnOnce;

pub type Callback = SendBoxFnOnce<()>;

pub struct Callbacks {
    callbacks: Vec<Callback>,
}

impl Callbacks {

    pub fn new() -> Callbacks {
        Callbacks{callbacks: Vec::new()}
    }
    
    pub fn call_soon(&mut self, cb: Callback) {
        self.callbacks.push(cb);
    }

}
