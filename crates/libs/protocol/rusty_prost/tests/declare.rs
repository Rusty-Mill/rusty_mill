//! The declaration macros (`message!`, `oneof!`, `open_enum!`) against `prost` on the same schema:
//! every field kind, written by the macro and by `prost`'s derive, must give the same bytes, and
//! each side must read the other's. Plus the merge rules and the enum helpers.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::Lcg;
use prost::Message as _;
use rusty_prost::{message, oneof, open_enum, Message, Oneof};

// ---- the schema, declared with the macros ---------------------------------------------------

open_enum! {
    /// A color.
    Color {
        Red = 0: "RED",
        Green = 1: "GREEN",
        Blue = 2: "BLUE",
    }
}

message! {
    /// A small message.
    Leaf {
        1 name: String = string,
        2 n: i32 = int32,
    }
}

mod pick {
    use super::*;

    oneof! {
        /// Three kinds of member.
        Pick {
            1 Text(String) = string,
            2 Blob(Vec<u8>) = bytes,
            3 Leaf(Leaf) = msg,
        }
    }
}

message! {
    /// Every field kind.
    Everything {
        oneof pick: pick::Pick;
        4 s: String = string,
        5 b: bool = bool,
        6 i: i32 = int32,
        7 c: i32 = enumv,
        8 os: Option<String> = opt_string,
        9 oi: Option<i32> = opt_int32,
        10 ob: Option<bool> = opt_bool,
        11 leaf: Option<Leaf> = msg,
        12 names: Vec<String> = rep_string,
        13 leaves: Vec<Leaf> = rep_msg,
        14 tags: BTreeMap<String, String> = map_str,
        15 by_name: BTreeMap<String, Leaf> = map_msg,
        16 raw: Vec<u8> = bytes,
        17 oraw: Option<Vec<u8>> = opt_bytes,
    }
}

// ---- the same schema in prost ---------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
struct PLeaf {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(int32, tag = "2")]
    n: i32,
}

mod ppick {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Pick {
        #[prost(string, tag = "1")]
        Text(String),
        #[prost(bytes = "vec", tag = "2")]
        Blob(Vec<u8>),
        #[prost(message, tag = "3")]
        Leaf(super::PLeaf),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct PEverything {
    #[prost(oneof = "ppick::Pick", tags = "1, 2, 3")]
    pick: Option<ppick::Pick>,
    #[prost(string, tag = "4")]
    s: String,
    #[prost(bool, tag = "5")]
    b: bool,
    #[prost(int32, tag = "6")]
    i: i32,
    #[prost(int32, tag = "7")]
    c: i32,
    #[prost(string, optional, tag = "8")]
    os: Option<String>,
    #[prost(int32, optional, tag = "9")]
    oi: Option<i32>,
    #[prost(bool, optional, tag = "10")]
    ob: Option<bool>,
    #[prost(message, optional, tag = "11")]
    leaf: Option<PLeaf>,
    #[prost(string, repeated, tag = "12")]
    names: Vec<String>,
    #[prost(message, repeated, tag = "13")]
    leaves: Vec<PLeaf>,
    #[prost(btree_map = "string, string", tag = "14")]
    tags: BTreeMap<String, String>,
    #[prost(btree_map = "string, message", tag = "15")]
    by_name: BTreeMap<String, PLeaf>,
    #[prost(bytes = "vec", tag = "16")]
    raw: Vec<u8>,
    #[prost(bytes = "vec", optional, tag = "17")]
    oraw: Option<Vec<u8>>,
}

fn leaf_to_p(l: &Leaf) -> PLeaf {
    PLeaf {
        name: l.name.clone(),
        n: l.n,
    }
}

fn to_p(e: &Everything) -> PEverything {
    PEverything {
        pick: e.pick.as_ref().map(|p| match p {
            pick::Pick::Text(s) => ppick::Pick::Text(s.clone()),
            pick::Pick::Blob(b) => ppick::Pick::Blob(b.clone()),
            pick::Pick::Leaf(l) => ppick::Pick::Leaf(leaf_to_p(l)),
        }),
        s: e.s.clone(),
        b: e.b,
        i: e.i,
        c: e.c,
        os: e.os.clone(),
        oi: e.oi,
        ob: e.ob,
        leaf: e.leaf.as_ref().map(leaf_to_p),
        names: e.names.clone(),
        leaves: e.leaves.iter().map(leaf_to_p).collect(),
        tags: e.tags.clone(),
        by_name: e
            .by_name
            .iter()
            .map(|(k, v)| (k.clone(), leaf_to_p(v)))
            .collect(),
        raw: e.raw.clone(),
        oraw: e.oraw.clone(),
    }
}

// ---- random values --------------------------------------------------------------------------

const WORDS: &[&str] = &["", "a", "hello", "ünïcödé ✓", "日本語", "x\0y", "task-1"];

fn word(rng: &mut Lcg) -> String {
    WORDS[rng.below(WORDS.len())].to_string()
}

/// A word that is not the default, for map keys and values: `prost` leaves a default key or
/// value out of an entry, and the reference implementation (and this crate) writes it, so
/// those entries are compared by what they decode to, not by bytes (see the last test).
fn solid(rng: &mut Lcg) -> String {
    loop {
        let w = word(rng);
        if !w.is_empty() {
            return w;
        }
    }
}

fn int(rng: &mut Lcg) -> i32 {
    [0, 1, -1, 7, 127, 128, i32::MAX, i32::MIN][rng.below(8)]
}

fn bytes(rng: &mut Lcg) -> Vec<u8> {
    (0..rng.below(6)).map(|_| rng.next() as u8).collect()
}

fn leaf(rng: &mut Lcg) -> Leaf {
    Leaf {
        name: word(rng),
        n: int(rng),
    }
}

fn random(rng: &mut Lcg) -> Everything {
    Everything {
        pick: match rng.below(4) {
            0 => None,
            1 => Some(pick::Pick::Text(word(rng))),
            2 => Some(pick::Pick::Blob(bytes(rng))),
            _ => Some(pick::Pick::Leaf(leaf(rng))),
        },
        s: word(rng),
        b: rng.chance(),
        i: int(rng),
        c: rng.below(4) as i32,
        os: rng.chance().then(|| word(rng)),
        oi: rng.chance().then(|| int(rng)),
        ob: rng.chance().then(|| rng.chance()),
        leaf: rng.chance().then(|| leaf(rng)),
        names: (0..rng.below(3)).map(|_| word(rng)).collect(),
        leaves: (0..rng.below(3)).map(|_| leaf(rng)).collect(),
        tags: (0..rng.below(3))
            .map(|_| (solid(rng), solid(rng)))
            .collect(),
        by_name: (0..rng.below(3))
            .map(|_| {
                let mut l = leaf(rng);
                l.n = l.n.max(1); // not the default message
                (solid(rng), l)
            })
            .collect(),
        raw: bytes(rng),
        oraw: rng.chance().then(|| bytes(rng)),
    }
}

// ---- the same bytes as prost ----------------------------------------------------------------

#[test]
fn every_kind_encodes_to_prosts_bytes_and_each_side_reads_the_others() {
    let mut rng = Lcg(0x5EED);
    for round in 0..3000 {
        let ours = random(&mut rng);
        let theirs = to_p(&ours);
        let bytes = ours.encode_to_vec().unwrap();
        assert_eq!(bytes, theirs.encode_to_vec(), "round {round}: {ours:?}");
        // prost reads ours, and we read prost's, to the same value.
        assert_eq!(
            PEverything::decode(bytes.as_slice()).unwrap(),
            theirs,
            "round {round}"
        );
        assert_eq!(
            Everything::decode(&theirs.encode_to_vec()).unwrap(),
            ours,
            "round {round}"
        );
    }
}

#[test]
fn the_empty_message_is_the_empty_buffer() {
    assert_eq!(
        Everything::default().encode_to_vec().unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(Everything::decode(&[]).unwrap(), Everything::default());
}

#[test]
fn optional_scalars_keep_zero_and_empty_but_implicit_ones_drop_them() {
    let e = Everything {
        os: Some(String::new()),
        oi: Some(0),
        ob: Some(false),
        oraw: Some(Vec::new()),
        ..Everything::default()
    };
    let bytes = e.encode_to_vec().unwrap();
    assert_eq!(bytes, to_p(&e).encode_to_vec());
    assert_eq!(Everything::decode(&bytes).unwrap(), e);
}

// ---- merge rules ----------------------------------------------------------------------------

/// Both sides must read `bytes` to the same value.
fn same_as_prost(bytes: &[u8]) -> Everything {
    let ours = Everything::decode(bytes).unwrap();
    assert_eq!(to_p(&ours), PEverything::decode(bytes).unwrap());
    ours
}

#[test]
fn a_scalar_repeated_takes_its_last_value() {
    // s = "first", then s = "second"
    let e = same_as_prost(&[
        0x22, 5, b'f', b'i', b'r', b's', b't', 0x22, 6, b's', b'e', b'c', b'o', b'n', b'd',
    ]);
    assert_eq!(e.s, "second");
}

#[test]
fn a_oneof_keeps_its_last_member_and_a_repeated_message_member_merges() {
    // Text("t"), then Blob([9])
    let e = same_as_prost(&[0x0a, 1, b't', 0x12, 1, 9]);
    assert_eq!(e.pick, Some(pick::Pick::Blob(vec![9])));
    // Leaf{name:"a"} then Leaf{n:5}: one Leaf{name:"a", n:5}
    let e = same_as_prost(&[0x1a, 3, 0x0a, 1, b'a', 0x1a, 2, 0x10, 5]);
    assert_eq!(
        e.pick,
        Some(pick::Pick::Leaf(Leaf {
            name: "a".into(),
            n: 5
        }))
    );
}

#[test]
fn a_message_field_repeated_merges_and_a_map_key_repeated_is_replaced() {
    // leaf{name:"a"} then leaf{n:5}
    let e = same_as_prost(&[0x5a, 3, 0x0a, 1, b'a', 0x5a, 2, 0x10, 5]);
    assert_eq!(
        e.leaf,
        Some(Leaf {
            name: "a".into(),
            n: 5
        })
    );
    // tags: {"k": "v1"} then {"k": "v2"}
    let entry = |v: u8| [0x72, 6, 0x0a, 1, b'k', 0x12, 1, v].to_vec();
    let bytes = [entry(b'1'), entry(b'2')].concat();
    let e = same_as_prost(&bytes);
    assert_eq!(e.tags.get("k").map(String::as_str), Some("2"));
}

#[test]
fn a_map_entry_with_no_key_or_no_value_uses_the_defaults() {
    // tags entry with only a value, and with only a key
    let e = same_as_prost(&[0x72, 3, 0x12, 1, b'v', 0x72, 3, 0x0a, 1, b'k']);
    assert_eq!(e.tags.get("").map(String::as_str), Some("v"));
    assert_eq!(e.tags.get("k").map(String::as_str), Some(""));
}

#[test]
fn an_unknown_field_leaves_a_oneof_that_is_set_alone() {
    // Text("t"), then field 100 (a varint), then s = "x"
    let e = same_as_prost(&[0x0a, 1, b't', 0xa0, 0x06, 1, 0x22, 1, b'x']);
    assert_eq!(e.pick, Some(pick::Pick::Text("t".into())));
    assert_eq!(e.s, "x");
}

#[test]
fn unknown_fields_are_skipped() {
    let e = same_as_prost(&[0xa0, 0x06, 1, 0x22, 1, b'x']); // field 100 varint, then s = "x"
    assert_eq!(e.s, "x");
}

// ---- the oneof trait and the enum -----------------------------------------------------------

#[test]
fn a_oneof_skips_a_field_that_is_not_its_own() {
    let mut slot = None;
    let bytes = [0x22, 1, b'x'];
    let f = rusty_prost::Reader::new(&bytes).next().unwrap().unwrap();
    <pick::Pick as Oneof>::merge_into(&mut slot, &f).unwrap();
    assert_eq!(slot, None);
}

#[test]
fn an_open_enum_has_names_and_refuses_values_it_does_not_know() {
    assert_eq!(Color::Green as i32, 1);
    assert_eq!(Color::Blue.as_str_name(), "BLUE");
    assert_eq!(Color::from_str_name("RED"), Some(Color::Red));
    assert_eq!(Color::from_str_name("red"), None);
    assert_eq!(Color::try_from(2).unwrap(), Color::Blue);
    for bad in [-1, 3, 99] {
        assert_eq!(Color::try_from(bad).unwrap_err().0, bad);
    }
    assert_eq!(
        rusty_prost::UnknownEnumValue(7).to_string(),
        "unknown enumeration value 7"
    );
    // An enum field keeps a value the schema does not know.
    let e = Everything {
        c: 99,
        ..Everything::default()
    };
    assert_eq!(
        Everything::decode(&e.encode_to_vec().unwrap()).unwrap().c,
        99
    );
}

#[test]
fn a_map_entry_with_a_default_key_or_value_differs_from_prost_in_bytes_but_not_in_meaning() {
    let mut e = Everything::default();
    e.tags.insert(String::new(), "v".into());
    e.tags.insert("k".into(), String::new());
    e.by_name.insert(String::new(), Leaf::default());
    let ours = e.encode_to_vec().unwrap();
    let theirs = to_p(&e).encode_to_vec();
    assert_ne!(
        ours, theirs,
        "prost leaves default keys and values out of an entry"
    );
    // Each reads the other's bytes to the same map.
    assert_eq!(to_p(&Everything::decode(&theirs).unwrap()), to_p(&e));
    assert_eq!(PEverything::decode(ours.as_slice()).unwrap(), to_p(&e));
}
