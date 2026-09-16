//! Every way the hand-rolled record layer must refuse.
//!
//! ADR-0002 makes this a hard gate, for the reason rusty_tls#25 states
//! plainly: the dangerous failure mode of a TLS implementation is *accepting*
//! something it should have rejected, and no happy-path test catches that.
//! The differential suite proves this code agrees with rustls on well-formed
//! input; this file is about ill-formed input, where "agrees with rustls" is
//! not the property anyone wants — "refuses" is.
//!
//! Each test names the specific rejection, so a regression points at what
//! stopped being checked rather than just "a test failed".

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::record::{
    Aead, ContentType, Opener, RecordError, Sealer, HEADER_LEN, MAX_ENCRYPTED_FRAGMENT_LEN,
    MAX_FRAGMENT_LEN, MAX_INNER_PLAINTEXT_LEN, NONCE_LEN, TAG_LEN,
};

const KEY: [u8; 32] = [7u8; 32];
const IV: [u8; NONCE_LEN] = [9u8; NONCE_LEN];

const ALGS: &[(Aead, usize)] = &[
    (Aead::Aes128Gcm, 16),
    (Aead::Aes256Gcm, 32),
    (Aead::ChaCha20Poly1305, 32),
];

fn sealer() -> Sealer {
    Sealer::new(Aead::Aes256Gcm, &KEY, &IV).expect("sealer builds")
}

fn opener() -> Opener {
    Opener::new(Aead::Aes256Gcm, &KEY, &IV).expect("opener builds")
}

fn sealed(fragment: &[u8]) -> Vec<u8> {
    sealer()
        .seal(ContentType::ApplicationData, fragment, 0)
        .expect("seals")
}

// ---------------------------------------------------------------------------
// Key material
// ---------------------------------------------------------------------------

#[test]
fn a_wrong_length_key_is_refused_for_every_algorithm() {
    for &(alg, correct) in ALGS {
        for wrong in [0usize, 1, correct - 1, correct + 1, 64] {
            let key = vec![0u8; wrong];
            assert_eq!(
                Sealer::new(alg, &key, &IV).err(),
                Some(RecordError::KeyLength {
                    expected: correct,
                    actual: wrong
                }),
                "{alg:?} accepted a {wrong}-byte key"
            );
            assert!(
                Opener::new(alg, &key, &IV).is_err(),
                "{alg:?} opener accepted a {wrong}-byte key"
            );
        }
        // ...and the right length is accepted, so the test above is not
        // passing because everything is refused.
        assert!(Sealer::new(alg, &vec![0u8; correct], &IV).is_ok());
    }
}

#[test]
fn a_wrong_length_iv_is_refused() {
    for wrong in [0usize, 1, 11, 13, 16, 32] {
        let iv = vec![0u8; wrong];
        assert_eq!(
            Sealer::new(Aead::Aes256Gcm, &KEY, &iv).err(),
            Some(RecordError::IvLength { actual: wrong })
        );
        assert!(Opener::new(Aead::Aes256Gcm, &KEY, &iv).is_err());
    }
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

#[test]
fn a_record_shorter_than_its_header_is_refused() {
    for len in 0..HEADER_LEN {
        let buf = vec![23u8; len];
        assert_eq!(
            opener().open(&buf),
            Err(RecordError::Truncated {
                len,
                min: HEADER_LEN
            })
        );
    }
}

/// The outer type byte is *not* covered by the AEAD — the additional data is
/// synthesized as `17 03 03 <len>` rather than copied off the wire, so an
/// attacker can flip this byte without disturbing the tag. It therefore has
/// to be checked explicitly, and this is the test that says so.
#[test]
fn an_outer_type_other_than_application_data_is_refused() {
    let mut record = sealed(b"payload");
    for wrong in [0u8, 20, 21, 22, 24, 255] {
        record[0] = wrong;
        assert_eq!(
            opener().open(&record),
            Err(RecordError::UnexpectedOuterType(wrong)),
            "outer type {wrong} was accepted"
        );
    }
    // The genuine value still works, so the loop above is meaningful.
    record[0] = 23;
    assert!(opener().open(&record).is_ok());
}

/// RFC 8446 §5.1: `legacy_record_version` "MUST be ignored for all purposes."
/// Rejecting on it would be a conformance bug, not extra safety.
#[test]
fn the_legacy_record_version_is_ignored() {
    let original = sealed(b"payload");
    for version in [[0x03u8, 0x01], [0x03, 0x02], [0x03, 0x04], [0x00, 0x00]] {
        let mut record = original.clone();
        record[1..3].copy_from_slice(&version);
        let opened = opener()
            .open(&record)
            .unwrap_or_else(|e| panic!("version {version:?} should be ignored, got {e}"));
        assert_eq!(opened.fragment, b"payload");
    }
}

#[test]
fn a_header_length_that_disagrees_with_the_body_is_refused() {
    let record = sealed(b"payload");
    let body_len = record.len() - HEADER_LEN;

    // Declared shorter than supplied.
    let mut short = record.clone();
    short[3..5].copy_from_slice(&((body_len - 1) as u16).to_be_bytes());
    assert_eq!(
        opener().open(&short),
        Err(RecordError::LengthMismatch {
            declared: body_len - 1,
            available: body_len
        })
    );

    // Declared longer than supplied.
    let mut long = record.clone();
    long[3..5].copy_from_slice(&((body_len + 1) as u16).to_be_bytes());
    assert_eq!(
        opener().open(&long),
        Err(RecordError::LengthMismatch {
            declared: body_len + 1,
            available: body_len
        })
    );

    // Trailing bytes are not silently ignored — `open` takes exactly one
    // whole record, and a caller that hands it two has a framing bug.
    let mut extra = record.clone();
    extra.push(0);
    assert_eq!(
        opener().open(&extra),
        Err(RecordError::LengthMismatch {
            declared: body_len,
            available: body_len + 1
        })
    );
}

#[test]
fn an_oversize_encrypted_record_is_refused() {
    let too_long = MAX_ENCRYPTED_FRAGMENT_LEN + 1;
    let mut record = vec![0u8; HEADER_LEN + too_long];
    record[0] = 23;
    record[1..3].copy_from_slice(&[0x03, 0x03]);
    record[3..5].copy_from_slice(&(too_long as u16).to_be_bytes());

    assert_eq!(
        opener().open(&record),
        Err(RecordError::EncryptedFragmentTooLong { len: too_long })
    );
}

/// The shortest legal `encrypted_record` is one content-type octet plus a
/// tag. Anything shorter cannot carry a content type no matter what it
/// decrypts to, and must be refused before the AEAD is asked.
#[test]
fn a_record_too_short_to_hold_a_content_type_is_refused() {
    for declared in 0..=TAG_LEN {
        let mut record = vec![0u8; HEADER_LEN + declared];
        record[0] = 23;
        record[1..3].copy_from_slice(&[0x03, 0x03]);
        record[3..5].copy_from_slice(&(declared as u16).to_be_bytes());

        assert_eq!(
            opener().open(&record),
            Err(RecordError::Truncated {
                len: declared,
                min: TAG_LEN + 1
            }),
            "a {declared}-byte encrypted record was not refused"
        );
    }
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[test]
fn a_tampered_ciphertext_byte_is_refused() {
    let original = sealed(b"the quick brown fox");
    for i in HEADER_LEN..original.len() {
        let mut record = original.clone();
        record[i] ^= 0x01;
        assert_eq!(
            opener().open(&record),
            Err(RecordError::Decrypt),
            "flipping a bit at offset {i} was not detected"
        );
    }
}

/// Changing the length in the header changes the additional data, so even a
/// record whose framing is self-consistent must fail if the length it
/// authenticates under is not the length it was sealed with.
#[test]
fn a_record_resized_consistently_still_fails_the_aead() {
    let original = sealed(b"payload");
    let body_len = original.len() - HEADER_LEN;

    let mut grown = original.clone();
    grown.push(0);
    grown[3..5].copy_from_slice(&((body_len + 1) as u16).to_be_bytes());

    // Framing now agrees with itself, so this gets as far as the AEAD.
    assert_eq!(opener().open(&grown), Err(RecordError::Decrypt));
}

#[test]
fn a_record_does_not_open_under_a_different_key() {
    let record = sealed(b"payload");

    let mut other_key = KEY;
    other_key[0] ^= 0xff;
    let mut opener = Opener::new(Aead::Aes256Gcm, &other_key, &IV).expect("builds");
    assert_eq!(opener.open(&record), Err(RecordError::Decrypt));

    let mut other_iv = IV;
    other_iv[0] ^= 0xff;
    let mut opener = Opener::new(Aead::Aes256Gcm, &KEY, &other_iv).expect("builds");
    assert_eq!(opener.open(&record), Err(RecordError::Decrypt));
}

#[test]
fn a_record_does_not_open_under_a_different_algorithm() {
    let record = sealed(b"payload");
    let mut opener = Opener::new(Aead::ChaCha20Poly1305, &KEY, &IV).expect("builds");
    assert_eq!(opener.open(&record), Err(RecordError::Decrypt));
}

/// Records must arrive in order. Replaying one, or skipping one, breaks the
/// nonce and must be detected.
#[test]
fn records_out_of_order_are_refused() {
    let mut sealer = sealer();
    let first = sealer
        .seal(ContentType::ApplicationData, b"one", 0)
        .unwrap();
    let second = sealer
        .seal(ContentType::ApplicationData, b"two", 0)
        .unwrap();

    let mut opener = opener();
    assert_eq!(opener.open(&second), Err(RecordError::Decrypt), "skipped");
    assert_eq!(opener.open(&first).unwrap().fragment, b"one");
    // A replay of the first record now fails, because the opener has moved on.
    assert_eq!(opener.open(&first), Err(RecordError::Decrypt), "replayed");
    assert_eq!(opener.open(&second).unwrap().fragment, b"two");
}

// ---------------------------------------------------------------------------
// Inner plaintext
// ---------------------------------------------------------------------------

/// RFC 8446 §5.4: an inner plaintext of nothing but zeros carries no content
/// type, and is an error rather than a zero-length `application_data` record.
///
/// The construction is legitimate rather than hand-forged: sealing
/// `ContentType::Unknown(0)` with an empty fragment produces exactly the
/// all-zero inner plaintext a malicious peer would send.
#[test]
fn an_all_zero_inner_plaintext_is_refused() {
    for padding in [0usize, 1, 16, 100] {
        let record = sealer()
            .seal(ContentType::Unknown(0), b"", padding)
            .expect("seals");
        assert_eq!(
            opener().open(&record),
            Err(RecordError::NoContentType),
            "an all-zero inner plaintext with {padding} bytes of padding was accepted"
        );
    }
}

/// Padding is stripped by scanning back for the first non-zero octet, so a
/// fragment that *ends* in zero bytes is the case that construction could
/// plausibly get wrong. It does not: the content type octet is non-zero and
/// stops the scan.
#[test]
fn a_fragment_ending_in_zeros_survives_padding_removal() {
    for trailing in 1..=8usize {
        let mut fragment = vec![0xabu8; 4];
        fragment.extend(std::iter::repeat_n(0u8, trailing));

        for padding in [0usize, 1, 7, 32] {
            let record = sealer()
                .seal(ContentType::ApplicationData, &fragment, padding)
                .expect("seals");
            let opened = opener().open(&record).expect("opens");
            assert_eq!(opened.typ, ContentType::ApplicationData);
            assert_eq!(
                opened.fragment, fragment,
                "{trailing} trailing zeros were eaten as padding"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Size limits on the sealing side
// ---------------------------------------------------------------------------

#[test]
fn an_oversize_fragment_is_refused() {
    let ok = vec![0u8; MAX_FRAGMENT_LEN];
    assert!(
        sealer().seal(ContentType::ApplicationData, &ok, 0).is_ok(),
        "exactly 2^14 must be allowed"
    );

    for over in [MAX_FRAGMENT_LEN + 1, MAX_FRAGMENT_LEN + 1000] {
        let too_big = vec![0u8; over];
        assert_eq!(
            sealer().seal(ContentType::ApplicationData, &too_big, 0),
            Err(RecordError::FragmentTooLong {
                len: over,
                max: MAX_FRAGMENT_LEN
            })
        );
    }
}

#[test]
fn padding_that_overflows_the_inner_plaintext_is_refused() {
    let fragment = vec![0u8; MAX_FRAGMENT_LEN];
    let max_padding = MAX_INNER_PLAINTEXT_LEN - MAX_FRAGMENT_LEN - 1;

    assert!(
        sealer()
            .seal(ContentType::ApplicationData, &fragment, max_padding)
            .is_ok(),
        "the largest legal padding must be allowed"
    );

    assert_eq!(
        sealer().seal(ContentType::ApplicationData, &fragment, max_padding + 1),
        Err(RecordError::FragmentTooLong {
            len: MAX_INNER_PLAINTEXT_LEN + 1,
            max: MAX_INNER_PLAINTEXT_LEN
        })
    );

    // An absurd padding request must not overflow into a small number and be
    // waved through.
    assert!(sealer()
        .seal(ContentType::ApplicationData, b"x", usize::MAX)
        .is_err());
}

// ---------------------------------------------------------------------------
// Sequence numbers
// ---------------------------------------------------------------------------

/// RFC 8446 §5.3 forbids the sequence number from wrapping. Wrapping would
/// reuse a nonce, which for both AES-GCM and ChaCha20-Poly1305 is a
/// catastrophic loss of confidentiality *and* integrity — so this refuses
/// instead.
#[test]
fn the_sequence_number_refuses_to_wrap() {
    let mut sealer = Sealer::new_at(Aead::Aes256Gcm, &KEY, &IV, u64::MAX).expect("builds");
    assert_eq!(sealer.sequence(), Some(u64::MAX));

    let record = sealer
        .seal(ContentType::ApplicationData, b"last", 0)
        .expect("the final record is still allowed");
    assert_eq!(sealer.sequence(), None, "the counter is now exhausted");

    assert_eq!(
        sealer.seal(ContentType::ApplicationData, b"one too many", 0),
        Err(RecordError::SequenceExhausted)
    );

    let mut opener = Opener::new_at(Aead::Aes256Gcm, &KEY, &IV, u64::MAX).expect("builds");
    assert_eq!(opener.open(&record).unwrap().fragment, b"last");
    assert_eq!(opener.sequence(), None);
    assert_eq!(opener.open(&record), Err(RecordError::SequenceExhausted));
}

/// A rejected record must not consume a sequence number, or one bad record
/// would desynchronize everything after it.
#[test]
fn a_rejected_record_does_not_advance_the_sequence() {
    let record = sealed(b"payload");

    let mut opener = opener();
    assert_eq!(opener.sequence(), Some(0));

    let mut tampered = record.clone();
    tampered[HEADER_LEN] ^= 0xff;
    assert_eq!(opener.open(&tampered), Err(RecordError::Decrypt));
    assert_eq!(opener.sequence(), Some(0), "a bad record moved the counter");

    assert!(opener.open(&[]).is_err());
    assert_eq!(opener.sequence(), Some(0));

    // The good record still opens, because nothing was consumed.
    assert_eq!(opener.open(&record).unwrap().fragment, b"payload");
    assert_eq!(opener.sequence(), Some(1));
}

/// Sealing failures likewise must not burn a sequence number.
#[test]
fn a_refused_seal_does_not_advance_the_sequence() {
    let mut sealer = sealer();
    let too_big = vec![0u8; MAX_FRAGMENT_LEN + 1];

    assert!(sealer
        .seal(ContentType::ApplicationData, &too_big, 0)
        .is_err());
    assert_eq!(sealer.sequence(), Some(0));

    assert!(sealer.seal(ContentType::ApplicationData, b"ok", 0).is_ok());
    assert_eq!(sealer.sequence(), Some(1));
}

// ---------------------------------------------------------------------------
// Round trip, across every algorithm
// ---------------------------------------------------------------------------

#[test]
fn every_algorithm_round_trips_including_the_one_the_differential_suite_cannot_reach() {
    for &(alg, key_len) in ALGS {
        let key = vec![0x5au8; key_len];
        let mut sealer = Sealer::new(alg, &key, &IV).expect("sealer builds");
        let mut opener = Opener::new(alg, &key, &IV).expect("opener builds");

        for (i, typ) in [
            ContentType::ApplicationData,
            ContentType::Handshake,
            ContentType::Alert,
            ContentType::ChangeCipherSpec,
            ContentType::Unknown(200),
        ]
        .into_iter()
        .enumerate()
        {
            let fragment = vec![i as u8; i * 37];
            let record = sealer.seal(typ, &fragment, i).expect("seals");
            let opened = opener.open(&record).expect("opens");
            assert_eq!(opened.typ, typ, "{alg:?}");
            assert_eq!(opened.fragment, fragment, "{alg:?}");
        }
    }
}

/// Debug output must never leak key material — these types hold traffic keys,
/// and a stray `{:?}` in a log is exactly how that gets published.
#[test]
fn debug_output_does_not_contain_key_material() {
    let sealer = Sealer::new(Aead::Aes256Gcm, &KEY, &IV).expect("builds");
    let opener = Opener::new(Aead::Aes256Gcm, &KEY, &IV).expect("builds");

    for rendered in [format!("{sealer:?}"), format!("{opener:?}")] {
        assert!(!rendered.contains('7'), "key bytes appear in {rendered}");
        assert!(!rendered.contains('9'), "iv bytes appear in {rendered}");
        assert!(rendered.contains("sequence"), "{rendered}");
    }
}
