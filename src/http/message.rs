use std;
use bytes::Bytes;

use http::headers::Headers;


/// Request http version
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Version {
    Http10,
    Http11,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    Close,
    KeepAlive,
    Upgrade,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentCompression {
    Default,
    Gzip,
    Deflate,
}

#[derive(Debug)]
pub struct Request {
    pub version: Version,
    pub headers: Headers,
    pub connection: ConnectionType,
    pub chunked: bool,
    pub websocket: bool,
    pub compress: ContentCompression,

    bytes: Bytes,
    meth: (u8, u8),
    path: (u8, u16),
}

impl Request {

    #[inline]
    pub fn method(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(
            &self.bytes[(self.meth.0 as usize)..(self.meth.1 as usize)]) }
    }

    #[inline]
    pub fn path(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(
            &self.bytes[(self.path.0 as usize)..(self.path.1 as usize)]) }
    }
}

pub trait RequestUpdater {

    fn new() -> Request;

    fn update_status(&mut self, src: Bytes, meth: (u8, u8), path: (u8, u16));
}

impl RequestUpdater for Request {

    fn new() -> Request {
        Request {
            version: Version::Http11,
            headers: Headers::new(),
            connection: ConnectionType::KeepAlive,
            chunked: false,
            websocket: false,
            compress: ContentCompression::Default,
            meth: (0, 0),
            path: (0, 0),
            bytes: Bytes::new(),
        }
    }

    fn update_status(&mut self, src: Bytes, meth: (u8, u8), path: (u8, u16)) {
        self.bytes = src;
        self.meth = meth;
        self.path = path;
    }
}
