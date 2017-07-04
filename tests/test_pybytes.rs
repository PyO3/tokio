#![allow(dead_code, unused_variables)]

extern crate bytes;
extern crate pyo3;
extern crate async_tokio;

use pyo3::*;
use bytes::{Bytes, BytesMut};
use async_tokio::{PyBytes, PyLogger};


macro_rules! py_run {
    ($py:expr, $val:ident, $code:expr) => {{
        let d = PyDict::new($py);
        d.set_item(stringify!($val), &$val).unwrap();
        $py.run($code, None, Some(&d)).expect($code);
    }}
}

macro_rules! py_assert {
    ($py:expr, $val:ident, $assertion:expr) => { py_run!($py, $val, concat!("assert ", $assertion)) };
}


#[test]
fn test_pybytes() {
    let gil = Python::acquire_gil();
    let py = gil.python();

    let bytestp = py.get_type::<PyBytes>();
    // creating instances from python is not allowed
    assert!(bytestp.call(NoArgs, None).is_err());

    let bytes = Bytes::from("{\"test\": \"value\"}");
    let pb = PyBytes::new(py, bytes).unwrap();
    assert_eq!(pb.as_ref(py).len(), 17);

    let d = PyDict::new(py);
    d.set_item("pb", pb.clone_ref(py)).unwrap();
    py.run("assert len(pb) == 17", None, Some(&d)).unwrap();
    py.run("assert bytes(pb) == b'{\"test\": \"value\"}'", None, Some(&d)).unwrap();

    py.run("assert str(pb, encoding=\"utf-8\") == '{\"test\": \"value\"}'",
           None, Some(&d)).unwrap();
    py.run("assert memoryview(pb) == b'{\"test\": \"value\"}'",
           None, Some(&d)).unwrap();

    // Fix in https://github.com/python/cpython/pull/1334
    //py.run("import json; assert json.loads(pb) == {\"test\": \"value\"}",
    //  None, Some(&d)).unwrap();

    py.run("assert str(pb[2:6], encoding=\"utf-8\") == 'test'", None, Some(&d))
        .log_error(py, "assert error").unwrap();
    py.run("assert str(pb[2:6:2], encoding=\"utf-8\") == 'ts'", None, Some(&d))
        .log_error(py, "assert error").unwrap();
    py.run("assert isinstance(pb[2], int)", None, Some(&d))
        .log_error(py, "assert error").unwrap();

    let mut buf = BytesMut::with_capacity(24);
    pb.as_ref(py).extend_into(&mut buf);
    assert_eq!(&buf[..], b"{\"test\": \"value\"}");

    py.run("b=bytearray(); b.extend(pb); assert b == b'{\"test\": \"value\"}'",
           None, Some(&d)).unwrap();
}

#[test]
fn test_pybytes_split() {
    let gil = Python::acquire_gil();
    let py = gil.python();

    // split
    let bytes = Bytes::from("1,2,,3,");
    let pb = PyBytes::new(py, bytes).unwrap();
    let d = PyDict::new(py);
    d.set_item("pb", pb.clone_ref(py)).unwrap();

    py.run("assert [bytes(i) for i in pb.split(b',')] == [b'1', b'2', b'', b'3', b'']",
           None, Some(&d)).unwrap();
    py.run("assert [bytes(i) for i in pb.split(b',', 0)] == [b'1,2,,3,']",
           None, Some(&d)).unwrap();
    py.run("assert [bytes(i) for i in pb.split(b',', 1)] == [b'1', b'2,,3,']",
           None, Some(&d)).unwrap();
    py.run("assert [bytes(i) for i in pb.split(b',', 2)] == [b'1', b'2', b',3,']",
           None, Some(&d)).unwrap();

    let bytes = Bytes::from("1 2 3");
    let pb = PyBytes::new(py, bytes).unwrap();
    d.set_item("pb", pb.clone_ref(py)).unwrap();

    py.run("assert [bytes(i) for i in pb.split()] == [b'1', b'2', b'3']",
           None, Some(&d)).unwrap();
    py.run("assert [bytes(i) for i in pb.split(maxsplit=1)] == [b'1', b'2 3']",
           None, Some(&d)).unwrap();

    let bytes = Bytes::from("   1  \t 2  \r 3   ");
    let pb = PyBytes::new(py, bytes).unwrap();
    d.set_item("pb", pb.clone_ref(py)).unwrap();

    py.run("assert [bytes(i) for i in pb.split()] == [b'1', b'2', b'3']",
           None, Some(&d)).unwrap();
}

#[test]
fn test_pybytes_strip() {
    let gil = Python::acquire_gil();
    let py = gil.python();

    let bytes = Bytes::from("   1   2   3   ");
    let pb = PyBytes::new(py, bytes).unwrap();
    let d = PyDict::new(py);
    d.set_item("pb", pb.clone_ref(py)).unwrap();

    let _ = py.run("assert pb.strip() == b'1   2   3'", None, Some(&d)).map_err(|e| e.print(py));
    let _ = py.run("assert pb.strip(b' 1') == b'2   3'", None, Some(&d)).unwrap();
}
