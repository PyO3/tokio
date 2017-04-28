#![allow(dead_code)]
#![allow(unused_variables)]

use std;
use std::ops::Range;
use std::hash::Hasher;
use std::ascii::AsciiExt;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use bytes::{Bytes, BytesMut};


#[derive(Debug)]
pub struct Headers {
    headers: HashMap<u64, Header>,
    bytes: Option<Bytes>,
    last_pos: u16,
}

impl Headers {

    pub fn new() -> Headers {
        Headers { headers: HashMap::with_capacity(64),
                  bytes: None,
                  last_pos: 0,
        }
    }

    pub fn headers(&self) -> Vec<(String, String)> {
        let mut vec = Vec::new();

        if let Some(ref bytes) = self.bytes {
            for header in self.headers.values() {
                vec.push((
                    String::from(
                        unsafe {
                            std::str::from_utf8_unchecked(&bytes[header.name_range()])
                        }
                    ),

                    String::from(
                        unsafe {
                            std::str::from_utf8_unchecked(&bytes[header.value_range()])
                        }
                    )));
            }
        }
        vec
    }
    
    pub fn get(&self, name: &str) -> Option<&str> {
        let mut hasher = DefaultHasher::new();
        for byte in name.bytes().map(|b| b.to_ascii_lowercase()) {
            hasher.write_u8(byte);
        }
        let hash = hasher.finish();

        if let Some(ref header) = self.headers.get(&hash) {
            if let Some(ref bytes) = self.bytes {
                Some(
                    unsafe {
                        std::str::from_utf8_unchecked(&bytes[header.value_range()])
                    })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_case(&self, name: &str) -> Option<&str> {
        let mut hasher = DefaultHasher::new(); //self.hasher.borrow_mut();
        for byte in name.bytes() {
            hasher.write_u8(byte);
        }
        let hash = hasher.finish();

        if let Some(ref header) = self.headers.get(&hash) {
            if let Some(ref bytes) = self.bytes {
                Some(
                    unsafe {
                        std::str::from_utf8_unchecked(&bytes[header.value_range()])
                    })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn has(&self) -> bool {
        true
    }

}

pub trait WriteHeaders {

    fn append(&mut self, header: Header);

    fn flush(&mut self, src: &mut BytesMut);

}

impl WriteHeaders for Headers {

    fn append(&mut self, header: Header) {
        self.last_pos = header.end();
        self.headers.insert(header.hash, header);
    }

    fn flush(&mut self, src: &mut BytesMut) {
        let end = self.last_pos + 4; // 2: header does not include CRLF
        self.bytes = Some(src.split_to(end as usize).freeze());
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Header {
    hash: u64,
    name_pos: u16,
    name_len: u16,
    value_pos: u16,
    value_len: u16,
}

impl Header {

    pub fn new() -> Header {
        Header {
            hash: 0,
            name_pos: 0,
            name_len: 0,
            value_pos: 0,
            value_len: 0,
        }
    }

    #[inline]
    pub fn set_hash(&mut self, hash: u64) {
        self.hash = hash
    }

    #[inline]
    pub fn set_name_pos(&mut self, pos: usize) {
        self.name_pos = pos as u16;
        self.name_len = 0;
    }

    #[inline]
    pub fn update_name_len(&mut self, cnt: usize) {
        self.name_len += cnt as u16
    }

    #[inline]
    pub fn set_value_pos(&mut self, pos: usize) {
        self.value_pos = pos as u16;
        self.value_len = 0;
    }

    #[inline]
    pub fn update_value_len(&mut self, cnt: usize) {
        self.value_len += cnt as u16
    }

    #[inline]
    pub fn end(&self) -> u16 {
        self.value_pos + self.value_len
    }

    #[inline]
    pub fn is_overflow(&self, max_size: u16) -> bool {
        (self.name_len + self.value_len) >= max_size
    }

    #[inline]
    pub fn name_range(&self) -> Range<usize> {
        Range{ start: self.name_pos as usize, end: (self.name_pos + self.name_len) as usize }
    }

    #[inline]
    pub fn value_range(&self) -> Range<usize> {
        Range{ start: self.value_pos as usize, end: (self.value_pos + self.value_len) as usize }
    }
}

/*pub struct HeadersIter<'h> {
    pos: usize,
    keys: Vec<u64>,
    headers: &'h Headers,
}

impl <'h> HeadersIter<'h> {

    fn new(headers: &'h Headers) -> HeadersIter<'h> {
        HeadersIter {
            pos: 0,
            keys: Vec::from(headers.headers.keys()),
            headers: headers,
        }
    }
}

impl<'h> Iterator for HeadersIter <'h> {
    type Item = (&'h str, &'h str);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.headers.get_header(self.pos) {
            self.idx += 1;

            if let Some(inner) = self.headers.bufs.get(self.idx) {
                self.curr = Some(inner.iter());
                return self.next();
            } else {
                return None
            }
        }
    }
}*/
