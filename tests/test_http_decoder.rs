extern crate bytes;
extern crate tokio_io;
extern crate async_tokio;

use bytes::BytesMut;
use tokio_io::codec::{Decoder};
use async_tokio::http::{
    ConnectionType, ContentCompression, Error, RequestDecoder, RequestMessage, Version};

macro_rules! test {
    ($name:ident, $($data:expr),+ => |$codec:ident, $buf:ident| $body:expr) => (
        #[test]
        fn $name() {
            let mut $codec = RequestDecoder::new();
            let mut $buf = BytesMut::from(concat!(
                $( $data ),+
            ));
            $body
        }
    )
}

macro_rules! expect_status {
    ($msg:ident => $codec:ident($buf:ident) => $meth:expr, $path:expr, $ver:expr) => {
        #[allow(unused_variables)]
        let $msg = match $codec.decode(&mut $buf) {
            Err(err) => panic!(format!("Got error: {:?}", err)),
            Ok(None) => panic!("Did not get any result"),
            Ok(Some(msg)) => {
                if let RequestMessage::Message(msg) = msg {
                    assert_eq!(msg.method(), $meth);
                    assert_eq!(msg.path(), $path);
                    assert_eq!(msg.version, $ver);
                    msg
                } else {
                    panic!("RequestMessage::Status is required");
                }
            }
        };
    }
}

macro_rules! expect_headers {
    ($msg:ident) => (
        expect_headers!(
            $msg => conn:ConnectionType::KeepAlive, chunked:false,
            compress:ContentCompression::Default);
    );
    ($msg:ident => $(($hdr_name:expr, $hdr_val:expr)),+) => (
        expect_headers!(
            $msg => conn:ConnectionType::KeepAlive, chunked:false,
            compress:ContentCompression::Default $(,($hdr_name, $hdr_val))*);
    );
    ($msg:ident => chunked:$chunked:expr $(,($hdr_name:expr, $hdr_val:expr))*) => (
        expect_headers!(
            $msg => conn:ConnectionType::KeepAlive, chunked:$chunked,
             compress:ContentCompression::Default $(,($hdr_name, $hdr_val))*);
    );
    ($msg:ident => conn:$conn:expr $(,($hdr_name:expr, $hdr_val:expr))*) => (
        expect_headers!(
            $msg => conn:$conn, chunked:false,
            compress:ContentCompression::Default $(,($hdr_name, $hdr_val))*);
    );
    ($msg:ident => compress:$compress:expr $(,($hdr_name:expr, $hdr_val:expr))*) => (
        expect_headers!(
            $msg => conn:ConnectionType::KeepAlive, chunked:false,
            compress:$compress $(,($hdr_name, $hdr_val))*);
    );
    ($msg:ident => conn:$conn:expr, chunked:$chunked:expr
     $(,($hdr_name:expr, $hdr_val:expr))*) => (
        expect_headers!(
            $msg => conn:$conn, chunked:$chunked,
            compress:ContentCompression::Default $(,($hdr_name, $hdr_val))*);
    );
    ($msg:ident => conn:$conn:expr, chunked:$chunked:expr, compress:$compress:expr
     $(,($hdr_name:expr, $hdr_val:expr))*)
        => {
            #[allow(unused_variables)]
            assert_eq!($msg.connection, $conn);
            assert_eq!($msg.chunked, $chunked);
            assert_eq!($msg.compress, $compress);

            $(
                if let Some(val) = $msg.headers.get($hdr_name) {
                    assert_eq!(val, $hdr_val);
                } else {
                    assert!(false, format!(
                        "Header {} expected, none found", $hdr_name));
                };
            )*;
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

macro_rules! expect_none {
    ($codec:ident($buf:ident)) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => (),
            Ok(Some(msg)) => assert!(false, format!("Got unexpected message: {:?}", msg)),
        }
    }
}


macro_rules! expect_eof_body {
    ($codec:ident($buf:ident): $body:expr) => {
        match $codec.decode_eof(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(RequestMessage::Body(body))) => assert_eq!(body, $body),
            Ok(Some(msg)) => assert!(false, "RequestMessage::Body is expected, got {:?}", msg),
        }
    }
}

macro_rules! expect_eof_error {
    ($codec:ident($buf:ident): $err:pat) => {
        match $codec.decode_eof(&mut $buf) {
            Err($err) => (),
            Err(err) => assert!(false, format!("Got unexpected error: {:?}", err)),
            Ok(None) => assert!(false, "Excepted error, did not get any result"),
            Ok(Some(msg)) => assert!(false, format!("Expected error got message: {:?}", msg)),
        }
    }
}

macro_rules! expect_eof_none {
    ($codec:ident($buf:ident)) => {
        match $codec.decode_eof(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => (),
            Ok(Some(msg)) => assert!(false, format!("Got unexpected message: {:?}", msg)),
        }
    }
}

macro_rules! expect_eof_completed {
    ($codec:ident($buf:ident)) => {
        match $codec.decode(&mut $buf) {
            Err(err) => assert!(false, format!("Got error: {:?}", err)),
            Ok(None) => assert!(false, "Did not get any result"),
            Ok(Some(RequestMessage::Completed)) => (),
            Ok(Some(msg)) => assert!(false, "RequestMessage::Completed is expected, got {:?}", msg),
        }
    }
}


test! { test_request_simple,
        "GET / HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
        }}

test! { test_request_simple_10,
        "POST / HTTP/1.0\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "POST", "/", Version::Http10);
            expect_headers!(msg => conn:ConnectionType::Close, chunked:false);
            expect_completed!(codec(buf));
        }}

test! { test_request_simple_with_prefix_cr,
        "\rGET / HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
        }}

test! { test_request_simple_with_prefix_crlf,
        "\r\nGET / HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
        }}

test! { test_parse_body,
        "GET /test HTTP/1.1\r\n",
        "Content-Length: 4\r\n\r\nbody" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("Content-Length", "4"));
            expect_body!(codec(buf): "body");
            expect_completed!(codec(buf));
        }}

test! { test_parse_delayed,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            expect_none!(codec(buf));

            buf.extend(b"\r\n");

            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
            expect_completed!(codec(buf));
        }}

test! { test_headers_multi_feed,
        "GE" => |codec, buf| {
            expect_none!(codec(buf));
            buf.extend(b"T /te");
            expect_none!(codec(buf));
            buf.extend(b"st HTTP/1.1");
            expect_none!(codec(buf));
            buf.extend(b"\r\n");

            // expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);

            buf.extend(b"test: line\r");
            expect_none!(codec(buf));
            buf.extend(b"\n");
            expect_none!(codec(buf));

            buf.extend(b" continue\r\n\r\n");

            //expect_headers!(codec(buf): ("test", "line \r\ncontinue\r\n"));
            //expect_headers_complete!(codec(buf): close:false, chunked:false);
            //expect_completed!(codec(buf));
        }}

//test! { test_parse_headers_multi,
//        "GET /test HTTP/1.1\r\n",
//        "Set-Cookie: c1=cookie1\r\n",
//        "Set-Cookie: c2=cookie2\r\n\r\n" => |codec, buf| {
//            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
//            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
//                            ("Set-Cookie", "c1=cookie1"),
//                            ("Set-Cookie", "c2=cookie2"));
//            expect_completed!(codec(buf));
//        }}

test! { test_parse_headers_max_multi,
        "GET /test HTTP/1.1\r\n",
        "Header1: val1\r\n",
        "Header2: val2\r\n",
        "Header3: val3\r\n",
        "Header4: val4\r\n",
        "Header5: val5\r\n",
        "Header6: val6\r\n",
        "Header7: val7\r\n",
        "Header8: val8\r\n",
        "Header9: val9\r\n",
        "Header10: val10\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("Header1", "val1"),
                            ("Header2", "val2"),
                            ("Header3", "val3"),
                            ("Header4", "val4"),
                            ("Header5", "val5"),
                            ("Header6", "val6"),
                            ("Header7", "val7"),
                            ("Header8", "val8"),
                            ("Header9", "val9"),
                            ("Header10", "val10"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_default_1_0,
        "GET /test HTTP/1.0\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http10);
            expect_headers!(msg => conn:ConnectionType::Close, chunked:false);
            expect_completed!(codec(buf));
        }
}

test! { test_conn_default_1_1,
        "GET /test HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
            expect_completed!(codec(buf));
        }}

test! { test_conn_close,
        "GET /test HTTP/1.1\r\n",
        "connection: close\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::Close, chunked:false,
                             ("connection", "close"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_close_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: close\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http10);
            expect_headers!(msg => conn:ConnectionType::Close, chunked:false,
                            ("connection", "close"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_keep_alive_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: keep-alive\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http10);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("connection", "keep-alive"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_other_1_0,
        "GET /test HTTP/1.0\r\n",
        "connection: test\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http10);
            expect_headers!(msg => conn:ConnectionType::Close, chunked:false,
                            ("connection", "test"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_other_1_1,
        "GET /test HTTP/1.1\r\n",
        "connection: test\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("connection", "test"));
            expect_completed!(codec(buf));
        }}

test! { test_request_chunked,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:true,
                            ("transfer-encoding", "chunked"));
        }}

test! { test_request_chunked_partial,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunk\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => chunked:false, ("transfer-encoding", "chunk"));
        }}

test! { test_special_headers_partial,
        "GET /test HTTP/1.1\r\n",
        "transfer-encod: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => chunked:false, ("transfer-encod", "chunked"));
        }}

test! { test_conn_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: upgrade\r\n",
        "upgrade: websocket\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::Upgrade, chunked:false,
                            ("connection", "upgrade"),
                            ("upgrade", "websocket"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_close_and_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: close, upgrade\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::Upgrade, chunked:false,
                            ("connection", "close, upgrade"));
            expect_completed!(codec(buf));
        }}

test! { test_conn_close_and_upgrade_order,
        "GET /test HTTP/1.1\r\n",
        "connection: upgrade, close\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::Upgrade, chunked:false,
                            ("connection", "upgrade, close"));
            expect_completed!(codec(buf));
        }}


test! { test_compression_deflate,
        "GET /test HTTP/1.1\r\n",
        "content-Encoding: deflate\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => compress:ContentCompression::Deflate,
                            ("content-Encoding", "deflate"));
        }}

test! { test_compression_gzip,
        "GET /test HTTP/1.1\r\n",
        "content-encoding: gzip\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => compress:ContentCompression::Gzip,
                            ("content-encoding", "gzip"));
        }}

test! { test_compression_unknown,
        "GET /test HTTP/1.1\r\n",
        "content-encoding: compress\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => compress:ContentCompression::Default,
                            ("content-encoding", "compress"));
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
           expect_error!(codec(buf): Error::ContentLength);
       }}

test! {test_headers_content_length_err_2,
       "GET /test HTTP/1.1\r\n",
       "content-length: -1\r\n\r\n" => |codec, buf| {
           expect_error!(codec(buf): Error::ContentLength);
       }}

test! {test_invalid_header,
      "GET /test HTTP/1.1\r\n",
      "test line\r\n\r\n" => |codec, buf| {
          expect_error!(codec(buf): Error::BadHeader);
      }}

test! { test_invalid_name,
        "GET /test HTTP/1.1\r\n",
        "test[]: line\r\n\r\n" => |codec, buf| {
            expect_error!(codec(buf): Error::BadHeader);
        }}

test! { test_max_header_name_size,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            buf.extend([b't'; 10 * 1024][..].as_ref());
            buf.extend(b":data\r\n\r\n");
            expect_error!(codec(buf): Error::LineTooLong);
        }}

test! { test_max_header_value_size,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            buf.extend(b"header:");
            buf.extend([b't'; 10 * 1024][..].as_ref());
            expect_error!(codec(buf): Error::LineTooLong);
        }}

test! { test_max_header_value_size_continuation,
        "GET /test HTTP/1.1\r\n" => |codec, buf| {
            buf.extend(b"header: test\r\n ");
            buf.extend([b't'; 10 * 1024][..].as_ref());
            expect_error!(codec(buf): Error::LineTooLong);
        }}

test! { test_http_request_bad_status_line,
        "getpath \r\n\r\n" => |codec, buf| {
            expect_error!(codec(buf): Error::BadStatusLine);
        }}

test! { test_http_request_upgrade,
        "GET /test HTTP/1.1\r\n",
        "connection: upgrade\r\n",
        "upgrade: websocket\r\n\r\n",
        "some raw data" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::Upgrade,
                            ("connection", "upgrade"),
                            ("upgrade", "websocket"));
            //except_bod!(codec(buf): "some raw data");
            //expect_completed!(codec(buf));
        }}

test! { test_http_request_parser_utf8,
        "GET /path HTTP/1.1\r\n",
        "x-test: тест\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/path", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("x-test", "тест"));
            expect_completed!(codec(buf));
        }}

// TODO: cp1251
test! { test_http_request_parser_non_utf8,
        "GET /path HTTP/1.1\r\n",
        "x-test: тест\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/path", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                             ("x-test", "тест"));
            expect_completed!(codec(buf));
        }}

test! { test_http_request_parser_two_slashes,
        "GET //path HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "//path", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false);
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
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:true,
                            ("Transfer-encoding", "chunked"));
            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));
        }}


test! { test_http_request_chunked_payload_and_next_message,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => chunked:true, ("transfer-encoding", "chunked"));

            buf.extend(b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
            buf.extend(b"POST /test2 HTTP/1.1\r\n");
            buf.extend(b"transfer-encoding: chunked\r\n\r\n");

            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));

            expect_status!(msg => codec(buf) => "POST", "/test2", Version::Http11);
            expect_headers!(msg => chunked:true,
                            ("transfer-encoding", "chunked"));
        }}

test! { test_http_request_chunked_payload_chunks,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => chunked:true, ("transfer-encoding", "chunked"));

            buf.extend(b"4\r\ndata\r");
            expect_body!(codec(buf): "data");

            buf.extend(b"\n4");
            expect_none!(codec(buf));

            buf.extend(b"\r");
            expect_none!(codec(buf));

            buf.extend(b"\n");
            expect_none!(codec(buf));

            buf.extend(b"line\r\n0\r\n");
            expect_body!(codec(buf): "line");

            // Trailers
            buf.extend(b"test: test\r\n");
            expect_none!(codec(buf));

            buf.extend(b"\r\n");
            expect_completed!(codec(buf));
        }}

test! { test_parse_chunked_payload_chunk_extension,
        "GET /test HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/test", Version::Http11);
            expect_headers!(msg => chunked:true, ("transfer-encoding", "chunked"));

            buf.extend(b"4;test\r\ndata\r\n4\r\nline\r\n0\r\ntest: test\r\n\r\n".as_ref());
            expect_body!(codec(buf): "data");
            expect_body!(codec(buf): "line");
            expect_completed!(codec(buf));
        }}

test! { test_parse_length_payload,
        "GET /path HTTP/1.1\r\n",
        "content-length: 4\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/path", Version::Http11);
            expect_headers!(msg => ("content-length", "4"));

            buf.extend(b"da");
            expect_body!(codec(buf): "da");
            buf.extend(b"taPO");
            expect_body!(codec(buf): "ta");
            expect_completed!(codec(buf));
        }}


test! { test_parse_no_length_payload,
        "PUT / HTTP/1.1\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "PUT", "/", Version::Http11);
            expect_headers!(msg);
            expect_eof_completed!(codec(buf));
        }}

test! { test_parse_eof_payload,
        "PUT / HTTP/1.1\r\n",
        "Content-Length: 4\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "PUT", "/", Version::Http11);
            expect_headers!(msg => ("Content-Length", "4"));

            buf.extend(b"data");
            expect_eof_body!(codec(buf): &"data");
        }}

test! { test_parse_length_payload_eof,
        "PUT / HTTP/1.1\r\n",
        "Content-Length: 4\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "PUT", "/", Version::Http11);
            expect_headers!(msg => ("Content-Length", "4"));

            buf.extend(b"da");
            expect_eof_error!(codec(buf): Error::PayloadNotCompleted);
        }}

test! { test_parse_chunked_payload_size_error,
        "PUT / HTTP/1.1\r\n",
        "transfer-encoding: chunked\r\n\r\n",
        "blah\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "PUT", "/", Version::Http11);
            expect_headers!(msg => chunked:true, ("transfer-encoding", "chunked"));
            expect_error!(codec(buf): Error::TransferEncoding);
        }}


test! { test_http_payload_parser_length,
        "PUT / HTTP/1.1\r\n",
        "Content-Length: 2\r\n\r\n",
        "1245" => |codec, buf| {
            expect_status!(msg => codec(buf) => "PUT", "/", Version::Http11);
            expect_headers!(msg => ("Content-Length", "2"));
            expect_body!(codec(buf): &"12");
            expect_completed!(codec(buf));
            assert_eq!(buf[..], b"45"[..]);
        }}


test! { test_http_request_websocket,
        "GET /path HTTP/1.1\r\n",
        "Upgrade: Websocket\r\n\r\n" => |codec, buf| {
            expect_status!(msg => codec(buf) => "GET", "/path", Version::Http11);
            expect_headers!(msg => conn:ConnectionType::KeepAlive, chunked:false,
                            ("upgrade", "Websocket"));
            assert_eq!(msg.websocket, true);
            expect_completed!(codec(buf));
        }}

//_comp = zlib.compressobj(wbits=-zlib.MAX_WBITS)
//_COMPRESSED = b''.join([_comp.compress(b'data'), _comp.flush()])

//def test_http_payload_parser_deflate(self):
//length = len(self._COMPRESSED)
//    out = aiohttp.FlowControlDataQueue(self.stream)
//p = HttpPayloadParser(
//out, length=length, compression='deflate')
//p.feed_data(self._COMPRESSED)
//self.assertEqual(b'data', b''.join(d for d, _ in out._buffer))
//self.assertTrue(out.is_eof())
