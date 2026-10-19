//! The documented limits, and the accessors' type checks.
#![allow(clippy::unwrap_used)]

mod common;
mod fixtures;

use common::unhex;
use rusty_protobuf::{Error, Reader, Value, WireType, Writer, DEFAULT_DEPTH_LIMIT};

/// `depth` messages nested through field 1, innermost empty.
fn nested(depth: usize) -> Vec<u8> {
    fn go(w: &mut Writer, depth: usize) {
        if depth > 0 {
            w.message(1, |w| go(w, depth - 1));
        }
    }
    let mut w = Writer::new();
    go(&mut w, depth);
    w.finish().unwrap()
}

/// How many levels of field-1 messages can be entered.
fn descend(r: Reader<'_>) -> Result<usize, Error> {
    let mut levels = 0;
    for f in r {
        levels = levels.max(1 + descend(f?.message()?)?);
    }
    Ok(levels)
}

#[test]
fn nesting_up_to_the_default_limit_is_read() {
    assert_eq!(DEFAULT_DEPTH_LIMIT, 100);
    let buf = nested(100);
    assert_eq!(descend(Reader::new(&buf)), Ok(100));
}

#[test]
fn one_level_past_the_limit_is_refused() {
    let buf = nested(101);
    assert_eq!(descend(Reader::new(&buf)), Err(Error::DepthExceeded));
}

#[test]
fn the_limit_is_configurable_and_zero_forbids_sub_messages() {
    let buf = nested(3);
    assert_eq!(descend(Reader::with_depth_limit(&buf, 3)), Ok(3));
    assert_eq!(
        descend(Reader::with_depth_limit(&buf, 2)),
        Err(Error::DepthExceeded)
    );
    let one = nested(1);
    assert_eq!(
        descend(Reader::with_depth_limit(&one, 0)),
        Err(Error::DepthExceeded)
    );
}

#[test]
fn deep_nesting_is_refused_before_a_recursive_decoder_can_exhaust_the_stack() {
    let mut buf = Vec::new();
    for _ in 0..5_000 {
        let mut next = Writer::new();
        next.bytes(1, &buf);
        buf = next.finish().unwrap();
    }
    assert_eq!(descend(Reader::new(&buf)), Err(Error::DepthExceeded));
}

fn first(buf: &[u8]) -> rusty_protobuf::Field<'_> {
    Reader::new(buf).next().unwrap().unwrap()
}

#[test]
fn varints_of_ten_bytes_that_fit_64_bits_are_read() {
    let buf = unhex("08ffffffffffffffffff01");
    assert_eq!(first(&buf).value, Value::Varint(u64::MAX));
    let f = first(&buf);
    assert_eq!(f.int64().unwrap(), -1);
    assert_eq!(f.int32().unwrap(), -1);
    assert_eq!(f.uint32().unwrap(), u32::MAX);
}

#[test]
fn wider_values_are_truncated_like_the_reference() {
    let buf = unhex("088080808010"); // 1 << 32 | 0
    assert_eq!(first(&buf).int32().unwrap(), 0);
    let buf = unhex("088180808010"); // 1 << 32 | 1
    assert_eq!(first(&buf).int32().unwrap(), 1);
    assert_eq!(first(&buf).uint32().unwrap(), 1);
}

#[test]
fn bool_is_any_non_zero_varint() {
    for (hex, want) in [
        ("0800", false),
        ("0801", true),
        ("0802", true),
        ("088001", true),
    ] {
        let buf = unhex(hex);
        assert_eq!(first(&buf).bool().unwrap(), want, "{hex}");
    }
}

#[test]
fn an_accessor_for_the_wrong_wire_type_says_so() {
    let varint = unhex("0801");
    let len = unhex("1201aa");
    let fixed32 = unhex("0d01000000");
    let fixed64 = unhex("090100000000000000");
    assert_eq!(
        first(&varint).bytes().unwrap_err(),
        Error::WrongWireType {
            expected: WireType::Len,
            found: WireType::Varint
        }
    );
    assert_eq!(
        first(&len).int32().unwrap_err(),
        Error::WrongWireType {
            expected: WireType::Varint,
            found: WireType::Len
        }
    );
    assert_eq!(
        first(&fixed32).fixed64().unwrap_err(),
        Error::WrongWireType {
            expected: WireType::Fixed64,
            found: WireType::Fixed32
        }
    );
    assert_eq!(
        first(&fixed64).float().unwrap_err(),
        Error::WrongWireType {
            expected: WireType::Fixed32,
            found: WireType::Fixed64
        }
    );
    assert!(first(&varint).message().is_err());
    assert!(first(&varint).packed_fixed32().is_err());
    assert_eq!(first(&len).wire_type(), WireType::Len);
}

#[test]
fn packed_varints_take_a_run_or_one_element() {
    let run = unhex("8a01030102ac"); // field 17, 3 bytes: 1, 2, then a truncated varint
    let items: Vec<_> = first(&run).packed_varints().unwrap().collect();
    assert_eq!(items, [Ok(1), Ok(2), Err(Error::Truncated)]);

    let single = unhex("880107");
    let items: Vec<_> = first(&single).packed_varints().unwrap().collect();
    assert_eq!(items, [Ok(7)]);
}

#[test]
fn packed_fixed_width_take_a_run_or_one_element() {
    let run = unhex("c201080100000002000000");
    assert_eq!(
        first(&run).packed_fixed32().unwrap().collect::<Vec<_>>(),
        [1, 2]
    );
    let single = unhex("c50109000000");
    assert_eq!(
        first(&single).packed_fixed32().unwrap().collect::<Vec<_>>(),
        [9]
    );
    let run = unhex("ca01100100000000000000ffffffffffffffff");
    assert_eq!(
        first(&run).packed_fixed64().unwrap().collect::<Vec<_>>(),
        [1, u64::MAX]
    );
    let single = unhex("c9010100000000000000");
    assert_eq!(
        first(&single).packed_fixed64().unwrap().collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn a_packed_fixed_run_must_be_a_whole_number_of_elements() {
    let three = unhex("c20103010203");
    assert_eq!(
        first(&three).packed_fixed32().unwrap_err(),
        Error::BadPackedLength
    );
    let seven = unhex("ca010700000000000000");
    assert_eq!(
        first(&seven).packed_fixed64().unwrap_err(),
        Error::BadPackedLength
    );
}

#[test]
fn a_truncated_fixed_value_is_refused_by_the_reader() {
    let short = unhex("c5010900");
    assert_eq!(
        Reader::new(&short).next().unwrap().unwrap_err(),
        Error::Truncated
    );
}

#[test]
fn a_string_is_checked_but_bytes_are_not() {
    let buf = unhex("7202c328");
    assert_eq!(first(&buf).str().unwrap_err(), Error::InvalidUtf8);
    assert_eq!(first(&buf).bytes().unwrap(), [0xc3, 0x28]);
}
