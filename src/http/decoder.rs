#![allow(dead_code)]

use std;
use std::hash::Hasher;
use std::ascii::AsciiExt;
use std::error::Error as StdError;
use std::collections::hash_map::DefaultHasher;
use bytes::{Bytes, BytesMut};
use tokio_io::codec::Decoder;

use http::headers::{Header, WriteHeaders};
use http::message::{Version, ContentCompression, ConnectionType, Request, RequestUpdater};


/// Parsed request
#[derive(Debug)]
pub enum RequestMessage {
    Message(Request),
    Body(Bytes),
    Completed,
}

/// An error in parsing.
#[derive(Debug)]
pub enum Error {
    /// Invalid byte in header.
    BadHeader,
    /// Line is too long.
    LineTooLong,
    /// Bad status line
    BadStatusLine,
    /// Invalid content-length header
    ContentLength,
    /// Content-Length and Trasnfer-Encoding: chunked
    ContentLengthAndTE,
    /// An error in parsing a chunk
    TransferEncoding,
    /// Eof received but payload is not completed yet
    PayloadNotCompleted,
    /// std::io::Error
    IOError(std::io::Error),
}

impl Error {
    #[inline]
    fn description_str(&self) -> &'static str {
        match *self {
            Error::BadHeader => "bad header",
            Error::LineTooLong => "line too long",
            Error::BadStatusLine => "bad status line",
            Error::ContentLength => "invalid content length",
            Error::ContentLengthAndTE => "Both defined Content-Length and Trasnfer-Encoding: chunked length",
            Error::TransferEncoding => "transfer encoding error",
            Error::PayloadNotCompleted => "Eof received but payload is not completed yet",
            Error::IOError(_) => "io error",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(self.description_str())
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        self.description_str()
    }
}

/// Convert Error to io::Error
impl std::convert::From<Error> for std::io::Error {
    fn from(err: Error) -> Self {
        std::io::Error::new(
            std::io::ErrorKind::Other, format!("Python exception: {:?}", err.description()))
    }
}

/// Convert to std::io::Error to Error
impl std::convert::From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::IOError(err)
    }
}

/// A Result of any parsing action.
///
/// If the input is invalid, an `Error` will be returned. Note that incomplete
/// data is not considered invalid, and so will not return an error, but rather
/// a `Ok(Status::Partial)`.
type Result<T, P> = std::result::Result<Status<T, P>, Error>;

/// The result of a successful parse pass.
///
/// `Complete` is used when the buffer contained the complete value.
/// `Partial` is used when parsing did not reach the end of the expected value,
/// but no invalid data was found.
#[derive(Copy, Clone, PartialEq, Debug)]
enum Status<T, P> {
    /// The completed result.
    Complete(T),
    /// A partial result.
    Partial(P)
}

#[derive(Copy, Clone, Debug)]
enum CRLF {
    CR,
    LF,
}

impl CRLF {
    #[inline]
    fn val(&self) -> u8 {
        match *self {
            CRLF::CR => CR,
            CRLF::LF => LF,
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum ParseHeader {
    Eol,
    Name,
    OWS,
    Value,
    ValueEol,
    ContentLength,
}

#[derive(Copy, Clone, Debug)]
enum ParseStatusLine {
    Skip(CRLF),
    Method,
    Path,
    Version,
    Eol(CRLF),
}

macro_rules! match_hname {
    ($enu:ident::$hdr:ident($idx:ident) == $ch:ident, $token:ident) => ({
        let next = $idx + 1;
        if next == $token.len {
            $enu::General
        } else if $token.token[next] != $ch {
            $enu::General
        } else {
            $enu::$hdr(next)
        }
    })
}

macro_rules! match_token {
    ($enu:ident::$hdr:ident($idx:ident) == $ch:ident, $token:ident) => ({
        let next = $idx + 1;
        if next == $token.len {
            $enu::General
        } else if $token.token[next] != $ch.to_ascii_lowercase() {
            $enu::General
        } else {
            $enu::$hdr(next)
        }
    })
}


#[derive(Copy, Clone, PartialEq, Debug)]
enum ParseHeaderName {
    New,
    General,

    Con(usize),
    Content(usize),

    Connection(usize),
    ContentLength(usize),
    ContentEncoding(usize),

    ProxyConnection(usize),
    TransferEncoding(usize),
    Upgrade(usize),
}


impl ParseHeaderName {

    #[inline]
    fn next(&self, ch: u8) -> ParseHeaderName {
        match *self {
            ParseHeaderName::General => ParseHeaderName::General,

            ParseHeaderName::Con(idx) => {
                match idx + 1 {
                    1 if ch == b'o' => ParseHeaderName::Con(1),
                    2 if ch == b'n' => ParseHeaderName::Con(2),
                    3 if ch == b'n' => ParseHeaderName::Connection(3),
                    3 if ch == b't' => ParseHeaderName::Content(3),
                    _ => ParseHeaderName::General
                }
            },
            ParseHeaderName::Content(idx) => {
                let next = idx + 1;
                if next == 8 {
                    match ch {
                        b'e' => ParseHeaderName::ContentEncoding(next),
                        b'l' => ParseHeaderName::ContentLength(next),
                        _    => ParseHeaderName::General
                    }
                } else if CONTENT.token[next] != ch {
                    ParseHeaderName::General
                } else {
                    ParseHeaderName::Content(next)
                }
            },
            ParseHeaderName::Connection(idx) => {
                match_hname!(ParseHeaderName::Connection(idx) == ch, CONNECTION)
            },
            ParseHeaderName::ContentLength(idx) => {
                match_hname!(ParseHeaderName::ContentLength(idx) == ch, CONTENT_LENGTH)
            },
            ParseHeaderName::ContentEncoding(idx) => {
                match_hname!(ParseHeaderName::ContentEncoding(idx) == ch, CONTENT_ENCODING)
            },
            ParseHeaderName::ProxyConnection(idx) => {
                match_hname!(ParseHeaderName::ProxyConnection(idx) == ch, PROXY_CONNECTION)
            },
            ParseHeaderName::TransferEncoding(idx) => {
                match_hname!(ParseHeaderName::TransferEncoding(idx) == ch, TRANSFER_ENCODING)
            },
            ParseHeaderName::Upgrade(idx) => {
                match_hname!(ParseHeaderName::Upgrade(idx) == ch, UPGRADE)
            },
            ParseHeaderName::New => {
                match ch {
                    b'c' => ParseHeaderName::Con(0),
                    b'p' => ParseHeaderName::ProxyConnection(0),
                    b't' => ParseHeaderName::TransferEncoding(0),
                    b'u' => ParseHeaderName::Upgrade(0),
                    _    => ParseHeaderName::General,
                }
            },
        }
    }

    #[inline]
    fn completed(&self) -> bool {
        match *self {
            ParseHeaderName::Connection(idx) => idx+1 == CONNECTION.len,
            ParseHeaderName::ContentLength(idx) => idx+1 == CONTENT_LENGTH.len,
            ParseHeaderName::ContentEncoding(idx) => idx+1 == CONTENT_ENCODING.len,
            ParseHeaderName::ProxyConnection(idx) => idx+1 == PROXY_CONNECTION.len,
            ParseHeaderName::TransferEncoding(idx) => idx+1 == TRANSFER_ENCODING.len,
            ParseHeaderName::Upgrade(idx) => idx+1 == UPGRADE.len,
            _ => true
        }
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum ParseTokens {
    New,
    General,
    C,
    Close(usize),
    Chunked(usize),
    Gzip(usize),
    Deflate(usize),
    KeepAlive(usize),
    Websocket(usize),
    Upgrade(usize),
}

impl ParseTokens {

    #[inline]
    fn next(&self, ch: u8) -> ParseTokens {
        match *self {
            ParseTokens::General => ParseTokens::General,
            ParseTokens::C => match ch.to_ascii_lowercase() {
                b'h' => ParseTokens::Chunked(1),
                b'l' => ParseTokens::Close(1),
                _    => ParseTokens::General,
            },
            ParseTokens::Chunked(idx) => {
                match_token!(ParseTokens::Chunked(idx) == ch, CHUNKED)
            },
            ParseTokens::Close(idx) => {
                match_token!(ParseTokens::Close(idx) == ch, CLOSE)
            },
            ParseTokens::Gzip(idx) => {
                match_token!(ParseTokens::Gzip(idx) == ch, GZIP)
            },
            ParseTokens::Deflate(idx) => {
                match_token!(ParseTokens::Deflate(idx) == ch, DEFLATE)
            },
            ParseTokens::KeepAlive(idx) => {
                match_token!(ParseTokens::KeepAlive(idx) == ch, KEEP_ALIVE)
            },
            ParseTokens::Websocket(idx) => {
                match_token!(ParseTokens::Websocket(idx) == ch, WEBSOCKET)
            },
            ParseTokens::Upgrade(idx) => {
                match_token!(ParseTokens::Upgrade(idx) == ch, UPGRADE)
            },
            ParseTokens::New => match ch.to_ascii_lowercase() {
                b'c' => ParseTokens::C,
                b'g' => ParseTokens::Gzip(0),
                b'd' => ParseTokens::Deflate(0),
                b'k' => ParseTokens::KeepAlive(0),
                b'w' => ParseTokens::Websocket(0),
                b'u' => ParseTokens::Upgrade(0),
                _    => ParseTokens::General,
            },
        }
    }

    //#[inline]
    fn completed(&self) -> bool {
        match *self {
            ParseTokens::Chunked(idx) => idx+1 == CHUNKED.len,
            ParseTokens::Close(idx) => idx+1 == CLOSE.len,
            ParseTokens::Gzip(idx) => idx+1 == GZIP.len,
            ParseTokens::Deflate(idx) => idx+1 == DEFLATE.len,
            ParseTokens::KeepAlive(idx) => idx+1 == KEEP_ALIVE.len,
            ParseTokens::Websocket(idx) => idx+1 == WEBSOCKET.len,
            ParseTokens::Upgrade(idx) => idx+1 == UPGRADE.len,
            _ => false
        }
    }

}

#[derive(Copy, Clone, Debug)]
enum ParseBody {
    ChunkSize(usize),
    ChunkSizeEol(u64),
    Chunk(u64),
    ChunkEOL(CRLF),
    ChunkMaybeTrailers,
    ChunkTrailers(CRLF),
    Length(u64),
    Unsized,
}


#[derive(Copy, Clone, Debug)]
enum State {
    Status(ParseStatusLine),
    Header(ParseHeader),
    Body(ParseBody),
    Done,
}

pub struct RequestDecoder {
    state: State,
    start: usize,
    meth_pos: u8,
    meth_end: u8,
    path_end: u16,
    path_pos: u8,

    request: Request,

    length: Option<u64>,
    chunked: bool,

    header: Header,
    has_header: bool,
    header_token: ParseTokens,
    header_name: ParseHeaderName,
    header_name_hash: DefaultHasher,

    max_line_size: u16,
    max_headers: u16,
    max_field_size: u16,
}

impl RequestDecoder {

    pub fn new() -> RequestDecoder {
        RequestDecoder {
            start: 0, state: State::Status(ParseStatusLine::Skip(CRLF::CR)),
            meth_pos: 0, meth_end: 0, path_pos: 0, path_end: 0,

            request: Request::new(),

            header: Header::new(), has_header: false, header_token: ParseTokens::New,
            header_name: ParseHeaderName::General, header_name_hash: DefaultHasher::new(),

            length: None, chunked: false,

            max_line_size: 8190, max_headers: 32768, max_field_size: 8190,
        }
    }

    fn update_msg_state(&mut self, token: ParseTokens) {
        match self.header_name {
            ParseHeaderName::Connection(..) =>
                if self.request.connection != ConnectionType::Upgrade {
                    match token {
                        ParseTokens::Close(..) =>
                            self.request.connection = ConnectionType::Close,
                        ParseTokens::KeepAlive(..) =>
                            self.request.connection = ConnectionType::KeepAlive,
                        ParseTokens::Upgrade(..) =>
                            self.request.connection = ConnectionType::Upgrade,
                        _ => (),
                    }
                },
            ParseHeaderName::TransferEncoding(..) => match token {
                ParseTokens::Chunked(..) => {
                    self.chunked = true;
                    self.request.chunked = true;
                },
                _ => (),
            },
            ParseHeaderName::ContentEncoding(..) => match token {
                ParseTokens::Gzip(..) =>
                    self.request.compress = ContentCompression::Gzip,
                ParseTokens::Deflate(..) =>
                    self.request.compress = ContentCompression::Deflate,
                _ => (),
            },
            ParseHeaderName::Upgrade(..) => match token {
                ParseTokens::Websocket(..) =>
                    self.request.websocket = true,
                _ => (),
            },
            _ => (),
        }
    }
}

impl Decoder for RequestDecoder {
    type Item = RequestMessage;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        let mut state = self.state;
        let mut bytes = BytesPtr::new(src.as_ref(), self.start);
        let mut header_name = self.header_name;
        let mut header_token = self.header_token;

        'run: loop {
            // println!("Start from: {:?}", state);
            match state {

            State::Status(status) => match status {
                // post requests may add extract CRLF after payload
                ParseStatusLine::Skip(marker) => {
                    if let Some(ch) = bytes.get_maybe() {
                        match marker {
                            CRLF::CR => match ch {
                                CR => {
                                    bytes.bump();
                                    state = State::Status(ParseStatusLine::Skip(CRLF::LF));
                                    continue
                                },
                                LF => {
                                    bytes.bump();
                                    state = State::Status(ParseStatusLine::Method);
                                }
                                _ => {
                                    state = State::Status(ParseStatusLine::Method);
                                }
                            },
                            CRLF::LF => {
                                if ch == LF {
                                    bytes.bump();
                                }
                                state = State::Status(ParseStatusLine::Method);
                            }
                        }
                        self.meth_pos = bytes.pos() as u8;
                        self.meth_end = self.meth_pos;
                    } else {
                        break
                    }
                },
                ParseStatusLine::Method => match parse_token(&mut bytes, SP)? {
                    Status::Complete(l) => {
                        if bytes.pos() > self.max_line_size as usize {
                            return Err(Error::LineTooLong);
                        }
                        self.meth_end = self.meth_end + l as u8;
                        self.path_pos = bytes.pos() as u8;
                        self.path_end = self.path_pos as u16;
                        state = State::Status(ParseStatusLine::Path);
                    }
                    Status::Partial(l) => {
                        if bytes.pos() > self.max_line_size as usize {
                            return Err(Error::LineTooLong);
                        }
                        self.meth_end = self.meth_end + l as u8;
                        break
                    }
                },
                ParseStatusLine::Path => match parse_path(&mut bytes)? {
                    Status::Complete(l) => {
                        if bytes.pos() > self.max_line_size as usize  {
                            return Err(Error::LineTooLong);
                        }
                       self.path_end = self.path_end + l as u16;
                        state = State::Status(ParseStatusLine::Version);
                    }
                    Status::Partial(l) => {
                        if bytes.pos() > self.max_line_size as usize  {
                            return Err(Error::LineTooLong);
                        }
                        self.path_end = self.path_end + l as u16;
                        break
                    }
                },
                ParseStatusLine::Version => match parse_version(&mut bytes)? {
                    Status::Complete(ver) => {
                        self.request.version = ver;
                        if ver == Version::Http10 {
                            self.request.connection = ConnectionType::Close;
                        }
                        self.start = 0;
                        self.request.update_status(
                            src.split_to(bytes.pos()).freeze(),
                            (self.meth_pos, self.meth_end),
                            (self.path_pos, self.path_end));
                        bytes = BytesPtr::new(src.as_ref(), 0);
                        state = State::Status(ParseStatusLine::Eol(CRLF::CR));
                    },
                    Status::Partial(..) => {
                        break
                    }
                },
                ParseStatusLine::Eol(marker) =>
                    match parse_crlf(&mut bytes, marker, Error::BadStatusLine)? {
                        Status::Complete(..) => {
                            self.length = None;
                            self.chunked = false;
                            state = State::Header(ParseHeader::Eol);
                        },
                        Status::Partial(marker) => {
                            state = State::Status(ParseStatusLine::Eol(marker));
                            break
                        },
                    }
            },
            State::Header(marker) => match marker {
                // eol of headers, possible scenarios
                // CRLF right avter status line
                // hedaer line with CRLF and then CRLF (end of headers)
                // hedaer line with CRLF and then SP (continuation)
                // token (new header line)
                ParseHeader::Eol => match bytes.get_maybe() {
                    Some(ch) =>
                        // reading end http message
                        if ch == CR {
                            bytes.bump();
                            if let Some(ch) = bytes.next_maybe() {
                                if ch == LF {
                                    if self.has_header {
                                        self.request.headers.append(self.header);
                                    }
                                    self.request.headers.flush(src);

                                    let length = match self.length{
                                        Some(length) =>
                                            if self.chunked {
                                                return Err(Error::ContentLengthAndTE);
                                            } else {
                                                length
                                            },
                                        None => 0,
                                    };

                                    self.start = 0;
                                    if self.chunked {
                                        self.state = State::Body(ParseBody::ChunkSize(0));
                                    } else if length > 0 {
                                        self.state = State::Body(ParseBody::Length(length));
                                    } else {
                                        self.state = State::Done;
                                    }
                                    let request = std::mem::replace(
                                        &mut self.request, Request::new());

                                    return Ok(Some(RequestMessage::Message(request)));
                                } else {
                                    return Err(Error::BadHeader);
                                }
                            } else {
                                break
                            }
                        } else if is_ows(ch) && self.has_header {
                            // header value continuation
                            state = State::Header(ParseHeader::Value);
                        } else {
                            // append processed header
                            if self.has_header {
                                self.request.headers.append(self.header);
                            } else {
                                self.has_header = true
                            }
                            // new header
                            state = State::Header(ParseHeader::Name);
                            header_name = ParseHeaderName::New;
                            self.header.set_name_pos(bytes.pos());
                        },
                    None => break
                },
                ParseHeader::Name => {
                    // parse header name
                    let len = bytes.len();
                    for idx in 0..len {
                        let ch = bytes.next();
                        if ch == b':' {
                            let h = self.header_name_hash.finish();
                            self.header_name_hash = DefaultHasher::new();
                            self.header.set_hash(h);
                            self.header.update_name_len(idx);
                            if self.header.is_overflow(self.max_line_size) {
                                return Err(Error::LineTooLong)
                            }

                            // move char pointer and prepare value parse
                            bytes.advance(idx+1);
                            header_token = ParseTokens::New;
                            state = State::Header(ParseHeader::OWS);

                            // complete header name parsing
                            if !header_name.completed() {
                                header_name = ParseHeaderName::General;
                            }
                            self.header_name = header_name;
                            continue 'run
                        } else if !is_token(ch) {
                            return Err(Error::BadHeader);
                        }
                        let ch = ch.to_ascii_lowercase();
                        self.header_name_hash.write_u8(ch);

                        // parse actual header name
                        header_name = header_name.next(ch);
                    }
                    bytes.advance(len);
                    self.header_name = header_name;
                    self.header.update_name_len(len);
                    if self.header.is_overflow(self.max_line_size) {
                        return Err(Error::LineTooLong)
                    }
                    break
                },
                ParseHeader::OWS => match parse_ows(&mut bytes)? {
                    // strip OWS
                    Status::Complete(..) => {
                        self.header.set_value_pos(bytes.pos());
                        if let ParseHeaderName::ContentLength(..) = header_name {
                            state = State::Header(ParseHeader::ContentLength);
                        } else {
                            state = State::Header(ParseHeader::Value);
                        }
                    },
                    Status::Partial(..) => break,
                },
                ParseHeader::ContentLength => {
                    // parse content length
                    let len = bytes.len();
                    for idx in 0..len {
                        let ch = bytes.next();
                        if ch == CR {
                            bytes.advance(idx+1);
                            state = State::Header(ParseHeader::ValueEol);
                            self.header.update_value_len(idx);

                            // parse content-length value
                            let l = unsafe {
                                std::str::from_utf8_unchecked(&src[self.header.value_range()]) };
                            match l.parse::<u64> () {
                                Ok(v) => self.length = Some(v),
                                Err(..) => return Err(Error::ContentLength)
                            }
                            continue 'run
                        } else if !is_num(ch) {
                            return Err(Error::ContentLength);
                        }
                    }
                    bytes.advance(len);
                    self.header.update_name_len(len);
                    break
                },
                ParseHeader::Value => {
                    // any parse header
                    let len = bytes.len();
                    for idx in 0..len {
                        let ch = bytes.next();
                        if ch == CR {
                            bytes.advance(idx+1);
                            // check for specific tokens
                            if header_token.completed() {
                                self.update_msg_state(header_token);
                            }
                            state = State::Header(ParseHeader::ValueEol);
                            self.header.update_value_len(idx);
                            if self.header.is_overflow(self.max_line_size) {
                                return Err(Error::LineTooLong)
                            }
                            continue 'run
                        } else if ! (is_vchar(ch) || is_obs_text(ch) || is_ows(ch)) {
                            return Err(Error::BadHeader);
                        }
                        if is_token(ch) {
                            header_token = header_token.next(ch);
                        } else if ch == b',' || ch == SP {
                            // check for specific tokens
                            if header_token.completed() {
                                self.update_msg_state(header_token);
                            }
                            header_token = ParseTokens::New;
                        } else {
                            header_token = ParseTokens::New;
                        }
                    }
                    bytes.advance(len);
                    self.header_token = header_token;
                    self.header.update_value_len(len);
                    if self.header.is_overflow(self.max_line_size) {
                        return Err(Error::LineTooLong)
                    }
                    break
                },
                ParseHeader::ValueEol =>
                    match parse_crlf(&mut bytes, CRLF::LF, Error::BadHeader)? {
                        Status::Complete(..) =>
                            state = State::Header(ParseHeader::Eol),
                        Status::Partial(..) => break
                    },
            },
            State::Body(step) => match step {
                ParseBody::Length(remaining) => {
                    // Read specific amount bytes
                    let len = src.len();
                    if len > 0 {
                        let len64 = len as u64;
                        if remaining > len64 {
                            self.state = State::Body(ParseBody::Length(remaining - len64));
                            return Ok(Some(
                                RequestMessage::Body(src.split_to(len).freeze())));
                        } else {
                            self.state = State::Done;
                            return Ok(Some(
                                RequestMessage::Body(src.split_to(remaining as usize).freeze())))
                        }
                    } else {
                        return Ok(None)
                    }
                },
                ParseBody::ChunkSize(count) => {
                    // chunk-size = 1*HEXDIG
                    let len = bytes.len();
                    for idx in 0..len {
                        let ch = bytes.get();
                        if ch == b';' || ch == CR {
                            // convert chunk size in hex to u64
                            let count = count + idx;
                            let origin = bytes.origin(count);

                            let hex = unsafe {
                                std::str::from_utf8_unchecked(&src[origin..origin+count]) };

                            let size = match u64::from_str_radix(hex, 16) {
                                Ok(v) => v,
                                Err(..) => return Err(Error::TransferEncoding),
                            };

                            if let Some(ch2) = bytes.get_next_maybe() {
                                if ch2 == LF && ch == CR {
                                    bytes.bump();
                                    bytes.bump();
                                    if size == 0 {
                                        state = State::Body(ParseBody::ChunkMaybeTrailers);
                                    } else {
                                        state = State::Body(ParseBody::Chunk(size));
                                    }
                                    continue 'run
                                }
                            }
                            state = State::Body(ParseBody::ChunkSizeEol(size));
                            continue 'run
                        } else if !is_hex(ch) {
                            return Err(Error::TransferEncoding);
                        }
                        bytes.bump();
                    }
                    state = State::Body(ParseBody::ChunkSize(count+len));
                    break
                },
                ParseBody::ChunkSizeEol(size) => {
                    // chunk ext and crlf: [ chunk-ext ] CRLF
                    let mut prev = 0;
                    let len = bytes.len();

                    for idx in 0..len {
                        let ch = bytes.next();
                        if ch == LF && prev == CR {
                            bytes.advance(idx+1);
                            if size == 0 {
                                state = State::Body(ParseBody::ChunkMaybeTrailers);
                            } else {
                                state = State::Body(ParseBody::Chunk(size));
                            }
                            continue 'run
                        }
                        prev = ch;
                    }
                    break
                },
                ParseBody::Chunk(remaining) => {
                    // Read specific amount bytes
                    let start = bytes.origin_offset();
                    let len = src.len() - start;
                    if len > 0 {
                        let len64 = len as u64;
                        if start != 0 {
                            src.split_to(start);
                        }
                        if remaining > len64 {
                            self.start = 0;
                            self.state = State::Body(ParseBody::Chunk(remaining - len64));
                            return Ok(Some(
                                RequestMessage::Body(src.take().freeze())));
                        } else {
                            self.start = 0;
                            self.state = State::Body(ParseBody::ChunkEOL(CRLF::CR));
                            return Ok(Some(
                                RequestMessage::Body(
                                    src.split_to(remaining as usize).freeze())))
                        }
                    }
                    break
                },
                ParseBody::ChunkEOL(marker) =>
                    match parse_crlf(&mut bytes, marker, Error::TransferEncoding)? {
                        Status::Complete(..) => {
                            state = State::Body(ParseBody::ChunkSize(0))
                        },
                        Status::Partial(marker) => {
                            state = State::Body(ParseBody::ChunkEOL(marker));
                            break
                        },
                    },
                ParseBody::ChunkMaybeTrailers =>
                    if let Some(ch) = bytes.get_maybe() {
                        if ch == CR {
                            if let Some(ch) = bytes.get_next_maybe() {
                                if ch == LF {
                                    state = State::Done;
                                    src.split_to(bytes.pos()+2);
                                    bytes = BytesPtr::new(src.as_ref(), 0);
                                } else {
                                    state = State::Body(ParseBody::ChunkTrailers(CRLF::CR));
                                }
                            } else {
                                break
                            }
                        } else {
                            state = State::Body(ParseBody::ChunkTrailers(CRLF::CR))
                        }
                    } else {
                        break
                    },
                ParseBody::ChunkTrailers(marker) => {
                    let len = bytes.len();
                    for idx in 0..len {
                        let ch = bytes.next();
                        if ch == marker.val() {
                            bytes.advance(idx+1);
                            if marker.val() == LF {
                                state = State::Body(ParseBody::ChunkMaybeTrailers);
                            } else {
                                state = State::Body(ParseBody::ChunkTrailers(CRLF::LF));
                            }
                            continue 'run
                        }
                    }
                    bytes.advance(len);
                    break
                },
                ParseBody::Unsized =>
                    if !src.is_empty() {
                        return Ok(Some(RequestMessage::Body(src.take().freeze())))
                    } else {
                        return Ok(None)
                    },
            },
            State::Done => {
                // reset
                self.start = 0;
                self.meth_pos = 0;
                self.meth_end = 0;
                self.state = State::Status(ParseStatusLine::Skip(CRLF::CR));
                return Ok(Some(RequestMessage::Completed))
            }
            }}
        self.start = bytes.pos();
        self.state = state;
        Ok(None)
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        let item = self.decode(buf)?;

        match self.state {
            State::Body(ParseBody::Length(..)) => Err(Error::PayloadNotCompleted),
            State::Body(ParseBody::Unsized) => Ok(item),
            State::Body(..) => Err(Error::PayloadNotCompleted),
            _ => Ok(item)
        }
    }
}

/// Determines if byte is a token char.
///
/// > ```notrust
/// > token          = 1*tchar
/// >
/// > tchar          = "!" / "#" / "$" / "%" / "&" / "'" / "*"
/// >                / "+" / "-" / "." / "^" / "_" / "`" / "|" / "~"
/// >                / DIGIT / ALPHA
/// >                ; any VCHAR, except delimiters
/// > ```
static TOKENS: [u8; 256] = [
/*   0 nul    1 soh    2 stx    3 etx    4 eot    5 enq    6 ack    7 bel  */
    0,       0,       0,       0,       0,       0,       0,       0,
/*   8 bs     9 ht    10 nl    11 vt    12 np    13 cr    14 so    15 si   */
    0,       0,       0,       0,       0,       0,       0,       0,
/*  16 dle   17 dc1   18 dc2   19 dc3   20 dc4   21 nak   22 syn   23 etb */
    0,       0,       0,       0,       0,       0,       0,       0,
/*  24 can   25 em    26 sub   27 esc   28 fs    29 gs    30 rs    31 us  */
    0,       0,       0,       0,       0,       0,       0,       0,
/*  32 sp    33  !    34  "    35  #    36  $    37  %    38  &    39  '  */
    0,       1,       0,       1,       1,       1,       1,       1,
/*  40  (    41  )    42  *    43  +    44  ,    45  -    46  .    47  /  */
    0,       0,       1,       1,       0,       1,       1,       0,
/*  48  0    49  1    50  2    51  3    52  4    53  5    54  6    55  7  */
    1,       1,       1,       1,       1,       1,       1,       1,
/*  56  8    57  9    58  :    59  ;    60  <    61  =    62  >    63  ?  */
    1,       1,       0,       0,       0,       0,       0,       0,
/*  64  @    65  A    66  B    67  C    68  D    69  E    70  F    71  G  */
    0,       1,       1,       1,       1,       1,       1,       1,
/*  72  H    73  I    74  J    75  K    76  L    77  M    78  N    79  O  */
    1,       1,       1,       1,       1,       1,       1,       1,
/*  80  P    81  Q    82  R    83  S    84  T    85  U    86  V    87  W  */
    1,       1,       1,       1,       1,       1,       1,       1,
/*  88  X    89  Y    90  Z    91  [    92  \    93  ]    94  ^    95  _  */
    1,       1,       1,       0,       0,       0,       1,       1,
/*  96  `    97  a    98  b    99  c   100  d   101  e   102  f   103  g  */
    1,       1,       1,       1,       1,       1,       1,       1,
/* 104  h   105  i   106  j   107  k   108  l   109  m   110  n   111  o  */
    1,       1,       1,       1,       1,       1,       1,       1,
/* 112  p   113  q   114  r   115  s   116  t   117  u   118  v   119  w  */
    1,       1,       1,       1,       1,       1,       1,       1,
/* 120  x   121  y   122  z   123  {   124  |   125  }   126  ~   127 del */
    1,       1,       1,       0,       1,       0,       1,       0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
];

const SP: u8 = b' ';
const CR: u8 = b'\r';
const LF: u8 = b'\n';
const HTAB: u8 = b'\t';

struct Token{
    len: usize,
    token: &'static [u8],
}

const PROXY_CONNECTION: Token = Token {len: 16, token: b"proxy-connection"};
const CONNECTION: Token = Token {len: 10, token: b"connection"};
const CONTENT: Token = Token {len: 8, token: b"content-"};
const CONTENT_LENGTH: Token = Token {len: 14, token: b"content-length"};
const CONTENT_ENCODING: Token = Token {len: 16, token: b"content-encoding"};
const TRANSFER_ENCODING: Token = Token {len: 17, token: b"transfer-encoding"};
const WEBSOCKET: Token = Token {len: 9, token: b"websocket"};
const CHUNKED: Token = Token {len: 7, token: b"chunked"};
const KEEP_ALIVE: Token = Token {len: 10, token: b"keep-alive"};
const CLOSE: Token = Token {len: 5, token: b"close"};
const GZIP: Token = Token {len: 4, token: b"gzip"};
const DEFLATE: Token = Token {len: 7, token: b"deflate"};
const UPGRADE: Token = Token {len: 7, token: b"upgrade"};



#[inline]
fn lower(ch: u8) -> u8 {
    ch | 0x20
}

fn is_num(ch: u8) -> bool {
    ch >= b'0' && ch <= b'9'
}

fn is_hex(ch: u8) -> bool {
    is_num(ch) || ch >= b'a' && ch <= b'f'
}

#[inline]
fn is_vchar(ch: u8) -> bool {
    ch >= b'!' || ch <= b'~'  // 0x21 .. 0x7E
}

#[inline]
fn is_ows(ch: u8) -> bool {
    ch == SP || ch == HTAB
}


#[inline]
fn is_obs_text(ch: u8) -> bool {
    ch >= 0x80 || ch <= 0xfe  // 0x80 .. 0xFF
}


#[inline]
fn is_url_char(ch: u8) -> bool {
    // refer to http_parser.c or ascii table for characters
    ch == b'!' || ch == b'"' || (ch >= b'$' && ch <= b'>') || (ch >= b'@' && ch <= b'~')
}

#[inline]
fn is_url(ch: u8) -> bool {
    is_url_char(ch) || ch == b'?' || ch == b'#'
}

#[inline]
fn is_token(b: u8) -> bool {
    TOKENS[b as usize] != 0
}

#[inline]
fn parse_token(bytes: &mut BytesPtr, stop: u8) -> Result<usize, usize> {
    let len = bytes.len();

    for idx in 0..len {
        let b = bytes.next();
        if b == stop {
            bytes.advance(idx+1);
            return Ok(Status::Complete(idx));
        } else if !is_token(b) {
            return Err(Error::BadStatusLine);
        }
    }
    bytes.advance(len);
    Ok(Status::Partial(len))
}

#[inline]
fn parse_ows(bytes: &mut BytesPtr) -> Result<(), ()> {
    loop {
        if let Some(ch) = bytes.get_maybe() {
            if is_ows(ch) {
                bytes.bump();
                continue
            } else {
                return Ok(Status::Complete(()));
            }
        } else {
            return Ok(Status::Partial(()))
        }
    }
}

#[inline]
fn parse_path(bytes: &mut BytesPtr) -> Result<usize, usize> {
    let len = bytes.len();

    for idx in 0..len {
        let b = bytes.next();
        if b == SP {
            bytes.advance(idx+1);
            return Ok(Status::Complete(idx));
        } else if !is_url(b) {
            return Err(Error::BadStatusLine);
        }
    }
    bytes.advance(len);
    Ok(Status::Partial(len))
}

macro_rules! next {
    ($bytes:ident) => ({
        match $bytes.next_maybe() {
            Some(v) => v,
            None => return Ok(Status::Partial(0))
        }
    })
}

macro_rules! expect {
    ($bytes:ident.next() == $pat:pat => $ret:expr) => {
        match next!($bytes) {
            v@$pat => v,
            _ => return $ret
        }
    }
}

#[inline]
fn parse_crlf(bytes: &mut BytesPtr, marker: CRLF, err: Error) -> Result<(), CRLF> {
    match marker {
        CRLF::CR => match bytes.next_maybe() {
            Some(ch) => {
                if ch != CR {
                    Err(err)
                } else {
                    match bytes.next_maybe() {
                        Some(ch) =>
                            if ch != LF {
                                Err(err)
                            } else {
                                Ok(Status::Complete(()))
                            },
                        None => Ok(Status::Partial(CRLF::LF)),
                    }
                }
            },
            None => Ok(Status::Partial(CRLF::LF)),
        },
        CRLF::LF => match bytes.next_maybe() {
            Some(ch) => {
                if ch != LF {
                    Err(err)
                } else {
                    Ok(Status::Complete(()))
                }
            },
            None => Ok(Status::Partial(CRLF::LF)),
        }
    }
}

#[inline]
fn parse_version(bytes: &mut BytesPtr) -> Result<Version, usize> {
    if bytes.len() < 9 {
        Ok(Status::Partial(0))
    } else {
        expect!(bytes.next() == b'H' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'T' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'T' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'P' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'/' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'1' => Err(Error::BadStatusLine));
        expect!(bytes.next() == b'.' => Err(Error::BadStatusLine));
        let v = match next!(bytes) {
            b'0' => Version::Http10,
            b'1' => Version::Http11,
            _ => return Err(Error::BadStatusLine)
        };
        Ok(Status::Complete(v))
    }
}

struct BytesPtr {
    ptr: *const u8,
    size: usize,
    len: usize,
}


impl BytesPtr {

    #[inline]
    fn new(slice: &[u8], start: usize) -> BytesPtr {
        let len = slice.len();
        let ptr = if start > 0 {
            unsafe { slice.as_ptr().offset(start as isize) }
        } else {
            slice.as_ptr()
        };
        BytesPtr {
            ptr: ptr,
            size: len,
            len: len - start,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[inline]
    fn pos(&self) -> usize {
        self.size - self.len
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        self.len -= cnt
    }

    #[inline]
    fn bump(&mut self) {
        self.len -= 1;
        self.ptr = unsafe { self.ptr.offset(1) }
    }

    #[inline]
    fn origin(&self, count: usize) -> usize {
        self.size - self.len - count
    }

    #[inline]
    fn origin_offset(&self) -> usize {
        self.size - self.len
    }

    #[inline]
    fn get(&mut self) -> u8 {
        unsafe { *self.ptr }
    }

    #[inline]
    fn get_maybe(&mut self) -> Option<u8> {
        if self.len != 0 {
            unsafe { Some(*self.ptr) }
        } else {
            None
        }
    }

    #[inline]
    fn get_next_maybe(&mut self) -> Option<u8> {
        if self.len > 1 {
            unsafe { Some(*self.ptr.offset(1)) }
        } else {
            None
        }
    }

    #[inline]
    fn next(&mut self) -> u8 {
        unsafe {
            let b = *self.ptr;
            self.ptr = self.ptr.offset(1);
            b
        }
    }

    #[inline]
    fn next_maybe(&mut self) -> Option<u8> {
        if self.len != 0 {
            unsafe {
                let b = *self.ptr;
                self.len -= 1;
                self.ptr = self.ptr.offset(1);
                Some(b)
            }
        } else {
            None
        }
    }

}
