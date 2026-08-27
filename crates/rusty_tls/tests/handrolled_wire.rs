//! The TLS presentation-language reader and writer — stage 3b.
//!
//! # Why this file exists
//!
//! Mutation testing stage 3b found one live gap: deleting the length-overrun
//! check from `wire::Reader` left all eighteen handshake tests passing. That
//! was a fair result. Every message-level test asserted *that* a malformed
//! input was refused, and the deleted check does not change whether anything
//! is refused — only what the refusal is called.
//!
//! Which raises the honest question of whether the check is worth having. It
//! is, for one reason: `LengthOverrun` and `UnexpectedEnd` describe different
//! failures, and a caller that cannot tell them apart cannot tell a truncated
//! stream from a lying one. A truncated stream is a network event and might be
//! worth waiting on; a length prefix claiming more than its container holds is
//! a peer sending a contradiction, and there is nothing to wait for. Anything
//! that reports one as the other reports a hostile peer as a flaky link.
//!
//! So the check stays, and these tests pin the distinction it exists to make —
//! which is what the handshake suite could not do, because by the time an
//! error reaches message level the two look alike.
//!
//! # What is *not* claimed here
//!
//! The overrun check is not what keeps a reader inside its buffer. `take` is
//! bounds-checked on its own, so removing the overrun check cannot let a read
//! escape — it can only mislabel the error. The memory-safety property is
//! `take`'s, and [`a_sub_reader_cannot_escape_its_vector`] pins that
//! separately rather than letting one test appear to cover both.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::wire::{Reader, WireError, Writer};

// ---------------------------------------------------------------------------
// The distinction M8 escaped: overrun versus running out
// ---------------------------------------------------------------------------

/// A length prefix claiming more than its container holds is an overrun, and
/// the error says so — with the numbers that make it diagnosable.
#[test]
fn a_length_prefix_larger_than_its_container_is_named_an_overrun() {
    // uint8 prefix: claims 5, two octets follow.
    let mut reader = Reader::new(&[0x05, 0xaa, 0xbb]);
    assert_eq!(
        reader.vector_u8(),
        Err(WireError::LengthOverrun {
            declared: 5,
            available: 2
        })
    );

    // uint16 prefix: claims 0x0100, one octet follows.
    let mut reader = Reader::new(&[0x01, 0x00, 0xaa]);
    assert_eq!(
        reader.vector_u16(),
        Err(WireError::LengthOverrun {
            declared: 256,
            available: 1
        })
    );

    // uint24 prefix: claims most of 16MiB from a four-octet buffer. This must
    // be refused by arithmetic, never by trying.
    let mut reader = Reader::new(&[0xff, 0xff, 0xff, 0xaa]);
    assert_eq!(
        reader.vector_u24(),
        Err(WireError::LengthOverrun {
            declared: 0x00ff_ffff,
            available: 1
        })
    );
}

/// Running out mid-value is a different error, because it is a different
/// event: nothing lied, the bytes simply were not there.
#[test]
fn running_out_mid_value_is_an_unexpected_end_not_an_overrun() {
    assert_eq!(
        Reader::new(&[]).u8(),
        Err(WireError::UnexpectedEnd {
            needed: 1,
            available: 0
        })
    );
    assert_eq!(
        Reader::new(&[0x03]).u16(),
        Err(WireError::UnexpectedEnd {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(
        Reader::new(&[0x03, 0x04]).u24(),
        Err(WireError::UnexpectedEnd {
            needed: 3,
            available: 2
        })
    );
    assert_eq!(
        Reader::new(&[0x03, 0x04, 0x05]).u32(),
        Err(WireError::UnexpectedEnd {
            needed: 4,
            available: 3
        })
    );

    // A vector whose *prefix* is truncated ends unexpectedly; a vector whose
    // prefix is intact and lies overruns. One byte apart on the wire, and the
    // errors must not blur.
    assert!(matches!(
        Reader::new(&[0x00]).vector_u16(),
        Err(WireError::UnexpectedEnd { .. })
    ));
    assert!(matches!(
        Reader::new(&[0x00, 0x01]).vector_u16(),
        Err(WireError::LengthOverrun { .. })
    ));
}

/// The two errors must never be equal, and must render differently — a caller
/// that distinguishes them in code and a human reading a log both depend on
/// it.
#[test]
fn the_two_length_failures_are_distinguishable() {
    let overrun = WireError::LengthOverrun {
        declared: 5,
        available: 2,
    };
    let end = WireError::UnexpectedEnd {
        needed: 5,
        available: 2,
    };

    assert_ne!(overrun, end);
    assert_ne!(overrun.to_string(), end.to_string());
    assert!(
        overrun.to_string().contains("declares"),
        "an overrun should read as a claim, not a shortage: {overrun}"
    );
}

// ---------------------------------------------------------------------------
// Nesting
// ---------------------------------------------------------------------------

/// A sub-reader is bounded by its vector even when the buffer behind it has
/// more to give.
///
/// This is the memory-safety half of the design, and it is `take`'s doing
/// rather than the overrun check's — see the module docs on why the two are
/// pinned by separate tests.
#[test]
fn a_sub_reader_cannot_escape_its_vector() {
    // A two-octet vector followed by four octets that are none of its
    // business.
    let input = [0x00, 0x02, 0xaa, 0xbb, 0xde, 0xad, 0xbe, 0xef];
    let mut outer = Reader::new(&input);
    let mut inner = outer.sub_u16().expect("the vector is well-formed");

    assert_eq!(inner.remaining(), 2);
    assert_eq!(inner.u16(), Ok(0xaabb));
    assert!(inner.is_empty());
    assert!(
        inner.u8().is_err(),
        "a sub-reader read past the vector that bounds it"
    );
    inner.finish().expect("the vector was consumed exactly");

    // The outer reader still sees everything after the vector, untouched.
    assert_eq!(outer.remaining(), 4);
    assert_eq!(outer.u32(), Ok(0xdead_beef));
}

/// Two levels of nesting: an inner vector overrunning the outer one is caught
/// against the *outer vector's* bound, not the buffer's.
#[test]
fn an_inner_vector_is_bounded_by_the_outer_vector_not_the_buffer() {
    // outer vector = 3 octets: an inner uint16 prefix claiming 8, plus one
    // octet. Eight octets do exist in the buffer — after the outer vector.
    let input = [
        0x00, 0x03, // outer length 3
        0x00, 0x08, // inner length 8 — a lie inside a 3-octet container
        0xaa, // the one octet the outer vector actually holds
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // out of bounds
    ];

    let mut outer = Reader::new(&input);
    let mut inner = outer.sub_u16().expect("the outer vector is well-formed");
    assert_eq!(
        inner.vector_u16(),
        Err(WireError::LengthOverrun {
            declared: 8,
            available: 1
        }),
        "an inner vector was measured against the buffer rather than its container"
    );
}

/// Nesting in the input must not become nesting on the stack. A thousand
/// levels of vector is a legal-looking message and must not be a crash.
#[test]
fn deep_nesting_does_not_exhaust_the_stack() {
    const DEPTH: usize = 5_000;

    // Build DEPTH nested uint16-prefixed vectors around a single octet.
    let mut payload = vec![0xaau8];
    for _ in 0..DEPTH {
        let mut writer = Writer::new();
        writer.vector_u16(|w| w.bytes(&payload));
        payload = writer.into_vec();
    }

    // Unwrap them with a loop, which is all the reader ever offers.
    let mut reader = Reader::new(&payload);
    for level in 0..DEPTH {
        reader = reader
            .sub_u16()
            .unwrap_or_else(|e| panic!("level {level} failed: {e}"));
    }
    assert_eq!(reader.u8(), Ok(0xaa));
    reader.finish().expect("exactly consumed");
}

// ---------------------------------------------------------------------------
// Exact consumption
// ---------------------------------------------------------------------------

/// `finish` is what makes a length-prefixed parse mean anything: without it a
/// structure longer than its fields parses "successfully" while the extra
/// bytes go unexamined.
#[test]
fn finish_refuses_leftover_bytes() {
    let mut reader = Reader::new(&[0x01, 0x02, 0x03]);
    assert_eq!(reader.u8(), Ok(0x01));
    assert_eq!(
        reader.finish(),
        Err(WireError::TrailingData { remaining: 2 })
    );

    let mut reader = Reader::new(&[0x01, 0x02, 0x03]);
    assert_eq!(reader.u8(), Ok(0x01));
    assert_eq!(reader.u16(), Ok(0x0203));
    assert_eq!(reader.finish(), Ok(()));

    assert_eq!(Reader::new(&[]).finish(), Ok(()));
}

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// Length prefixes are backfilled, so a prefix cannot disagree with what
/// follows it — nothing writes one by hand.
#[test]
fn the_writer_backfills_every_prefix() {
    let mut writer = Writer::new();
    writer.u8(0x16);
    writer.vector_u24(|w| {
        w.u16(0x0303);
        w.vector_u8(|w| w.bytes(&[0xaa, 0xbb, 0xcc]));
        w.vector_u16(|w| {
            w.vector_u8(|w| w.bytes(b"nested"));
        });
    });

    let encoded = writer.into_vec();

    // Read it back with the reader and require every prefix to be right.
    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.u8(), Ok(0x16));
    let mut body = reader.sub_u24().expect("the uint24 prefix is honest");
    reader.finish().expect("the outer vector spans the rest");

    assert_eq!(body.u16(), Ok(0x0303));
    assert_eq!(body.vector_u8(), Ok(&[0xaa, 0xbb, 0xcc][..]));
    let mut inner = body.sub_u16().expect("the uint16 prefix is honest");
    body.finish().expect("nothing trails the inner vector");
    assert_eq!(inner.vector_u8(), Ok(&b"nested"[..]));
    inner.finish().expect("exactly consumed");
}

/// Integers go out big-endian, as everything in TLS does, and come back the
/// same. A `uint24` in particular is three octets, not four.
#[test]
fn integers_round_trip_big_endian() {
    let mut writer = Writer::new();
    writer.u8(0x12);
    writer.u16(0x3456);
    writer.u24(0x0078_9abc);
    writer.u32(0xdead_beef);

    let encoded = writer.into_vec();
    assert_eq!(
        encoded,
        vec![0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xad, 0xbe, 0xef],
        "a uint24 was not three octets, or an integer was little-endian"
    );

    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.u8(), Ok(0x12));
    assert_eq!(reader.u16(), Ok(0x3456));
    assert_eq!(reader.u24(), Ok(0x0078_9abc));
    assert_eq!(reader.u32(), Ok(0xdead_beef));
    assert_eq!(reader.finish(), Ok(()));
}

/// An empty vector is a legal encoding, and its prefix must be written and
/// read as zero rather than omitted.
#[test]
fn an_empty_vector_survives_a_round_trip() {
    let mut writer = Writer::new();
    writer.vector_u8(|_| {});
    writer.vector_u16(|_| {});
    writer.vector_u24(|_| {});
    let encoded = writer.into_vec();
    assert_eq!(encoded, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.vector_u8(), Ok(&[][..]));
    assert_eq!(reader.vector_u16(), Ok(&[][..]));
    assert_eq!(reader.vector_u24(), Ok(&[][..]));
    assert_eq!(reader.finish(), Ok(()));
}

/// A vector at the maximum a one-octet prefix can describe, to catch an
/// off-by-one in the backfill.
#[test]
fn a_vector_at_its_prefix_maximum_round_trips() {
    for length in [0usize, 1, 254, 255] {
        let contents = vec![0x5au8; length];
        let mut writer = Writer::new();
        writer.vector_u8(|w| w.bytes(&contents));
        let encoded = writer.into_vec();

        assert_eq!(encoded.len(), length + 1);
        assert_eq!(encoded[0] as usize, length);
        assert_eq!(
            Reader::new(&encoded).vector_u8(),
            Ok(&contents[..]),
            "a {length}-octet vector did not survive"
        );
    }
}
