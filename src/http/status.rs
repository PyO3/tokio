use std;
use bytes::Bytes;


/// Request http version
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Version {
    Http10,
    Http11,
}

/// Request status line
#[derive(PartialEq, Debug)]
pub struct RequestStatusLine {
    method: *const str,
    path: *const str,
    pub version: Version,
    bytes: Bytes,
}

pub fn req_status_line(src: Bytes, meth: (u8, u8),
                       path: (u8, u16), ver: Version) -> RequestStatusLine {
    RequestStatusLine {
        method: unsafe { std::str::from_utf8_unchecked(
            &src[(meth.0 as usize)..(meth.1 as usize)]) },
        path: unsafe { std::str::from_utf8_unchecked(
            &src[(path.0 as usize)..(path.1 as usize)]) },
        version: ver,
        bytes: src,
    }
}

impl RequestStatusLine {

    #[inline]
    pub fn method(&self) -> &str {
        unsafe { &*self.method }
    }

    #[inline]
    pub fn path(&self) -> &str {
        unsafe { &*self.path }
    }
}
