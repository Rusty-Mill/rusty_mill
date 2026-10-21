//! Hostile input: nothing here may panic, read outside the buffer, or be accepted when the
//! reference rejects it.
#![allow(clippy::unwrap_used)]

mod common;
mod fixtures;

use common::{unhex, All};
use fixtures as fx;
use rusty_protobuf::{Error, Reader};

const GOOD: &[&str] = &[
    fx::SCALARS,
    fx::STRINGS,
    fx::NESTED,
    fx::REPEATED,
    fx::MAP_TWO_SORTED,
    fx::ONEOF_STR,
    fx::EMPTY_NESTED,
    fx::FAR_FIELD,
    fx::WIDE,
    fx::UNPACKED_REPEATED,
    fx::PACKED_TWICE,
    fx::MERGED_SUBMESSAGE,
];

/// A tiny deterministic generator, so a failure reproduces without a dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

#[test]
fn the_reference_rejects_what_this_crate_rejects() {
    for (name, hex, reference_rejects) in fx::MALFORMED {
        let buf = unhex(hex);
        assert!(All::decode(&buf).is_err(), "{name} was accepted");
        if name.starts_with("strict_") {
            assert!(
                !reference_rejects,
                "{name}: strict rows are ones the reference accepts"
            );
        }
    }
}

#[test]
fn malformed_input_fails_with_the_matching_error() {
    let expect = |name: &str| {
        let hex = fx::MALFORMED.iter().find(|(n, ..)| *n == name).unwrap().1;
        All::decode(&unhex(hex)).unwrap_err()
    };
    assert_eq!(expect("truncated_varint"), Error::Truncated);
    assert_eq!(expect("truncated_tag"), Error::Truncated);
    assert_eq!(expect("len_past_end"), Error::Truncated);
    assert_eq!(expect("len_huge"), Error::Truncated);
    assert_eq!(expect("truncated_fixed32"), Error::Truncated);
    assert_eq!(expect("truncated_fixed64"), Error::Truncated);
    assert_eq!(expect("field_zero"), Error::BadTag);
    assert_eq!(expect("field_number_too_big"), Error::BadTag);
    assert_eq!(expect("wire_type_6"), Error::BadWireType(6));
    assert_eq!(expect("wire_type_7"), Error::BadWireType(7));
    assert_eq!(expect("invalid_utf8"), Error::InvalidUtf8);
    assert_eq!(expect("varint_11_bytes"), Error::VarintOverflow);
    assert_eq!(expect("strict_varint_overflow"), Error::VarintOverflow);
    assert_eq!(expect("packed_varint_truncated"), Error::Truncated);
    assert_eq!(expect("packed_fixed32_bad_len"), Error::BadPackedLength);
    assert_eq!(expect("packed_fixed64_bad_len"), Error::BadPackedLength);
    assert_eq!(expect("nested_truncated"), Error::Truncated);
    assert_eq!(expect("nested_len_past_end"), Error::Truncated);
    assert_eq!(expect("end_group_alone"), Error::Group);
    assert_eq!(expect("start_group_unterminated"), Error::Group);
    assert_eq!(expect("strict_group_well_formed"), Error::Group);
}

#[test]
fn every_prefix_of_every_recording_is_handled() {
    for hex in GOOD {
        let buf = unhex(hex);
        for end in 0..=buf.len() {
            let _ = All::decode(&buf[..end]);
        }
    }
}

#[test]
fn a_cut_inside_a_single_field_message_is_an_error_not_an_empty_message() {
    for hex in [fx::NESTED, fx::ONEOF_STR, fx::FAR_FIELD] {
        let buf = unhex(hex);
        for end in 1..buf.len() {
            assert!(
                All::decode(&buf[..end]).is_err(),
                "{hex}[..{end}] was accepted"
            );
        }
    }
}

#[test]
fn random_corruption_never_panics() {
    let mut rng = Lcg(0x5eed);
    for hex in GOOD {
        let good = unhex(hex);
        for _ in 0..4000 {
            let mut buf = good.clone();
            for _ in 0..1 + rng.next() % 3 {
                if buf.is_empty() {
                    break;
                }
                let at = rng.next() as usize % buf.len();
                match rng.next() % 4 {
                    0 => buf[at] = rng.next() as u8,
                    1 => buf[at] ^= 1 << (rng.next() % 8),
                    2 => buf.insert(at, rng.next() as u8),
                    _ => {
                        buf.remove(at);
                    }
                }
            }
            let _ = All::decode(&buf);
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(42);
    for _ in 0..20_000 {
        let len = rng.next() as usize % 48;
        let buf: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = All::decode(&buf);
    }
}

#[test]
fn a_reader_yields_nothing_after_an_error() {
    let buf = unhex("0880"); // truncated varint
    let mut r = Reader::new(&buf);
    assert_eq!(r.next().unwrap().unwrap_err(), Error::Truncated);
    assert!(r.next().is_none());
    assert!(r.next().is_none());
}

#[test]
fn a_huge_claimed_length_costs_nothing() {
    // 4 GiB claimed, 1 byte present: an error, not an allocation.
    let buf = unhex("72ffffffff0f61");
    assert_eq!(All::decode(&buf).unwrap_err(), Error::Truncated);
}
