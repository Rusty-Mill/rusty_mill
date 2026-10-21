//! The well-known types and `google.rpc` against buffers recorded from Google's `protobuf`
//! for Python, and against `prost-types` on generated values.
#![allow(clippy::unwrap_used)]

mod common;
mod fixtures;

use std::collections::BTreeMap;
use std::fmt::Debug;

use common::{unhex, Lcg};
use fixtures as fx;
use prost::Message as _;
use rusty_prost::rpc::{ErrorInfo, Status};
use rusty_prost::wkt::{Any, Empty, Struct, Timestamp, Value};
use rusty_prost::{Error, Message, Writer};

/// Decodes the recording, and checks writing the result back yields the same bytes.
fn roundtrip<M: Message + Debug>(hex: &str) -> M {
    let buf = unhex(hex);
    let m = M::decode(&buf).unwrap();
    assert_eq!(m.encode_to_vec().unwrap(), buf, "{m:?}");
    m
}

fn obj(members: &[(&str, Value)]) -> Value {
    Value::Struct(Struct {
        fields: members
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    })
}

#[test]
fn empty_is_zero_bytes_and_skips_whatever_it_is_given() {
    assert_eq!(roundtrip::<Empty>(fx::EMPTY), Empty);
    assert_eq!(Empty::decode(&unhex("0801")).unwrap(), Empty);
}

#[test]
fn timestamps() {
    let t: Timestamp = roundtrip(fx::TIMESTAMP);
    assert_eq!((t.seconds, t.nanos), (1_700_000_000, 123_456_789));
    let t: Timestamp = roundtrip(fx::TIMESTAMP_NEGATIVE);
    assert_eq!((t.seconds, t.nanos), (-1, 999_999_999));
    assert_eq!(
        roundtrip::<Timestamp>(fx::TIMESTAMP_ZERO),
        Timestamp::default()
    );
    let t: Timestamp = roundtrip(fx::TIMESTAMP_SECONDS_ONLY);
    assert_eq!((t.seconds, t.nanos), (60, 0));
}

#[test]
fn every_value_kind() {
    assert_eq!(roundtrip::<Value>(fx::VALUE_NULL), Value::Null);
    assert_eq!(roundtrip::<Value>(fx::VALUE_NUMBER), Value::Number(-1.5));
    assert_eq!(
        roundtrip::<Value>(fx::VALUE_STRING),
        Value::String("h\u{e9}llo".into())
    );
    assert_eq!(roundtrip::<Value>(fx::VALUE_TRUE), Value::Bool(true));
    assert_eq!(
        roundtrip::<Value>(fx::VALUE_LIST),
        Value::List(vec![
            Value::Number(1.0),
            Value::String("a".into()),
            Value::Null,
            Value::List(vec![Value::Bool(true)]),
        ])
    );
    assert_eq!(
        roundtrip::<Value>(fx::VALUE_STRUCT),
        obj(&[("k", Value::Number(2.0))])
    );
}

#[test]
fn a_oneof_member_is_written_even_when_it_is_zero() {
    // `Number(0.0)` and `Bool(false)` are explicit on the wire: a kind that is set is not a
    // default, unlike a plain proto3 scalar.
    assert_eq!(
        roundtrip::<Value>(fx::VALUE_ZERO_NUMBER),
        Value::Number(0.0)
    );
    assert_eq!(roundtrip::<Value>(fx::VALUE_FALSE), Value::Bool(false));
}

#[test]
fn a_value_that_sets_no_kind_is_null() {
    let buf = unhex(fx::VALUE_UNSET);
    let v = Value::decode(&buf).unwrap();
    assert_eq!(v, Value::Null);
    // Null is written as a kind, so this one does not round-trip byte for byte.
    assert_eq!(v.encode_to_vec().unwrap(), unhex(fx::VALUE_NULL));
}

#[test]
fn structs() {
    let want = obj(&[
        ("name", Value::String("x".into())),
        ("n", Value::Number(1.5)),
        ("ok", Value::Bool(true)),
        ("nil", Value::Null),
        (
            "list",
            Value::List(vec![
                Value::Number(1.0),
                Value::String("a".into()),
                obj(&[("k", Value::Bool(false))]),
            ]),
        ),
        ("obj", obj(&[("a", obj(&[("b", Value::List(vec![]))]))])),
        ("", Value::Number(0.0)),
    ]);
    let Value::Struct(want) = want else {
        unreachable!()
    };
    // This recording has keys that are prefixes of each other (`n`, `name`, `nil`) and an empty
    // key. Map order is not part of the meaning, and the reference (upb) puts a key after the
    // keys it is a prefix of, so read it back and compare by value, not by bytes.
    let got = Struct::decode(&unhex(fx::STRUCT)).unwrap();
    assert_eq!(got, want);
    // ...and a value written by us reads back to itself, with the empty key and zero value
    // written in full as the reference writes them.
    assert_eq!(Struct::decode(&got.encode_to_vec().unwrap()).unwrap(), want);
    let entry_for_empty_key = [
        0x0a, 0x0d, 0x0a, 0x00, 0x12, 0x09, 0x11, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert!(got
        .encode_to_vec()
        .unwrap()
        .windows(15)
        .any(|w| w == entry_for_empty_key));
    assert_eq!(roundtrip::<Struct>(fx::STRUCT_EMPTY), Struct::default());
}

#[test]
fn a_struct_whose_keys_are_not_prefixes_matches_the_reference_bytes() {
    let Value::Struct(want) = obj(&[
        ("alpha", Value::Number(1.0)),
        ("beta", Value::String("x".into())),
        ("gamma", Value::List(vec![Value::Bool(true), Value::Null])),
    ]) else {
        unreachable!()
    };
    assert_eq!(roundtrip::<Struct>(fx::STRUCT_PLAIN), want);
}

#[test]
fn any_packs_and_unpacks_by_type_url() {
    let a: Any = roundtrip(fx::ANY);
    assert_eq!(a.type_url, "type.googleapis.com/test.Inner");
    assert_eq!(a.value, [0x08, 0x07]);
    assert_eq!(roundtrip::<Any>(fx::ANY_EMPTY), Any::default());

    let info = ErrorInfo {
        reason: "R".into(),
        domain: "d".into(),
        metadata: BTreeMap::new(),
    };
    let packed = Any::pack(&info).unwrap();
    assert_eq!(packed.type_url, "type.googleapis.com/google.rpc.ErrorInfo");
    assert_eq!(packed.unpack::<ErrorInfo>().unwrap(), Some(info));
    assert_eq!(
        packed.unpack::<Status>().unwrap(),
        None,
        "another type's URL is not an error"
    );
}

#[test]
fn unpacking_a_corrupt_payload_is_an_error() {
    let a = Any {
        type_url: "type.googleapis.com/google.rpc.ErrorInfo".into(),
        value: vec![0x0a, 0x09],
    };
    assert_eq!(a.unpack::<ErrorInfo>().unwrap_err(), Error::Truncated);
}

fn task_not_found() -> ErrorInfo {
    ErrorInfo {
        reason: "TASK_NOT_FOUND".into(),
        domain: "a2a-protocol.org".into(),
        metadata: [("task", "t-1"), ("attempt", "2")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn error_info() {
    assert_eq!(roundtrip::<ErrorInfo>(fx::ERROR_INFO), task_not_found());
    let e: ErrorInfo = roundtrip(fx::ERROR_INFO_NO_METADATA);
    assert_eq!((e.reason.as_str(), e.domain.as_str()), ("R", "d"));
    assert!(e.metadata.is_empty());
}

#[test]
fn status_carries_an_error_info_in_an_any() {
    let s: Status = roundtrip(fx::STATUS);
    assert_eq!((s.code, s.message.as_str()), (5, "task not found"));
    assert_eq!(s.details.len(), 1);
    assert_eq!(
        s.details[0].unpack::<ErrorInfo>().unwrap(),
        Some(task_not_found())
    );

    let s: Status = roundtrip(fx::STATUS_NO_DETAILS);
    assert_eq!((s.code, s.message.as_str(), s.details.len()), (3, "bad", 0));

    let s: Status = roundtrip(fx::STATUS_TWO_DETAILS);
    assert_eq!(s.details.len(), 2);
    assert_eq!(
        s.details[1],
        Any::default(),
        "an empty Any is still an element"
    );
}

// ---- nesting ----

/// A list nested `wraps` times: each wrap is a `Value` holding a `ListValue` holding the
/// `Value` before it, so two reader levels per wrap.
fn nested_list(wraps: usize) -> Vec<u8> {
    let mut v = unhex(fx::VALUE_NULL);
    for _ in 0..wraps {
        let mut list = Writer::new();
        list.bytes(1, &v);
        let mut value = Writer::new();
        value.bytes(6, &list.finish().unwrap());
        v = value.finish().unwrap();
    }
    v
}

#[test]
fn value_nesting_is_bounded_by_the_depth_limit() {
    assert!(Value::decode(&nested_list(40)).is_ok());
    assert_eq!(
        Value::decode(&nested_list(60)).unwrap_err(),
        Error::DepthExceeded
    );
    // 5,000 levels would overflow a recursive decoder's stack without the limit.
    assert_eq!(
        Value::decode(&nested_list(5_000)).unwrap_err(),
        Error::DepthExceeded
    );
}

// ---- against prost-types ----

fn to_prost(v: &Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        Value::Null => Kind::NullValue(0),
        Value::Number(n) => Kind::NumberValue(*n),
        Value::String(s) => Kind::StringValue(s.clone()),
        Value::Bool(b) => Kind::BoolValue(*b),
        Value::Struct(s) => Kind::StructValue(struct_to_prost(s)),
        Value::List(items) => Kind::ListValue(prost_types::ListValue {
            values: items.iter().map(to_prost).collect(),
        }),
    };
    prost_types::Value { kind: Some(kind) }
}

fn struct_to_prost(s: &Struct) -> prost_types::Struct {
    prost_types::Struct {
        fields: s
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), to_prost(v)))
            .collect(),
    }
}

fn random_value(rng: &mut Lcg, depth: usize) -> Value {
    match rng.below(if depth == 0 { 4 } else { 6 }) {
        0 => Value::Null,
        1 => Value::Number((rng.next() as i32) as f64 / 8.0),
        2 => Value::String(["a", "h\u{e9}llo", "", "x".repeat(130).as_str()][rng.below(4)].into()),
        3 => Value::Bool(rng.chance()),
        4 => Value::List(
            (0..rng.below(4))
                .map(|_| random_value(rng, depth - 1))
                .collect(),
        ),
        _ => Value::Struct(random_struct(rng, depth - 1)),
    }
}

fn random_struct(rng: &mut Lcg, depth: usize) -> Struct {
    // Keys are non-empty: the reference writes an empty key in full and prost drops it.
    Struct {
        fields: (0..rng.below(4))
            .map(|_| (format!("k{}", rng.below(30)), random_value(rng, depth)))
            .collect(),
    }
}

#[test]
fn struct_and_value_bytes_match_prost_types() {
    let mut rng = Lcg(7);
    for i in 0..1500 {
        let v = random_value(&mut rng, 4);
        assert_eq!(
            v.encode_to_vec().unwrap(),
            to_prost(&v).encode_to_vec(),
            "value {i}: {v:?}"
        );
        let s = random_struct(&mut rng, 4);
        let theirs = struct_to_prost(&s).encode_to_vec();
        assert_eq!(s.encode_to_vec().unwrap(), theirs, "struct {i}: {s:?}");
        assert_eq!(Struct::decode(&theirs).unwrap(), s, "struct {i} read back");
    }
}

#[test]
fn timestamp_and_any_bytes_match_prost_types() {
    let mut rng = Lcg(8);
    for _ in 0..1000 {
        let ts = Timestamp {
            seconds: rng.next() as i64 - (1 << 30),
            nanos: rng.below(1_000_000_000) as i32,
        };
        let theirs = prost_types::Timestamp {
            seconds: ts.seconds,
            nanos: ts.nanos,
        };
        assert_eq!(ts.encode_to_vec().unwrap(), theirs.encode_to_vec());

        let any = Any {
            type_url: ["", "type.googleapis.com/x"][rng.below(2)].into(),
            value: (0..rng.below(150)).map(|_| rng.next() as u8).collect(),
        };
        let theirs = prost_types::Any {
            type_url: any.type_url.clone(),
            value: any.value.clone(),
        };
        assert_eq!(any.encode_to_vec().unwrap(), theirs.encode_to_vec());
    }
}

// ---- Any identity (review findings on 49830e7e) ----

fn any_with(url: &str) -> Any {
    Any {
        type_url: url.into(),
        value: ErrorInfo {
            reason: "R".into(),
            ..Default::default()
        }
        .encode_to_vec()
        .unwrap(),
    }
}

#[test]
fn any_matches_the_final_path_segment_whatever_the_prefix() {
    for url in [
        "type.googleapis.com/google.rpc.ErrorInfo",
        "example.com/google.rpc.ErrorInfo",
        "/google.rpc.ErrorInfo",
        "a/b/c/google.rpc.ErrorInfo",
    ] {
        let got = any_with(url).unpack::<ErrorInfo>().unwrap();
        assert_eq!(got.map(|e| e.reason), Some("R".to_string()), "{url}");
    }
}

#[test]
fn any_with_another_final_name_is_not_this_type() {
    for url in [
        "type.googleapis.com/google.rpc.Status",
        "type.googleapis.com/google.rpc.errorinfo",
        "type.googleapis.com/x.google.rpc.ErrorInfo",
        "type.googleapis.com/google.rpc.ErrorInfo.Extra",
    ] {
        assert_eq!(any_with(url).unpack::<ErrorInfo>().unwrap(), None, "{url}");
    }
}

#[test]
fn a_malformed_type_url_names_no_type() {
    // no slash, a trailing slash, an empty name, a name with a leading dot
    for url in [
        "",
        "google.rpc.ErrorInfo",
        "type.googleapis.com/",
        "type.googleapis.com/.google.rpc.ErrorInfo",
    ] {
        assert_eq!(
            any_with(url).unpack::<ErrorInfo>().unwrap(),
            None,
            "{url:?}"
        );
    }
}

#[test]
fn the_built_in_types_can_be_packed_in_an_any() {
    let ts = Timestamp {
        seconds: 5,
        nanos: 6,
    };
    let a = Any::pack(&ts).unwrap();
    assert_eq!(a.type_url, "type.googleapis.com/google.protobuf.Timestamp");
    assert_eq!(a.unpack::<Timestamp>().unwrap(), Some(ts));
    for (url, got) in [
        (
            "type.googleapis.com/google.protobuf.Empty",
            Any::pack(&Empty).unwrap().type_url,
        ),
        (
            "type.googleapis.com/google.protobuf.Any",
            Any::pack(&Any::default()).unwrap().type_url,
        ),
        (
            "type.googleapis.com/google.protobuf.Struct",
            Any::pack(&Struct::default()).unwrap().type_url,
        ),
        (
            "type.googleapis.com/google.protobuf.Value",
            Any::pack(&Value::Null).unwrap().type_url,
        ),
    ] {
        assert_eq!(got, url);
    }
    // and prost-types agrees with the names
    use prost::Name as _;
    assert_eq!(
        prost_types::Timestamp::type_url(),
        "type.googleapis.com/google.protobuf.Timestamp"
    );
}
