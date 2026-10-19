//! The writer's framing rules, independent of the recordings.
#![allow(clippy::unwrap_used)]

mod common;

use common::unhex;
use rusty_protobuf::{Error, Reader, Value, Writer, MAX_FIELD_NUMBER};

fn written(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    f(&mut w);
    w.finish().unwrap()
}

#[test]
fn the_specs_own_examples() {
    // developers.google.com/protocol-buffers/docs/encoding
    assert_eq!(written(|w| w.uint64(1, 150)), unhex("089601"));
    assert_eq!(
        written(|w| w.string(2, "testing")),
        unhex("120774657374696e67")
    );
    assert_eq!(
        written(|w| w.message(3, |w| w.uint64(1, 150))),
        unhex("1a03089601")
    );
}

#[test]
fn a_tag_with_a_large_field_number_takes_more_bytes() {
    assert_eq!(written(|w| w.uint64(16, 1)), unhex("800101"));
    assert_eq!(
        written(|w| w.uint64(MAX_FIELD_NUMBER, 1)),
        unhex("f8ffffff0f01")
    );
}

#[test]
fn a_sub_message_over_127_bytes_gets_a_two_byte_prefix() {
    let body = vec![7u8; 200];
    let buf = written(|w| w.message(1, |w| w.bytes(1, &body)));
    // inner: tag(1) + len(2 bytes) + 200 = 203 = 0xcb 0x01
    assert_eq!(&buf[..3], unhex("0acb01"));
    let outer = Reader::new(&buf).next().unwrap().unwrap();
    let inner = outer.message().unwrap().next().unwrap().unwrap();
    assert_eq!(inner.bytes().unwrap(), &body[..]);
}

#[test]
fn the_length_fixup_is_exact_at_the_one_to_two_byte_boundary() {
    for body_len in [124usize, 125, 126, 127, 128, 129] {
        // message payload = tag(1) + len prefix (1 or 2) + body_len
        let body = vec![1u8; body_len];
        let buf = written(|w| w.message(1, |w| w.bytes(1, &body)));
        let outer = Reader::new(&buf).next().unwrap().unwrap();
        let Value::Len(payload) = outer.value else {
            panic!("not a message")
        };
        let inner = Reader::new(payload).next().unwrap().unwrap();
        assert_eq!(inner.bytes().unwrap().len(), body_len);
        assert_eq!(Reader::new(payload).count(), 1);
    }
}

#[test]
fn nested_sub_messages_fix_each_level() {
    let buf = written(|w| {
        w.message(1, |w| {
            w.message(2, |w| {
                w.bytes(3, &[9; 300]);
            });
            w.uint32(4, 5);
        })
    });
    let l1 = Reader::new(&buf)
        .next()
        .unwrap()
        .unwrap()
        .message()
        .unwrap();
    let fields: Vec<_> = l1.map(|f| f.unwrap()).collect();
    assert_eq!(fields.len(), 2);
    let l2 = fields[0].message().unwrap().next().unwrap().unwrap();
    assert_eq!((l2.number, l2.bytes().unwrap().len()), (3, 300));
    assert_eq!(fields[1].uint32().unwrap(), 5);
}

#[test]
fn empty_messages_and_empty_runs_are_a_tag_and_a_zero() {
    assert_eq!(written(|w| w.message(1, |_| {})), unhex("0a00"));
    assert_eq!(written(|w| w.packed_varints(1, [])), unhex("0a00"));
    assert_eq!(written(|w| w.packed_fixed32(1, [])), unhex("0a00"));
    assert_eq!(written(|w| w.packed_fixed64(1, [])), unhex("0a00"));
}

#[test]
fn packed_runs() {
    assert_eq!(
        written(|w| w.packed_varints(4, [3, 270, 86942])),
        unhex("2206038e029ea705")
    );
    assert_eq!(
        written(|w| w.packed_fixed32(1, [1, 2])),
        unhex("0a080100000002000000")
    );
    assert_eq!(
        written(|w| w.packed_fixed64(1, [1])),
        unhex("0a080100000000000000")
    );
}

#[test]
fn scalars_are_little_endian_and_negatives_are_sign_extended() {
    assert_eq!(written(|w| w.fixed32(1, 1)), unhex("0d01000000"));
    assert_eq!(written(|w| w.sfixed64(1, -1)), unhex("09ffffffffffffffff"));
    assert_eq!(written(|w| w.float(1, 1.0)), unhex("0d0000803f"));
    assert_eq!(written(|w| w.double(1, 1.0)), unhex("09000000000000f03f"));
    assert_eq!(written(|w| w.int32(1, -1)), unhex("08ffffffffffffffffff01"));
    assert_eq!(written(|w| w.int64(1, -1)), unhex("08ffffffffffffffffff01"));
    assert_eq!(written(|w| w.sint32(1, -1)), unhex("0801"));
    assert_eq!(written(|w| w.sint64(1, 1)), unhex("0802"));
    assert_eq!(written(|w| w.bool(1, true)), unhex("0801"));
    assert_eq!(written(|w| w.bool(1, false)), unhex("0800"));
}

#[test]
fn defaults_are_written_when_asked_for() {
    assert_eq!(written(|w| w.uint32(1, 0)), unhex("0800"));
    assert_eq!(written(|w| w.string(1, "")), unhex("0a00"));
}

#[test]
fn an_invalid_field_number_is_reported_by_finish_and_writes_nothing() {
    for bad in [0, MAX_FIELD_NUMBER + 1, u32::MAX] {
        let mut w = Writer::new();
        w.uint64(1, 1);
        w.uint64(bad, 1);
        w.string(2, "after");
        assert_eq!(w.finish(), Err(Error::TooLarge), "field {bad}");
    }
    let mut w = Writer::new();
    w.message(0, |w| w.uint64(1, 1));
    assert_eq!(w.finish(), Err(Error::TooLarge));
}

#[test]
fn an_error_inside_a_sub_message_survives_the_fixup() {
    let mut w = Writer::new();
    w.message(1, |w| w.uint64(0, 1));
    w.uint64(2, 2);
    assert_eq!(w.finish(), Err(Error::TooLarge));
}
