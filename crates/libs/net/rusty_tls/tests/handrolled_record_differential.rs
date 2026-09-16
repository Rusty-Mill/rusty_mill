//! Differential testing: the hand-rolled record layer against rustls'.
//!
//! `ARCHITECTURE.md` names this mechanism directly — "same input, both
//! backends, byte-identical output" — and ADR-0002 makes it a hard gate for
//! shipping any stage of the hand-rolled engine. This file is stage 1's.
//!
//! rustls' record layer is reachable from outside the crate:
//! `Tls13CipherSuite::aead_alg` is public, and it hands out the same
//! `MessageEncrypter`/`MessageDecrypter` pair rustls uses on a real
//! connection. So these are not two implementations being compared through a
//! summary of their behavior — it is rustls' actual production record layer,
//! keyed identically, asserted byte for byte.
//!
//! Three properties, over a matrix of algorithm × payload length × sequence
//! number × content type:
//!
//! 1. sealing produces byte-identical records,
//! 2. rustls opens what this crate seals,
//! 3. this crate opens what rustls seals.
//!
//! (1) alone would be satisfied by two implementations that agree on
//! encryption and disagree on parsing, which is why (2) and (3) are separate.
//!
//! # The gap this suite does not cover
//!
//! AES-128-GCM. rustls' `AeadKey` is publicly constructible only via
//! `From<[u8; 32]>`, at its maximum length — there is no public way to build
//! the 16-byte key AES-128-GCM needs, so its `MessageEncrypter` cannot be
//! constructed from outside rustls at all. AES-128-GCM is covered instead by
//! the RFC 8448 known-answer vectors in `handrolled_record_kat.rs`, which
//! every one of those vectors happens to use. ADR-0002 records this gap
//! rather than leaving it to be discovered.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rustls::crypto::cipher::{
    AeadKey, InboundOpaqueMessage, Iv, OutboundChunks, OutboundPlainMessage,
};
use rustls::crypto::ring::cipher_suite::{
    TLS13_AES_256_GCM_SHA384, TLS13_CHACHA20_POLY1305_SHA256,
};
use rustls::{ContentType as RustlsType, ProtocolVersion, SupportedCipherSuite, Tls13CipherSuite};
use rusty_tls::handrolled::record::{Aead, ContentType, Opener, Sealer, HEADER_LEN};

fn tls13(suite: SupportedCipherSuite) -> &'static Tls13CipherSuite {
    match suite {
        SupportedCipherSuite::Tls13(inner) => inner,
        other => panic!("{other:?} is not a TLS 1.3 cipher suite"),
    }
}

/// The suites whose keys are 32 bytes, and therefore constructible through
/// rustls' public `AeadKey: From<[u8; 32]>`. See the module docs.
fn comparable_suites() -> Vec<(&'static str, Aead, &'static Tls13CipherSuite)> {
    vec![
        (
            "AES-256-GCM",
            Aead::Aes256Gcm,
            tls13(TLS13_AES_256_GCM_SHA384),
        ),
        (
            "ChaCha20-Poly1305",
            Aead::ChaCha20Poly1305,
            tls13(TLS13_CHACHA20_POLY1305_SHA256),
        ),
    ]
}

/// Deterministic key material — reproducible failures matter more here than
/// unpredictability, and these keys protect nothing.
fn material(seed: u64) -> ([u8; 32], [u8; 12]) {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
    };
    let mut key = [0u8; 32];
    let mut iv = [0u8; 12];
    key.iter_mut().for_each(|b| *b = next());
    iv.iter_mut().for_each(|b| *b = next());
    (key, iv)
}

fn payload_of(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Payload lengths chosen around the boundaries that matter: empty, the AEAD
/// block sizes, and both sides of the 2^14 fragment limit.
const LENGTHS: &[usize] = &[
    0, 1, 15, 16, 17, 31, 32, 63, 64, 255, 1000, 4096, 16383, 16384,
];

/// Sequence numbers chosen around each byte boundary of the 64-bit counter,
/// because the §5.3 nonce is the sequence number XORed into the IV's tail —
/// a bug in that construction is most likely to show up at a carry.
const SEQUENCES: &[u64] = &[
    0,
    1,
    2,
    127,
    128,
    255,
    256,
    65_535,
    65_536,
    16_777_215,
    4_294_967_295,
    4_294_967_296,
    u64::MAX - 1,
    u64::MAX,
];

const TYPES: &[(ContentType, RustlsType)] = &[
    (ContentType::ApplicationData, RustlsType::ApplicationData),
    (ContentType::Handshake, RustlsType::Handshake),
    (ContentType::Alert, RustlsType::Alert),
    (ContentType::ChangeCipherSpec, RustlsType::ChangeCipherSpec),
];

/// The headline property: same key, same IV, same sequence, same plaintext,
/// byte-identical record.
#[test]
fn sealing_matches_rustls_byte_for_byte() {
    let mut cases = 0usize;
    for (name, alg, suite) in comparable_suites() {
        for (i, &len) in LENGTHS.iter().enumerate() {
            let payload = payload_of(len);
            for (j, &seq) in SEQUENCES.iter().enumerate() {
                let (key, iv) = material((i * 977 + j) as u64);
                for &(ours_typ, theirs_typ) in TYPES {
                    let mut sealer = Sealer::new_at(alg, &key, &iv, seq).expect("sealer builds");
                    let ours = sealer
                        .seal(ours_typ, &payload, 0)
                        .expect("sealing succeeds");

                    let mut encrypter = suite.aead_alg.encrypter(AeadKey::from(key), Iv::new(iv));
                    let theirs = encrypter
                        .encrypt(
                            OutboundPlainMessage {
                                typ: theirs_typ,
                                version: ProtocolVersion::TLSv1_2,
                                payload: OutboundChunks::Single(&payload),
                            },
                            seq,
                        )
                        .expect("rustls seals")
                        .encode();

                    assert_eq!(
                        ours, theirs,
                        "{name}: divergence at len={len} seq={seq} typ={ours_typ:?}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 2 * LENGTHS.len() * SEQUENCES.len() * TYPES.len());
}

/// Byte-identical output would still be consistent with a broken parser, so
/// the two directions are asserted separately.
#[test]
fn rustls_opens_what_we_seal() {
    for (name, alg, suite) in comparable_suites() {
        for (i, &len) in LENGTHS.iter().enumerate() {
            let payload = payload_of(len);
            for (j, &seq) in SEQUENCES.iter().enumerate() {
                let (key, iv) = material((i * 613 + j) as u64);
                for &(ours_typ, theirs_typ) in TYPES {
                    let mut sealer = Sealer::new_at(alg, &key, &iv, seq).expect("sealer builds");
                    let ours = sealer.seal(ours_typ, &payload, 0).expect("seals");

                    let mut decrypter = suite.aead_alg.decrypter(AeadKey::from(key), Iv::new(iv));
                    let mut body = ours[HEADER_LEN..].to_vec();
                    let opened = decrypter
                        .decrypt(
                            InboundOpaqueMessage::new(
                                RustlsType::ApplicationData,
                                ProtocolVersion::TLSv1_2,
                                &mut body,
                            ),
                            seq,
                        )
                        .unwrap_or_else(|e| {
                            panic!(
                                "{name}: rustls rejected our record at len={len} seq={seq}: {e:?}"
                            )
                        });

                    assert_eq!(opened.typ, theirs_typ, "{name}: content type at len={len}");
                    assert_eq!(opened.payload, &payload[..], "{name}: payload at len={len}");
                }
            }
        }
    }
}

#[test]
fn we_open_what_rustls_seals() {
    for (name, alg, suite) in comparable_suites() {
        for (i, &len) in LENGTHS.iter().enumerate() {
            let payload = payload_of(len);
            for (j, &seq) in SEQUENCES.iter().enumerate() {
                let (key, iv) = material((i * 419 + j) as u64);
                for &(ours_typ, theirs_typ) in TYPES {
                    let mut encrypter = suite.aead_alg.encrypter(AeadKey::from(key), Iv::new(iv));
                    let theirs = encrypter
                        .encrypt(
                            OutboundPlainMessage {
                                typ: theirs_typ,
                                version: ProtocolVersion::TLSv1_2,
                                payload: OutboundChunks::Single(&payload),
                            },
                            seq,
                        )
                        .expect("rustls seals")
                        .encode();

                    let mut opener = Opener::new_at(alg, &key, &iv, seq).expect("opener builds");
                    let opened = opener.open(&theirs).unwrap_or_else(|e| {
                        panic!("{name}: we rejected rustls' record at len={len} seq={seq}: {e}")
                    });

                    assert_eq!(opened.typ, ours_typ, "{name}: content type at len={len}");
                    assert_eq!(opened.fragment, payload, "{name}: payload at len={len}");
                }
            }
        }
    }
}

/// Padding is the one thing this record layer does that rustls' encrypter has
/// no API for, so it gets its own check: a padded record must still be
/// something rustls accepts and unpads to the original fragment.
#[test]
fn rustls_accepts_our_padded_records() {
    for (name, alg, suite) in comparable_suites() {
        for &padding in &[1usize, 2, 15, 16, 100, 239] {
            for &len in &[0usize, 1, 100, 1000] {
                let (key, iv) = material((padding * 31 + len) as u64);
                let payload = payload_of(len);

                let mut sealer = Sealer::new(alg, &key, &iv).expect("sealer builds");
                let ours = sealer
                    .seal(ContentType::ApplicationData, &payload, padding)
                    .expect("seals");
                assert_eq!(
                    ours.len(),
                    HEADER_LEN + len + 1 + padding + 16,
                    "{name}: padded record length"
                );

                let mut decrypter = suite.aead_alg.decrypter(AeadKey::from(key), Iv::new(iv));
                let mut body = ours[HEADER_LEN..].to_vec();
                let opened = decrypter
                    .decrypt(
                        InboundOpaqueMessage::new(
                            RustlsType::ApplicationData,
                            ProtocolVersion::TLSv1_2,
                            &mut body,
                        ),
                        0,
                    )
                    .unwrap_or_else(|e| panic!("{name}: rustls rejected padding={padding}: {e:?}"));

                assert_eq!(opened.typ, RustlsType::ApplicationData);
                assert_eq!(
                    opened.payload,
                    &payload[..],
                    "{name}: rustls must strip {padding} bytes of padding"
                );
            }
        }
    }
}

/// A record sealed at one sequence number must not open at another, in either
/// implementation. This is the property that makes the sequence counter more
/// than bookkeeping.
#[test]
fn neither_implementation_opens_a_record_at_the_wrong_sequence() {
    for (name, alg, suite) in comparable_suites() {
        let (key, iv) = material(7);
        let payload = payload_of(64);

        let mut sealer = Sealer::new_at(alg, &key, &iv, 42).expect("sealer builds");
        let record = sealer
            .seal(ContentType::ApplicationData, &payload, 0)
            .unwrap();

        for wrong in [0u64, 41, 43, u64::MAX] {
            let mut opener = Opener::new_at(alg, &key, &iv, wrong).expect("opener builds");
            assert!(
                opener.open(&record).is_err(),
                "{name}: our opener accepted seq {wrong} for a seq-42 record"
            );

            let mut decrypter = suite.aead_alg.decrypter(AeadKey::from(key), Iv::new(iv));
            let mut body = record[HEADER_LEN..].to_vec();
            assert!(
                decrypter
                    .decrypt(
                        InboundOpaqueMessage::new(
                            RustlsType::ApplicationData,
                            ProtocolVersion::TLSv1_2,
                            &mut body,
                        ),
                        wrong,
                    )
                    .is_err(),
                "{name}: rustls accepted seq {wrong} for a seq-42 record"
            );
        }
    }
}
