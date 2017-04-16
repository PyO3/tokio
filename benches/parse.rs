#![feature(test)]
#![allow(dead_code)]
#![allow(unused_variables)]

//extern crate pico_sys as pico;
extern crate test;
extern crate async_tokio;
extern crate bytes;
extern crate tokio_io;
extern crate http_muncher;
extern crate httparse;

use bytes::BytesMut;
use tokio_io::codec::Decoder;
use async_tokio::http::{RequestDecoder, RequestMessage};
use http_muncher::{Parser, ParserHandler};


const REQ: &'static [u8] = b"\
GET /wp-content/uploads/2010/03/hello-kitty-darth-vader-pink.jpg HTTP/1.1\r\n\
Host: www.kittyhell.com\r\n\
User-Agent: Mozilla/5.0 (Macintosh; U; Intel Mac OS X 10.6; ja-JP-mac; rv:1.9.2.3) Gecko/20100401 Firefox/3.6.3 Pathtraq/0.9\r\n\
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n\
Accept-Language: ja,en-us;q=0.7,en;q=0.3\r\n\
Accept-Encoding: gzip,deflate\r\n\
Accept-Charset: Shift_JIS,utf-8;q=0.7,*;q=0.7\r\n\
Keep-Alive: 115\r\n\
Connection: keep-alive\r\n\
Cookie: wp_ozh_wsa_visits=2; wp_ozh_wsa_visit_lasttime=xxxxxxxxxx; __utma=xxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.xxxxxxxxxx.x; __utmz=xxxxxxxxx.xxxxxxxxxx.x.x.utmccn=(referral)|utmcsr=reader.livedoor.com|utmcct=/reader/|utmcmd=referral\r\n\r\n";


// Now let's define a new listener for parser events:
struct MyHandler;

impl ParserHandler for MyHandler {

    fn on_header_field(&mut self, parser: &mut Parser, header: &[u8]) -> bool {
        true
    }

    fn on_header_value(&mut self, parser: &mut Parser, value: &[u8]) -> bool {
        true
    }
}


#[bench]
fn bench_http_parser(b: &mut test::Bencher) {
    let mut callbacks_handler = MyHandler {};
    let mut parser = Parser::request();

    b.iter(|| {
        let buf = BytesMut::from(REQ);
        parser.parse(&mut callbacks_handler, &buf[..]);
    });
}


#[bench]
fn bench_decoder(b: &mut test::Bencher) {
    let mut decoder = RequestDecoder::new();
    b.iter(|| {
        let mut buf = BytesMut::from(REQ);
        loop {
            match decoder.decode(&mut buf) {
                Ok(Some(msg)) => {
                    match msg {
                        RequestMessage::Completed => break,
                        _ => (),
                    }
                },
                Ok(None) => assert!(false, "should not happen"),
                Err(err) => assert!(false, format!("Error: {:?}", err)),
            }
        }
    })
}


//#[bench]
fn bench_httparse(b: &mut test::Bencher) {
    let mut headers = [httparse::Header{ name: "", value: &[] }; 16];
    let mut req = httparse::Request::new(&mut headers);
    b.iter(|| {
        assert_eq!(req.parse(REQ).unwrap(), httparse::Status::Complete(REQ.len()));
    })
}
