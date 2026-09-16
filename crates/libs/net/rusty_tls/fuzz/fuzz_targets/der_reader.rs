//! Coverage-guided fuzzing of the DER reader.
//!
//! The assertion is not merely "does not panic". It is canonicality: if the
//! reader accepts a value, re-encoding that value's tag and contents with a
//! minimal length must reproduce the accepted bytes exactly. That is the
//! whole promise of `der.rs` as a property libFuzzer can attack directly —
//! any non-minimal length, indefinite length, or high-tag-number form that
//! slipped through would produce an `encoded` longer than its canonical form
//! and fail here.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::der::{Reader, Tag};

/// Minimal-length DER encoding — the canonical form.
fn canonical_encode(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = contents.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let first = bytes.iter().position(|&b| b != 0).unwrap();
        out.push(0x80 | (bytes.len() - first) as u8);
        out.extend_from_slice(&bytes[first..]);
    }
    out.extend_from_slice(contents);
    out
}

fn within(inner: &[u8], outer: &[u8]) -> bool {
    let (i, o) = (inner.as_ptr_range(), outer.as_ptr_range());
    i.start >= o.start && i.end <= o.end
}

fuzz_target!(|data: &[u8]| {
    // Iterative, with a work stack: the reader is non-recursive by design and
    // a recursive harness would blow the stack on nested input and blame the
    // library for it.
    let mut work: Vec<&[u8]> = vec![data];
    let mut values = 0usize;

    while let Some(region) = work.pop() {
        let mut reader = Reader::new(region);
        while !reader.is_empty() {
            let before = reader.remaining();
            let Ok(value) = reader.read_any() else {
                break;
            };
            values += 1;

            assert!(within(value.encoded, data));
            assert!(within(value.contents, value.encoded));
            assert!(value.encoded.len() > value.contents.len());
            assert_eq!(before - reader.remaining(), value.encoded.len());
            assert_eq!(
                canonical_encode(value.tag.0, value.contents),
                value.encoded,
                "a non-canonical encoding was accepted"
            );

            if value.tag.is_constructed() && values < 100_000 {
                work.push(value.contents);
            }
        }
    }

    // The typed readers must never panic, and must return the shapes they
    // promise.
    if let Ok(magnitude) = Reader::new(data).read_unsigned_integer() {
        assert!(!magnitude.is_empty());
        assert!(magnitude == [0] || magnitude[0] != 0);
    }
    if let Ok(oid) = Reader::new(data).read_oid() {
        let bytes = oid.as_bytes();
        assert!(!bytes.is_empty());
        assert_eq!(bytes[bytes.len() - 1] & 0x80, 0);
        let _ = format!("{oid:?}");
    }
    if let Ok((_, unused)) = Reader::new(data).read_bit_string_flags() {
        assert!(unused <= 7);
    }
    let _ = Reader::new(data).read_bool();
    let _ = Reader::new(data).read_u64();
    let _ = Reader::new(data).read_null();
    let _ = Reader::new(data).read_bit_string_octets();
    let _ = Reader::new(data).read(Tag::SEQUENCE);
    let _ = Reader::new(data).read_optional(Tag::BOOLEAN);
});
