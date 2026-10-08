//! Differential testing: the hand-rolled TLS 1.2 record layer against rustls'.
//!
//! Same mechanism as `handrolled_record_differential.rs`, for stage 4b-i:
//! `Tls12CipherSuite::aead_alg` is public, and hands out the very
//! `MessageEncrypter`/`MessageDecrypter` pair rustls uses on a real TLS 1.2
//! connection, so this is rustls' production record layer asserted byte for
//! byte, not a summary of it.
//!
//! Three properties over algorithm × payload length × sequence number ×
//! content type:
//!
//! 1. sealing produces byte-identical records,
//! 2. rustls opens what this crate seals,
//! 3. this crate opens what rustls seals.
//!
//! # The explicit nonce, and why one test uses a non-zero start
//!
//! RFC 5288 leaves the 8-byte explicit part of an AES-GCM nonce to the sender.
//! This crate sends the sequence number. rustls XORs the sequence number into a
//! starting value it takes from the key block. With a zero starting value the
//! two coincide, so properties 1 and 2 use zero and can be byte-identical. A
//! real rustls connection does *not* start at zero, so property 3 is repeated
//! with a non-zero start: that is the case where the nonce on the wire differs
//! from the sequence number, and the one an opener that assumed otherwise would
//! get wrong.
//!
//! # The gap this suite does not cover
//!
//! AES-128-GCM, for the reason `handrolled_record_differential.rs` gives:
//! rustls' `AeadKey` is only publicly constructible at 32 bytes.
//! `handrolled_record12_kat.rs` covers it with vectors from an unrelated
//! implementation.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rustls::crypto::cipher::{AeadKey, InboundOpaqueMessage, OutboundChunks, OutboundPlainMessage};
use rustls::crypto::ring::cipher_suite::{
    TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
};
use rustls::{ContentType as RustlsType, ProtocolVersion, SupportedCipherSuite, Tls12CipherSuite};
use rusty_tls::handrolled::record::{Aead, ContentType};
use rusty_tls::handrolled::record12::{fixed_iv_len, Opener, Sealer};

const HEADER_LEN: usize = 5;

fn tls12(suite: SupportedCipherSuite) -> &'static Tls12CipherSuite {
    match suite {
        SupportedCipherSuite::Tls12(inner) => inner,
        other => panic!("{other:?} is not a TLS 1.2 cipher suite"),
    }
}

fn comparable_suites() -> Vec<(&'static str, Aead, &'static Tls12CipherSuite)> {
    vec![
        (
            "AES-256-GCM",
            Aead::Aes256Gcm,
            tls12(TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384),
        ),
        (
            "ChaCha20-Poly1305",
            Aead::ChaCha20Poly1305,
            tls12(TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256),
        ),
    ]
}

/// Deterministic key material: reproducible failures matter more than
/// unpredictability, and these keys protect nothing.
fn material(seed: u64, fixed_len: usize) -> ([u8; 32], Vec<u8>) {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
    };
    let mut key = [0u8; 32];
    key.iter_mut().for_each(|b| *b = next());
    let fixed = (0..fixed_len).map(|_| next()).collect();
    (key, fixed)
}

fn payload_of(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Empty, around the AEAD block sizes, and both sides of the 2^14 limit.
const LENGTHS: &[usize] = &[
    0, 1, 15, 16, 17, 31, 32, 63, 64, 255, 1000, 4096, 16383, 16384,
];

/// Around each byte boundary of the 64-bit counter, where a carry bug in the
/// nonce or the additional data would show.
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

/// rustls' encrypter. `explicit` is the starting explicit nonce (ignored for
/// ChaCha20-Poly1305, which has none).
fn rustls_encrypter(
    suite: &Tls12CipherSuite,
    key: [u8; 32],
    fixed: &[u8],
    explicit: &[u8],
) -> Box<dyn rustls::crypto::cipher::MessageEncrypter> {
    suite
        .aead_alg
        .encrypter(AeadKey::from(key), fixed, explicit)
}

fn rustls_seals(
    encrypter: &mut dyn rustls::crypto::cipher::MessageEncrypter,
    typ: RustlsType,
    payload: &[u8],
    seq: u64,
) -> Vec<u8> {
    encrypter
        .encrypt(
            OutboundPlainMessage {
                typ,
                version: ProtocolVersion::TLSv1_2,
                payload: OutboundChunks::Single(payload),
            },
            seq,
        )
        .expect("rustls seals")
        .encode()
}

#[test]
fn sealing_matches_rustls_byte_for_byte() {
    let mut cases = 0usize;
    for (name, alg, suite) in comparable_suites() {
        for (i, &len) in LENGTHS.iter().enumerate() {
            let payload = payload_of(len);
            for (j, &seq) in SEQUENCES.iter().enumerate() {
                let (key, fixed) = material((i * 977 + j) as u64, fixed_iv_len(alg));
                for &(ours_typ, theirs_typ) in TYPES {
                    let mut sealer = Sealer::new_at(alg, &key, &fixed, seq).expect("sealer builds");
                    let ours = sealer.seal(ours_typ, &payload).expect("seals");

                    // A zero starting explicit nonce makes rustls' wire nonce
                    // equal to the sequence number, as ours is.
                    let mut encrypter = rustls_encrypter(suite, key, &fixed, &[0u8; 8]);
                    let theirs = rustls_seals(&mut *encrypter, theirs_typ, &payload, seq);

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

#[test]
fn rustls_opens_what_we_seal() {
    for (name, alg, suite) in comparable_suites() {
        for (i, &len) in LENGTHS.iter().enumerate() {
            let payload = payload_of(len);
            for (j, &seq) in SEQUENCES.iter().enumerate() {
                let (key, fixed) = material((i * 613 + j) as u64, fixed_iv_len(alg));
                for &(ours_typ, theirs_typ) in TYPES {
                    let mut sealer = Sealer::new_at(alg, &key, &fixed, seq).expect("sealer builds");
                    let ours = sealer.seal(ours_typ, &payload).expect("seals");

                    let mut decrypter = suite.aead_alg.decrypter(AeadKey::from(key), &fixed);
                    let mut body = ours[HEADER_LEN..].to_vec();
                    let opened = decrypter
                        .decrypt(
                            InboundOpaqueMessage::new(
                                theirs_typ,
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

                    assert_eq!(opened.typ, theirs_typ, "{name}: type at len={len}");
                    assert_eq!(opened.payload, &payload[..], "{name}: payload at len={len}");
                }
            }
        }
    }
}

/// Run twice: with a zero starting explicit nonce (wire nonce = sequence) and
/// with a non-zero one (wire nonce differs from the sequence), which is how a
/// real rustls connection behaves.
#[test]
fn we_open_what_rustls_seals() {
    let starts: [[u8; 8]; 2] = [[0u8; 8], [0xa5, 0x5a, 0x01, 0x80, 0xff, 0x00, 0x7e, 0xc3]];
    for explicit in starts {
        for (name, alg, suite) in comparable_suites() {
            for (i, &len) in LENGTHS.iter().enumerate() {
                let payload = payload_of(len);
                for (j, &seq) in SEQUENCES.iter().enumerate() {
                    let (key, fixed) = material((i * 419 + j) as u64, fixed_iv_len(alg));
                    for &(ours_typ, theirs_typ) in TYPES {
                        let mut encrypter = rustls_encrypter(suite, key, &fixed, &explicit);
                        let theirs = rustls_seals(&mut *encrypter, theirs_typ, &payload, seq);

                        let mut opener =
                            Opener::new_at(alg, &key, &fixed, seq).expect("opener builds");
                        let opened = opener.open(&theirs).unwrap_or_else(|e| {
                            panic!(
                                "{name}: we rejected rustls' record at len={len} seq={seq} \
                                 explicit={explicit:02x?}: {e}"
                            )
                        });

                        assert_eq!(opened.typ, ours_typ, "{name}: type at len={len}");
                        assert_eq!(opened.fragment, payload, "{name}: payload at len={len}");
                    }
                }
            }
        }
    }
}

/// The non-zero start really does put a different value on the wire. Without
/// this the test above could pass with both runs exercising the same case.
#[test]
fn a_non_zero_starting_nonce_changes_the_wire_nonce() {
    let (_, alg, suite) = comparable_suites().remove(0);
    assert_eq!(alg, Aead::Aes256Gcm);
    let (key, fixed) = material(7, fixed_iv_len(alg));
    let mut zero = rustls_encrypter(suite, key, &fixed, &[0u8; 8]);
    let mut nonzero = rustls_encrypter(suite, key, &fixed, &[1u8; 8]);
    let a = rustls_seals(&mut *zero, RustlsType::ApplicationData, b"x", 3);
    let b = rustls_seals(&mut *nonzero, RustlsType::ApplicationData, b"x", 3);
    assert_eq!(
        &a[5..13],
        &3u64.to_be_bytes(),
        "zero start puts seq on the wire"
    );
    assert_ne!(
        &a[5..13],
        &b[5..13],
        "non-zero start must differ on the wire"
    );
}
