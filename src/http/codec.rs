use std::io;
use pyo3::*;
use bytes::{Bytes, BytesMut};
use tokio_io::codec::{Encoder, Decoder};

use http;
// use pyunsafe::GIL;


pub enum EncoderMessage {
    Bytes(Bytes),
    PyBytes(PyBytes),
}


pub struct HttpTransportCodec {
    decoder: http::RequestDecoder,
}

impl HttpTransportCodec {
    pub fn new() -> HttpTransportCodec {
        HttpTransportCodec {
            decoder: http::RequestDecoder::new(),
        }
    }
}

impl Decoder for HttpTransportCodec {
    type Item = http::RequestMessage;
    type Error = http::Error;

    #[inline]
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }

}

impl Encoder for HttpTransportCodec {
    type Item = EncoderMessage;
    type Error = io::Error;

    fn encode(&mut self, msg: EncoderMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match msg {
            EncoderMessage::Bytes(bytes) => {
                dst.extend(bytes);
            },
            EncoderMessage::PyBytes(bytes) => {
                dst.extend(bytes.data());
            },
        }
        Ok(())
    }

}
