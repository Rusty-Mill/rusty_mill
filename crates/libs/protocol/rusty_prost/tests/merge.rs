//! The format's merge rules, on hand-built buffers, each also read by `prost`: last scalar wins,
//! sub-messages merge, repeated fields append, a oneof keeps its last member, unknown fields are
//! skipped.
#![allow(clippy::unwrap_used)]

mod common;

use common::{unhex, All, Inner, One, PAll};
use prost::Message as _;
use rusty_prost::wkt::{Struct, Value};
use rusty_prost::{Message, Writer};

/// Decodes `hex` with both, checks they read the same message, and returns ours.
fn read(hex: &str) -> All {
    let buf = unhex(hex);
    let ours = All::decode(&buf).unwrap();
    let theirs = PAll::decode(buf.as_slice()).unwrap();
    assert_eq!(
        PAll::from(&ours),
        theirs,
        "{hex}: prost reads it differently"
    );
    ours
}

#[test]
fn the_last_scalar_wins() {
    assert_eq!(read("08010802").i32_, 2);
    assert_eq!(read("720161720162").s, "b");
}

#[test]
fn sub_messages_merge_field_by_field() {
    // inner{id: 1}, then inner{name: "x"}: one Inner with both
    //   82 01 02 | 08 01   82 01 03 | 12 01 78
    let m = read("8201020801820103120178");
    assert_eq!(
        m.inner,
        Some(Inner {
            id: 1,
            name: "x".into()
        })
    );
    // a later occurrence of the same field overrides it
    let m = read("82010208018201020802");
    assert_eq!(
        m.inner,
        Some(Inner {
            id: 2,
            name: String::new()
        })
    );
}

#[test]
fn repeated_fields_append_across_occurrences_and_forms() {
    let m = read("9201016192010162"); // strs: "a", "b"
    assert_eq!(m.strs, ["a", "b"]);
    // field 17 as one unpacked element, then as a packed run of two
    let m = read("8801018a01020203");
    assert_eq!(m.packed, [1, 2, 3]);
    // two inners stay two inners
    let m = read("9a010208019a01020802");
    assert_eq!(m.inners.len(), 2);
}

#[test]
fn a_oneof_keeps_its_last_member_whichever_that_is() {
    assert_eq!(read("a80105b201017a").one, Some(One::Str("z".into())));
    assert_eq!(read("b201017aa80105").one, Some(One::Int(5)));
}

#[test]
fn a_repeated_map_key_keeps_the_last_value_and_a_missing_half_is_the_default() {
    // k->1, then k->2
    let m = read("a201050a016b1001a201050a016b1002");
    assert_eq!(m.m.get("k"), Some(&2));
    // an entry with no value, and one with no key
    let m = read("a201030a016ba201021001");
    assert_eq!(m.m.get("k"), Some(&0));
    assert_eq!(m.m.get(""), Some(&1));
}

#[test]
fn unknown_fields_of_every_wire_type_are_skipped() {
    let mut w = Writer::new();
    w.int32(1, 7);
    w.uint64(99, 1);
    w.fixed64(100, 2);
    w.bytes(101, b"zz");
    w.fixed32(102, 3);
    w.message(103, |w| w.string(1, "nested"));
    w.string(14, "kept");
    let buf = w.finish().unwrap();
    let ours = All::decode(&buf).unwrap();
    assert_eq!((ours.i32_, ours.s.as_str()), (7, "kept"));
    assert_eq!(PAll::from(&ours), PAll::decode(buf.as_slice()).unwrap());
    // and what is written back leaves them out
    assert_eq!(
        ours.encode_to_vec().unwrap(),
        PAll::from(&ours).encode_to_vec()
    );
}

#[test]
fn value_members_merge_like_their_message_types() {
    // struct_value {a: 1} then struct_value {b: 2}: one struct with both
    let one = |k: &str, n: f64| {
        let mut w = Writer::new();
        w.message(1, |w| {
            w.string(1, k);
            w.message(2, |w| w.double(2, n));
        });
        w.finish().unwrap()
    };
    let mut w = Writer::new();
    w.bytes(5, &one("a", 1.0));
    w.bytes(5, &one("b", 2.0));
    let buf = w.finish().unwrap();
    let v = Value::decode(&buf).unwrap();
    let Value::Struct(Struct { fields }) = &v else {
        panic!("{v:?}")
    };
    assert_eq!(fields.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(
        v.encode_to_vec().unwrap(),
        prost_types::Value::decode(buf.as_slice())
            .unwrap()
            .encode_to_vec()
    );

    // list_value twice: the second run of values is appended
    let list = |n: f64| {
        let mut w = Writer::new();
        w.message(1, |w| w.double(2, n));
        w.finish().unwrap()
    };
    let mut w = Writer::new();
    w.bytes(6, &list(1.0));
    w.bytes(6, &list(2.0));
    let buf = w.finish().unwrap();
    assert_eq!(
        Value::decode(&buf).unwrap(),
        Value::List(vec![Value::Number(1.0), Value::Number(2.0)])
    );
    assert_eq!(
        Value::decode(&buf).unwrap().encode_to_vec().unwrap(),
        prost_types::Value::decode(buf.as_slice())
            .unwrap()
            .encode_to_vec()
    );

    // a different member replaces what was there
    let mut w = Writer::new();
    w.string(3, "text");
    w.bool(4, true);
    assert_eq!(
        Value::decode(&w.finish().unwrap()).unwrap(),
        Value::Bool(true)
    );

    // including null_value, which carries no data of its own
    let mut w = Writer::new();
    w.string(3, "text");
    w.int32(1, 0);
    let buf = w.finish().unwrap();
    assert_eq!(Value::decode(&buf).unwrap(), Value::Null);
    assert_eq!(
        Value::decode(&buf).unwrap().encode_to_vec().unwrap(),
        prost_types::Value::decode(buf.as_slice())
            .unwrap()
            .encode_to_vec()
    );
}

#[test]
fn decode_with_depth_counts_levels_below_the_message() {
    let buf = Inner {
        id: 1,
        name: "n".into(),
    }
    .encode_to_vec()
    .unwrap();
    let mut w = Writer::new();
    w.bytes(16, &buf);
    let outer = w.finish().unwrap();
    assert!(All::decode_with_depth(&outer, 1).is_ok());
    assert_eq!(
        All::decode_with_depth(&outer, 0).unwrap_err(),
        rusty_prost::Error::DepthExceeded
    );
}
