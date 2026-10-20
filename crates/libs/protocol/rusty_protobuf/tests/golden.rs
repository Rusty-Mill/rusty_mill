//! The runtime against buffers recorded from the reference implementation (Google's protobuf
//! for Python, upb): decode what it wrote, and write the same bytes back.
#![allow(clippy::unwrap_used)]

mod common;
mod fixtures;

use common::{unhex, All, Inner, Oneof};
use fixtures as fx;
use rusty_protobuf::{Reader, Value, Writer};

/// Decodes `hex` as the reference's `All`, and checks that writing the result back yields
/// exactly the recorded bytes.
fn roundtrip(hex: &str) -> All {
    let buf = unhex(hex);
    let m = All::decode(&buf).unwrap();
    assert_eq!(
        m.encode().unwrap(),
        buf,
        "re-encoding differs from the recording"
    );
    m
}

#[test]
fn scalars_of_every_type_including_extremes() {
    let m = roundtrip(fx::SCALARS);
    assert_eq!(m.i32_, -1);
    assert_eq!(m.i64_, -2);
    assert_eq!(m.u32_, u32::MAX);
    assert_eq!(m.u64_, u64::MAX);
    assert_eq!(m.s32, -3);
    assert_eq!(m.s64, i64::MIN);
    assert!(m.b);
    assert_eq!(m.f32_, 0xDEAD_BEEF);
    assert_eq!(m.f64_, u64::MAX);
    assert_eq!(m.sf32, -5);
    assert_eq!(m.sf64, -6);
    assert_eq!(m.fl, 1.5);
    assert_eq!(m.db, -0.25);
}

#[test]
fn strings_and_bytes() {
    let m = roundtrip(fx::STRINGS);
    assert_eq!(m.s, "h\u{e9}llo \u{2603}");
    assert_eq!(m.by, [0x00, 0xff, 0x10]);
}

#[test]
fn nested_messages_and_an_empty_one_that_is_still_present() {
    let m = roundtrip(fx::NESTED);
    assert_eq!(
        m.inner,
        Some(Inner {
            id: 7,
            name: "x".into()
        })
    );
    let m = roundtrip(fx::EMPTY_NESTED);
    assert_eq!(m.inner, Some(Inner::default()));
}

#[test]
fn repeated_fields_packed_and_not() {
    let m = roundtrip(fx::REPEATED);
    assert_eq!(m.packed, [1, 300, -1]);
    assert_eq!(m.strs, ["a", "", "b"]);
    assert_eq!(
        m.inners,
        [
            Inner {
                id: 1,
                name: String::new()
            },
            Inner {
                id: 2,
                name: "n".into()
            }
        ]
    );
    assert_eq!(m.pf32, [1, 2]);
    assert_eq!(m.pdb, [1.0, -2.5]);
    assert_eq!(m.ps64, [-1, 1, 1 << 40]);
}

#[test]
fn maps_are_repeated_entry_messages() {
    assert_eq!(roundtrip(fx::MAP_ONE).map, [("k".to_string(), 1)]);
    assert_eq!(
        roundtrip(fx::MAP_TWO_SORTED).map,
        [("a".to_string(), 3), ("b".to_string(), -2)]
    );
}

#[test]
fn oneof_and_enum() {
    assert_eq!(roundtrip(fx::ONEOF_INT).one, Oneof::Int(5));
    assert_eq!(roundtrip(fx::ONEOF_STR).one, Oneof::Str("z".into()));
    assert_eq!(roundtrip(fx::ENUM).e, 2);
}

#[test]
fn an_empty_message_is_zero_bytes() {
    assert_eq!(roundtrip(fx::EMPTY), All::default());
}

#[test]
fn the_largest_field_number() {
    let buf = unhex(fx::FAR_FIELD);
    let f = Reader::new(&buf).next().unwrap().unwrap();
    assert_eq!((f.number, f.int32().unwrap()), (536_870_911, 1));
    let mut w = Writer::new();
    w.int32(536_870_911, 1);
    assert_eq!(w.finish().unwrap(), buf);
}

// What the reference accepts although it is not what it would write.

#[test]
fn unpacked_and_split_packed_runs_concatenate() {
    assert_eq!(
        All::decode(&unhex(fx::UNPACKED_REPEATED)).unwrap().packed,
        [1, 2, 3]
    );
    assert_eq!(
        All::decode(&unhex(fx::PACKED_TWICE)).unwrap().packed,
        [1, 2, 3]
    );
}

#[test]
fn overlong_varints_are_accepted() {
    assert_eq!(All::decode(&unhex(fx::OVERLONG_VARINT)).unwrap().i32_, 1);
}

#[test]
fn the_last_scalar_wins_and_sub_messages_merge() {
    assert_eq!(All::decode(&unhex(fx::LAST_WINS)).unwrap().i32_, 2);
    let m = All::decode(&unhex(fx::MERGED_SUBMESSAGE)).unwrap();
    assert_eq!(
        m.inner,
        Some(Inner {
            id: 1,
            name: "x".into()
        })
    );
    assert_eq!(
        All::decode(&unhex(fx::ONEOF_LAST_WINS)).unwrap().one,
        Oneof::Str("z".into())
    );
}

#[test]
fn unknown_fields_are_yielded_and_a_decoder_may_skip_them() {
    let buf = unhex(fx::WIDE);
    assert_eq!(
        Inner::decode(Reader::new(&buf)).unwrap(),
        Inner {
            id: 9,
            name: "w".into()
        }
    );
    let unknown: Vec<(u32, Value<'_>)> = Reader::new(&buf)
        .map(|f| f.unwrap())
        .filter(|f| f.number > 2)
        .map(|f| (f.number, f.value))
        .collect();
    assert_eq!(
        unknown,
        [
            (3, Value::Varint(1 << 63)),
            (4, Value::Len(b"zz")),
            (5, Value::Fixed32(7)),
            (6, Value::Fixed64(8)),
            (536_870_911, Value::Varint(3)),
        ]
    );
}
