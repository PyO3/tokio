extern crate bytes;
extern crate tokio_io;
extern crate async_tokio;

use bytes::BytesMut;
use tokio_io::codec::{Decoder};
use async_tokio::http::{ContentCompression, Error, RequestCodec, RequestMessage, Version};

macro_rules! test {
    ($name:ident, $($data:expr),+ => |$codec:ident, $buf:ident| $body:expr) => (
        #[test]
        fn $name() {
            let mut $codec = RequestCodec::new();
            let mut $buf = BytesMut::from(concat!(
                $( $data ),+
            ));
            $body
        }
    )
}

macro_rules! expect_status {
    ($codec:ident($buf:ident): $meth:expr, $path:expr, $ver:expr) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(msg)) => {
                if let RequestMessage::Status(status) = msg {
                    assert_eq!(status.method(), $meth);
                    assert_eq!(status.path(), $path);
                    assert_eq!(status.version, $ver);
                } else {
                    assert!(false, "RequestMessage::Status is required");
                }
            }
        }
    }
}

macro_rules! expect_headers {
    ($codec:ident($buf:ident): $(($hdr_name:expr, $hdr_val:expr)),+) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(msg)) => match msg {
                RequestMessage::Headers(headers) => {
                    let mut iter = headers.iter();

                    $(
                        if let Some(item) = iter.next() {
                            assert_eq!(item.0, $hdr_name);
                            assert_eq!(item.1, $hdr_val);
                        } else {
                            assert!(false, format!(
                                "Header {} expected, none found", $hdr_name));
                        };
                    )*
                }
                _ => assert!(false, "RequestMessage::HeadersComplete is required"),
            }
        }
    }
}

macro_rules! expect_headers_complete {
    ($codec:ident($buf:ident)) => (
        expect_headers_complete!(
            $codec($buf): close:false, chunked:false, upgrade:false,
            compress:ContentCompression::Default);
    );
    ($codec:ident($buf:ident): chunked:$chunked:expr) => (
        expect_headers_complete!
            ($codec($buf): close:false, chunked:$chunked, upgrade:false,
             compress:ContentCompression::Default);
    );
    ($codec:ident($buf:ident): upgrade:$upgrade:expr) => (
        expect_headers_complete!(
            $codec($buf): close:false, chunked:false, upgrade:$upgrade,
            compress:ContentCompression::Default);
    );
    ($codec:ident($buf:ident): compress:$compress:expr) => (
        expect_headers_complete!(
            $codec($buf): close:false, chunked:false, upgrade:false, compress:$compress);
    );
    ($codec:ident($buf:ident): close:$close:expr,
     chunked:$chunked:expr, upgrade:$upgrade:expr) => (
        expect_headers_complete!(
            $codec($buf): close:$close, chunked:$chunked, upgrade:$upgrade,
            compress:ContentCompression::Default);
    );
    ($codec:ident($buf:ident): close:$close:expr,
     chunked:$chunked:expr, upgrade:$upgrade:expr, compress:$compress:expr) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(msg)) => match msg {
                RequestMessage::HeadersCompleted {close, chunked, upgrade, compress} => {
                    assert_eq!(close, $close);
                    assert_eq!(chunked, $chunked);
                    assert_eq!(upgrade, $upgrade);
                    assert_eq!(compress, $compress);
                },
                _ => assert!(false, "RequestMessage::HeadersComplete is required"),
            }
        }
    }
}

macro_rules! expect_body {
    ($codec:ident($buf:ident): $body:expr) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(msg)) => match msg {
                RequestMessage::Body(body) => assert_eq!(body, $body),
                _ => assert!(false, "RequestMessage::Body is required"),
            }
        }
    }
}

macro_rules! expect_error {
    ($codec:ident($buf:ident): $err:pat) => {
        match $codec.decode(&mut $buf) {
            Err($err) => (),
            Err(err) => assert!(false, format!("Got unexpected error: {:?}", err)),
            Ok(None) => assert!(false, "Excepted error, did not get any result"),
            Ok(Some(msg)) => assert!(false, format!("Expected error got message: {:?}", msg)),
        }
    }
}

macro_rules! expect_completed {
    ($codec:ident($buf:ident)) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(msg)) => match msg {
                RequestMessage::Completed => (),
                _ => assert!(false, "RequestMessage::Completed is required"),
            }
        }
    }
}

macro_rules! expect_wait {
    ($codec:ident($buf:ident)) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => (),
            Ok(Some(msg)) => assert!(false, format!("Got unexpected message: {:?}", msg)),
        }
    }
}


test! { test_request_simple,
        "GET / HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/", Version::Http11);
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
        }}

test! { test_request_simple_10,
        "POST / HTTP/1.0\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "POST", "/", Version::Http10);
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_parse_body,
        "GET /test HTTP/1.1\r\n",
        "Content-Length: 4\r\n\r\nbody" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("Content-Length", "4"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_body!(codec(buf): "body");
            expect_completed!(codec(buf));
        }}

test! { test_parse_delayed,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_wait!(codec(buf));

            buf.extend(b"\r\n");

            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_headers_multi_feed,
        "GE" => |codec, buf| {
            expect_wait!(codec(buf));
            buf.extend(b"T /te");
            expect_wait!(codec(buf));
            buf.extend(b"st HTTP/1.1");
            expect_wait!(codec(buf));
            buf.extend(b"\r\n");

            expect_status!(codec(buf): "GET", "/test", Version::Http11);

            buf.extend(b"test: line\r");
            expect_wait!(codec(buf));
            buf.extend(b"\n");
            expect_wait!(codec(buf));

            buf.extend(b" continue\r\n\r\n");

            //expect_headers!(codec(buf): ("test", "line \r\ncontinue\r\n"));
            //expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            //expect_completed!(codec(buf));
        }}

test! { test_parse_headers_multi,
        "GET /test HTTP/1.1\r\n",
        "Set-Cookie: c1=cookie1\r\n",
        "Set-Cookie: c2=cookie2\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("Set-Cookie", "c1=cookie1"),
                            ("Set-Cookie", "c2=cookie2"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }
}

test! { test_conn_default_1_0,
        "GET /test HTTP/1.0\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http10);
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }
}

test! { test_conn_default_1_1,
        "GET /test HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_close,
        "GET /test HTTP/1.1\r\n",
        "connection: close\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("connection", "close"));
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_close_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: close\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http10);
            expect_headers!(codec(buf): ("connection", "close"));
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_keep_alive_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: keep-alive\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http10);
            expect_headers!(codec(buf): ("connection", "keep-alive"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_other_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: test\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http10);
            expect_headers!(codec(buf): ("connection", "test"));
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_other_1_1,
        "GET /test HTTP/1.1\r\n",
        "connection: test\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("connection", "test"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_request_chunked,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): close:false, chunked:true, upgrade:false);
        }}

test! { test_request_chunked_partial,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunk\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunk"));
            expect_headers_complete!(codec(buf): chunked:false);
        }}

test! { test_special_headers_partial,
        "GET /test HTTP/1.1\r\n",
        "transfer-encod: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encod", "chunked"));
            expect_headers_complete!(codec(buf): chunked:false);
        }}

test! { test_conn_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: upgrade\r\n",
        "upgrade: websocket\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("connection", "upgrade"),
                            ("upgrade", "websocket"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:true);
            expect_completed!(codec(buf));
        }}

test! { test_conn_close_and_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: close, upgrade\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("connection", "close, upgrade"));
            expect_headers_complete!(codec(buf): close:true, chunked:false, upgrade:true);
            expect_completed!(codec(buf));
        }}

test! { test_compression_deflate,
        "GET /test HTTP/1.1\r\n",
        "content-Encoding: deflate\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("content-Encoding", "deflate"));
            expect_headers_complete!(codec(buf): compress:ContentCompression::Deflate);
        }}

test! { test_compression_gzip,
        "GET /test HTTP/1.1\r\n",
        "content-encoding: gzip\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("content-encoding", "gzip"));
            expect_headers_complete!(codec(buf): compress:ContentCompression::Gzip);
        }}

test! { test_compression_unknown,
        "GET /test HTTP/1.1\r\n",
        "content-encoding: compress\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("content-encoding", "compress"));
            expect_headers_complete!(codec(buf): compress:ContentCompression::Default);
        }}

// def test_headers_connect(parser):
// text = (b'CONNECT www.google.com HTTP/1.1\r\n'
//         b'content-length: 0\r\n\r\n')
//     messages, upgrade, tail = parser.feed_data(text)
//     msg, payload = messages[0]
//     assert upgrade
//     assert isinstance(payload, streams.FlowControlStreamReader)

// def test_headers_old_websocket_key1(parser):
// text = (b'GET /test HTTP/1.1\r\n'
//         b'SEC-WEBSOCKET-KEY1: line\r\n\r\n')
//  with pytest.raises(http_exceptions.BadHttpMessage):
// parser.feed_data(text)

test! { test_headers_content_length_err_1,
       "GET /test HTTP/1.1\r\n",
       "content-length: line\r\n\r\n" => |codec, buf| {
           expect_status!(codec(buf): "GET", "/test", Version::Http11);
           expect_error!(codec(buf): Error::ContentLength);
       }}

test! {test_headers_content_length_err_2,
       "GET /test HTTP/1.1\r\n",
       "content-length: -1\r\n\r\n" => |codec, buf| {
           expect_status!(codec(buf): "GET", "/test", Version::Http11);
           expect_error!(codec(buf): Error::ContentLength);
       }}

test! {test_invalid_header,
      "GET /test HTTP/1.1\r\n",
      "test line\r\n\r\n" => |codec, buf| {
          expect_status!(codec(buf): "GET", "/test", Version::Http11);
          expect_error!(codec(buf): Error::BadHeader);
      }}

test! { test_invalid_name,
        "GET /test HTTP/1.1\r\n",
        "test[]: line\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_error!(codec(buf): Error::BadHeader);
        }}

test! { test_max_header_name_size,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            buf.extend([b't'; 10 * 1024][..].as_ref());
            buf.extend(b":data\r\n\r\n");
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_error!(codec(buf): Error::LineTooLong);
        }}

test! { test_max_header_value_size,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            buf.extend(b"header:");
            buf.extend([b't'; 10 * 1024][..].as_ref());
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_error!(codec(buf): Error::LineTooLong);
        }}

//test! { test_max_header_value_size_continuation,
//        "GET /test HTTP/1.1\r\n" => |codec, buf| {
//            buf.extend(b"header: test\r\n ");
//            buf.extend([b't'; 10 * 1024][..].as_ref());
//            expect_status!(codec(buf): "GET", "/test", Version::Http11);
//            expect_error!(codec(buf): Error::LineTooLong);
//        }}

test! { test_http_request_bad_status_line,
        "getpath \r\n\r\n" => |codec, buf| {
            expect_error!(codec(buf): Error::BadStatusLine);
        }}

test! { test_http_request_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: upgrade\r\n",
        "upgrade: websocket\r\n\r\n",
        "some raw data" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf):
                            ("connection", "upgrade"),
                            ("upgrade", "websocket"));
            expect_headers_complete!(codec(buf): upgrade:true);
            //except_bod!(codec(buf): "some raw data");
            //expect_completed!(codec(buf));
        }}

test! { test_http_request_parser_utf8,
        "GET /path HTTP/1.1\r\n",
        "x-test: тест\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/path", Version::Http11);
            expect_headers!(codec(buf): ("x-test", "тест"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

// TODO: cp1251
test! { test_http_request_parser_non_utf8,
        "GET /path HTTP/1.1\r\n",
        "x-test: тест\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/path", Version::Http11);
            expect_headers!(codec(buf): ("x-test", "тест"));
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_http_request_parser_two_slashes,
        "GET //path HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "//path", Version::Http11);
            expect_headers_complete!(codec(buf): close:false, chunked:false, upgrade:false);
            expect_completed!(codec(buf));
        }}

test! { test_http_request_parser_bad_method,
        "!12%()+=~$ /get HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_error!(codec(buf): Error::BadStatusLine);
        }}

test! { test_http_request_parser_bad_version,
        "GET //get HT/11\r\n\r\n" => |codec, buf| {
            expect_error!(codec(buf): Error::BadStatusLine);
        }}

test! { test_http_request_max_status_line,
        "GET /path" => |codec, buf| {
            buf.extend([b't'; 10 * 1024][..].as_ref());
            expect_error!(codec(buf): Error::LineTooLong);
        }}

test! { test_http_request_chunked_payload,
        "GET /test HTTP/1.1\r\n",
        "Transfer-encoding: chunked\r\n\r\n",
        "4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("Transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): close:false, chunked:true, upgrade:false);
            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));
        }}


test! { test_http_request_chunked_payload_and_next_message,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): chunked:true);

            buf.extend(b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
            buf.extend(b"POST /test2 HTTP/1.1\r\n");
            buf.extend(b"transfer-encoding: chunked\r\n\r\n");

            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));

            expect_status!(codec(buf): "POST", "/test2", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): chunked:true);
        }}

test! { test_http_request_chunked_payload_chunks,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): chunked:true);

            buf.extend(b"4\r\ndata\r");
            expect_body!(codec(buf): "data");

            buf.extend(b"\n4");
            expect_wait!(codec(buf));

            buf.extend(b"\r");
            expect_wait!(codec(buf));

            buf.extend(b"\n");
            expect_wait!(codec(buf));

            buf.extend(b"line\r\n0\r\n");
            expect_body!(codec(buf): "line");

            // Trailers
            buf.extend(b"test: test\r\n");
            expect_wait!(codec(buf));

            buf.extend(b"\r\n");
            expect_completed!(codec(buf));
        }}

test! { test_parse_chunked_payload_chunk_extension,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/test", Version::Http11);
            expect_headers!(codec(buf): ("transfer-encoding", "chunked"));
            expect_headers_complete!(codec(buf): chunked:true);

            buf.extend(b"4;test\r\ndata\r\n4\r\nline\r\n0\r\ntest: test\r\n\r\n".as_ref());
            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));
        }}

test! { test_parse_length_payload,
        "GET /path HTTP/1.1\r\n",
        "content-length: 4\r\n\r\n" => |codec, buf| {
            expect_status!(codec(buf): "GET", "/path", Version::Http11);
            expect_headers!(codec(buf): ("content-length", "4"));
            expect_headers_complete!(codec(buf));

            buf.extend(b"da");
            expect_body!(codec(buf): "da");
            buf.extend(b"taPO");
            expect_body!(codec(buf): "ta");
            expect_completed!(codec(buf));
        }}
