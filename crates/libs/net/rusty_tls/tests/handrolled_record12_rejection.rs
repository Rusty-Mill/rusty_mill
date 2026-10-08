//! The TLS 1.2 record layer's refusals.
//!
//! A record layer fails open if it is wrong: a missing check does not break
//! any valid connection, it quietly accepts something forged, replayed or
//! reordered. So this file is a list of things that must be refused, each named
//! for the thing and not for the code path, run for all three AEADs.
//!
//! The centrepiece is `every_single_bit_flip_is_refused_with_the_right_error`:
//! it flips each bit of a valid record in turn and requires not just *an* error
//! but the specific one the flipped region calls for. A flip in the type, the
//! explicit nonce or the ciphertext must fail authentication; one in the
//! version must be refused as a version; one in the length as a length. That
//! pins, in one test, that the content type is covered by the tag (it travels in
//! the clear, unlike TLS 1.3) and that nothing is accepted merely because it
//! parsed.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::record::{Aead, ContentType, RecordError, HEADER_LEN};
use rusty_tls::handrolled::record12::{fixed_iv_len, overhead, Opener, Sealer, MAX_CIPHERTEXT_LEN};

const MAX_FRAGMENT: usize = 1 << 14;

const ALGS: [Aead; 3] = [Aead::Aes128Gcm, Aead::Aes256Gcm, Aead::ChaCha20Poly1305];

fn key_for(alg: Aead) -> Vec<u8> {
    (0..alg.key_len())
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(1))
        .collect()
}

fn fixed_for(alg: Aead) -> Vec<u8> {
    (0..fixed_iv_len(alg))
        .map(|i| (i as u8).wrapping_add(0x40))
        .collect()
}

fn sealer(alg: Aead) -> Sealer {
    Sealer::new(alg, &key_for(alg), &fixed_for(alg)).expect("sealer builds")
}

fn opener(alg: Aead) -> Opener {
    Opener::new(alg, &key_for(alg), &fixed_for(alg)).expect("opener builds")
}

fn record(alg: Aead, payload: &[u8]) -> Vec<u8> {
    sealer(alg)
        .seal(ContentType::ApplicationData, payload)
        .expect("seals")
}

// ---------------------------------------------------------------- construction

#[test]
fn a_key_of_the_wrong_length_is_refused() {
    for alg in ALGS {
        let fixed = fixed_for(alg);
        for bad in [0, alg.key_len() - 1, alg.key_len() + 1, 64] {
            let key = vec![1u8; bad];
            let expected = Err(RecordError::KeyLength {
                expected: alg.key_len(),
                actual: bad,
            });
            assert_eq!(
                Sealer::new(alg, &key, &fixed).map(|_| ()),
                expected,
                "{alg:?}"
            );
            assert_eq!(
                Opener::new(alg, &key, &fixed).map(|_| ()),
                expected,
                "{alg:?}"
            );
        }
    }
}

#[test]
fn a_fixed_iv_of_the_wrong_length_is_refused() {
    // The two algorithm families want different lengths (4 and 12), so each is
    // given the other's.
    for alg in ALGS {
        let key = key_for(alg);
        let wrong = if fixed_iv_len(alg) == 4 { 12 } else { 4 };
        for bad in [0, wrong, fixed_iv_len(alg) + 1] {
            let expected = Err(RecordError::FixedIvLength {
                expected: fixed_iv_len(alg),
                actual: bad,
            });
            let iv = vec![9u8; bad];
            assert_eq!(Sealer::new(alg, &key, &iv).map(|_| ()), expected, "{alg:?}");
            assert_eq!(Opener::new(alg, &key, &iv).map(|_| ()), expected, "{alg:?}");
        }
    }
}

// --------------------------------------------------------------------- sealing

#[test]
fn the_fragment_limit_is_exactly_two_to_the_fourteen() {
    for alg in ALGS {
        let mut s = sealer(alg);
        s.seal(ContentType::ApplicationData, &vec![0u8; MAX_FRAGMENT])
            .expect("2^14 is allowed");
        assert_eq!(
            s.seal(ContentType::ApplicationData, &vec![0u8; MAX_FRAGMENT + 1]),
            Err(RecordError::FragmentTooLong {
                len: MAX_FRAGMENT + 1,
                max: MAX_FRAGMENT
            }),
            "{alg:?}"
        );
    }
}

#[test]
fn a_refused_seal_does_not_consume_a_sequence_number() {
    for alg in ALGS {
        let mut s = sealer(alg);
        let _ = s.seal(ContentType::ApplicationData, &vec![0u8; MAX_FRAGMENT + 1]);
        assert_eq!(s.sequence(), Some(0), "{alg:?}");
        s.seal(ContentType::ApplicationData, b"x").expect("seals");
        assert_eq!(s.sequence(), Some(1), "{alg:?}");
    }
}

#[test]
fn records_have_exactly_the_documented_overhead() {
    for alg in ALGS {
        for len in [0usize, 1, 100, MAX_FRAGMENT] {
            let r = record(alg, &vec![7u8; len]);
            assert_eq!(
                r.len(),
                HEADER_LEN + len + overhead(alg),
                "{alg:?} len={len}"
            );
            let declared = usize::from(u16::from_be_bytes([r[3], r[4]]));
            assert_eq!(declared, len + overhead(alg), "{alg:?}: header length");
        }
    }
}

#[test]
fn the_content_type_and_version_are_in_the_clear() {
    for alg in ALGS {
        for typ in [
            ContentType::Handshake,
            ContentType::Alert,
            ContentType::ApplicationData,
            ContentType::Unknown(99),
        ] {
            let r = sealer(alg).seal(typ, b"abc").expect("seals");
            assert_eq!(r[0], typ.as_u8(), "{alg:?}");
            assert_eq!(&r[1..3], &[3, 3], "{alg:?}");
        }
    }
}

#[test]
fn gcm_sends_the_sequence_number_as_its_explicit_nonce() {
    for alg in [Aead::Aes128Gcm, Aead::Aes256Gcm] {
        let mut s = sealer(alg);
        for expected in 0u64..3 {
            let r = s.seal(ContentType::ApplicationData, b"x").expect("seals");
            assert_eq!(
                &r[HEADER_LEN..HEADER_LEN + 8],
                &expected.to_be_bytes(),
                "{alg:?}"
            );
        }
    }
}

#[test]
fn an_unknown_content_type_round_trips_for_the_caller_to_refuse() {
    for alg in ALGS {
        let r = sealer(alg)
            .seal(ContentType::Unknown(99), b"abc")
            .expect("seals");
        let opened = opener(alg).open(&r).expect("opens");
        assert_eq!(opened.typ, ContentType::Unknown(99), "{alg:?}");
        assert_eq!(opened.fragment, b"abc");
    }
}

#[test]
fn an_empty_fragment_round_trips() {
    // RFC 5246 allows zero-length application_data.
    for alg in ALGS {
        let r = record(alg, b"");
        let opened = opener(alg).open(&r).expect("opens");
        assert!(opened.fragment.is_empty(), "{alg:?}");
    }
}

// --------------------------------------------------------------------- framing

#[test]
fn a_record_shorter_than_its_header_is_refused() {
    for alg in ALGS {
        for len in 0..HEADER_LEN {
            assert_eq!(
                opener(alg).open(&vec![23u8; len]),
                Err(RecordError::Truncated {
                    len,
                    min: HEADER_LEN
                }),
                "{alg:?}"
            );
        }
    }
}

#[test]
fn a_header_that_disagrees_with_the_bytes_supplied_is_refused() {
    for alg in ALGS {
        let good = record(alg, b"hello");
        let declared = good.len() - HEADER_LEN;

        let mut short = good.clone();
        short.pop();
        assert_eq!(
            opener(alg).open(&short),
            Err(RecordError::LengthMismatch {
                declared,
                available: declared - 1
            }),
            "{alg:?}: one byte short"
        );

        let mut long = good.clone();
        long.push(0);
        assert_eq!(
            opener(alg).open(&long),
            Err(RecordError::LengthMismatch {
                declared,
                available: declared + 1
            }),
            "{alg:?}: one byte extra"
        );
    }
}

#[test]
fn a_ciphertext_longer_than_the_limit_is_refused_before_decrypting() {
    for alg in ALGS {
        let declared = MAX_CIPHERTEXT_LEN + 1;
        let mut r = vec![23, 3, 3, (declared >> 8) as u8, declared as u8];
        r.extend(std::iter::repeat_n(0xaa, declared));
        assert_eq!(
            opener(alg).open(&r),
            Err(RecordError::EncryptedFragmentTooLong { len: declared }),
            "{alg:?}"
        );
    }
}

#[test]
fn a_record_too_short_to_hold_its_overhead_is_refused() {
    for alg in ALGS {
        for declared in 0..overhead(alg) {
            let mut r = vec![23, 3, 3, 0, declared as u8];
            r.extend(std::iter::repeat_n(0, declared));
            assert_eq!(
                opener(alg).open(&r),
                Err(RecordError::Truncated {
                    len: declared,
                    min: overhead(alg)
                }),
                "{alg:?} declared={declared}"
            );
        }
    }
}

#[test]
fn a_plaintext_over_the_limit_is_refused_without_decrypting() {
    // One byte over 2^14 of plaintext, but still inside the ciphertext limit
    // (2^14 + 2048), so only the plaintext limit can refuse it. The body is
    // garbage, so a refusal as `FragmentTooLong` rather than `Decrypt` shows
    // the length was checked first and no crypto was spent on it.
    for alg in ALGS {
        let declared = MAX_FRAGMENT + 1 + overhead(alg);
        assert!(declared <= MAX_CIPHERTEXT_LEN);
        let mut r = vec![23, 3, 3, (declared >> 8) as u8, declared as u8];
        r.extend(std::iter::repeat_n(0xaa, declared));
        assert_eq!(
            opener(alg).open(&r),
            Err(RecordError::FragmentTooLong {
                len: MAX_FRAGMENT + 1,
                max: MAX_FRAGMENT
            }),
            "{alg:?}"
        );

        // Exactly 2^14 is within bounds, so it reaches the AEAD and fails there.
        let declared = MAX_FRAGMENT + overhead(alg);
        let mut r = vec![23, 3, 3, (declared >> 8) as u8, declared as u8];
        r.extend(std::iter::repeat_n(0xaa, declared));
        assert_eq!(opener(alg).open(&r), Err(RecordError::Decrypt), "{alg:?}");
    }
}

#[test]
fn any_version_but_tls12_is_refused() {
    for alg in ALGS {
        for version in [[3u8, 1], [3, 2], [3, 4], [2, 0], [0, 0], [0xff, 0xff]] {
            let mut r = record(alg, b"hello");
            r[1] = version[0];
            r[2] = version[1];
            assert_eq!(
                opener(alg).open(&r),
                Err(RecordError::UnexpectedVersion(version)),
                "{alg:?}"
            );
        }
    }
}

// ------------------------------------------------------------- authentication

/// What a flip in a given byte of the record must be refused as.
fn expected_refusal(index: usize, flipped: &[u8], original_len: usize) -> RecordError {
    match index {
        1 | 2 => RecordError::UnexpectedVersion([flipped[1], flipped[2]]),
        3 | 4 => RecordError::LengthMismatch {
            declared: usize::from(u16::from_be_bytes([flipped[3], flipped[4]])),
            available: original_len - HEADER_LEN,
        },
        _ => RecordError::Decrypt,
    }
}

#[test]
fn every_single_bit_flip_is_refused_with_the_right_error() {
    for alg in ALGS {
        let good = record(
            alg,
            b"a payload long enough to span a few blocks of ciphertext",
        );
        let mut checked = 0;
        for index in 0..good.len() {
            for bit in 0..8 {
                let mut bad = good.clone();
                bad[index] ^= 1 << bit;

                let mut o = opener(alg);
                assert_eq!(
                    o.open(&bad),
                    Err(expected_refusal(index, &bad, good.len())),
                    "{alg:?}: flipping bit {bit} of byte {index} was not refused correctly"
                );
                // A refused record must not consume a sequence number, so the
                // genuine record still opens afterwards.
                assert_eq!(
                    o.sequence(),
                    Some(0),
                    "{alg:?}: sequence moved on a refusal"
                );
                o.open(&good).expect("the genuine record still opens");
                checked += 1;
            }
        }
        assert_eq!(checked, good.len() * 8);
    }
}

#[test]
fn a_replayed_record_is_refused() {
    for alg in ALGS {
        let r = record(alg, b"once");
        let mut o = opener(alg);
        o.open(&r).expect("first delivery");
        assert_eq!(o.open(&r), Err(RecordError::Decrypt), "{alg:?}");
    }
}

#[test]
fn records_delivered_out_of_order_are_refused_until_the_gap_is_filled() {
    for alg in ALGS {
        let mut s = sealer(alg);
        let r0 = s
            .seal(ContentType::ApplicationData, b"zero")
            .expect("seals");
        let r1 = s.seal(ContentType::ApplicationData, b"one").expect("seals");

        let mut o = opener(alg);
        assert_eq!(o.open(&r1), Err(RecordError::Decrypt), "{alg:?}: r1 first");
        assert_eq!(o.open(&r0).expect("r0").fragment, b"zero");
        assert_eq!(o.open(&r1).expect("r1").fragment, b"one");
    }
}

#[test]
fn the_wrong_key_or_iv_is_refused() {
    for alg in ALGS {
        let r = record(alg, b"secret");

        let mut wrong_key = key_for(alg);
        wrong_key[0] ^= 1;
        let mut o = Opener::new(alg, &wrong_key, &fixed_for(alg)).expect("builds");
        assert_eq!(o.open(&r), Err(RecordError::Decrypt), "{alg:?}: key");

        let mut wrong_iv = fixed_for(alg);
        wrong_iv[0] ^= 1;
        let mut o = Opener::new(alg, &key_for(alg), &wrong_iv).expect("builds");
        assert_eq!(o.open(&r), Err(RecordError::Decrypt), "{alg:?}: iv");
    }
}

#[test]
fn a_record_for_another_algorithm_is_refused_not_a_panic() {
    for from in ALGS {
        for to in ALGS {
            if from == to {
                continue;
            }
            let r = record(from, b"hello");
            assert!(
                opener(to).open(&r).is_err(),
                "{from:?} record opened as {to:?}"
            );
        }
    }
}

// ------------------------------------------------------------------ exhaustion

#[test]
fn the_sequence_number_never_wraps() {
    for alg in ALGS {
        let mut s = Sealer::new_at(alg, &key_for(alg), &fixed_for(alg), u64::MAX).expect("builds");
        let last = s
            .seal(ContentType::ApplicationData, b"last")
            .expect("u64::MAX is usable once");
        assert_eq!(s.sequence(), None, "{alg:?}");
        assert_eq!(
            s.seal(ContentType::ApplicationData, b"again"),
            Err(RecordError::SequenceExhausted),
            "{alg:?}"
        );

        let mut o = Opener::new_at(alg, &key_for(alg), &fixed_for(alg), u64::MAX).expect("builds");
        assert_eq!(o.open(&last).expect("opens").fragment, b"last");
        assert_eq!(o.sequence(), None, "{alg:?}");
        assert_eq!(
            o.open(&last),
            Err(RecordError::SequenceExhausted),
            "{alg:?}"
        );
    }
}

// --------------------------------------------------------------- never panics

#[test]
fn arbitrary_bytes_never_panic_and_never_authenticate() {
    // Hostile input, including inputs with a plausible header, since random
    // bytes alone are refused at the first octet and never reach the AEAD.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for alg in ALGS {
        for _ in 0..3000 {
            let len = (next() % 200) as usize;
            let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if bytes.len() >= HEADER_LEN {
                bytes[0] = 20 + (next() % 4) as u8;
                bytes[1] = 3;
                bytes[2] = 3;
                let body = (bytes.len() - HEADER_LEN) as u16;
                bytes[3..5].copy_from_slice(&body.to_be_bytes());
            }
            assert!(
                opener(alg).open(&bytes).is_err(),
                "{alg:?}: forged record accepted"
            );
        }
    }
}

#[test]
fn debug_output_never_contains_key_material() {
    for alg in ALGS {
        let s = format!("{:?}", sealer(alg));
        let o = format!("{:?}", opener(alg));
        assert_eq!(s, "Sealer { sequence: Some(0), .. }");
        assert_eq!(o, "Opener { sequence: Some(0), .. }");
    }
}
