//! The TLS 1.2 PRF and the derivations built on it, against two other
//! implementations: OpenSSL's `TLS1-PRF` (34 captured vectors) and rustls'
//! `Prf::for_secret` (a sweep). The derivations themselves are checked against
//! real OpenSSL handshakes in `handrolled_schedule12_openssl.rs`.
//!
//! What this file pins:
//!
//! 1. the PRF is `P_hash` over `label + seed` (OpenSSL, rustls),
//! 2. it is right at every HMAC block boundary and for pieced seeds,
//! 3. each derivation has the shape the RFC gives it (key block size and
//!    partition, `Finished` truncation), and
//! 4. each refuses what it cannot derive from (empty pre-master secret, a hash
//!    of the wrong length, a wrong-length `Finished`).

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rustls::crypto::ring::cipher_suite::{
    TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
};
use rustls::SupportedCipherSuite;
use rusty_tls::handrolled::record::Aead;
use rusty_tls::handrolled::schedule::Hash;
use rusty_tls::handrolled::schedule12::{
    extended_master_secret, finished_verify_data, key_block, prf, verify_finished, MasterSecret,
    ScheduleError, Side, MASTER_SECRET_LEN, RANDOM_LEN, VERIFY_DATA_LEN,
};

struct PrfVector {
    hash: Hash,
    secret: &'static str,
    label: &'static str,
    seed: &'static str,
    out: &'static str,
}

include!("data/tls12_prf_vectors.rs");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn pat(len: usize, start: u8) -> Vec<u8> {
    (0..len)
        .map(|i| start.wrapping_add((i as u8).wrapping_mul(13)))
        .collect()
}

fn rustls_prf(hash: Hash) -> &'static dyn rustls::crypto::tls12::Prf {
    let suite = match hash {
        Hash::Sha256 => TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        Hash::Sha384 => TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        other => panic!("no rustls suite for {other:?}"),
    };
    match suite {
        SupportedCipherSuite::Tls12(inner) => inner.prf_provider,
        other => panic!("{other:?} is not TLS 1.2"),
    }
}

fn run_prf(hash: Hash, secret: &[u8], label: &[u8], seed: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    prf(hash, secret, label, &[seed], &mut out);
    out
}

// ------------------------------------------------------------------ the PRF

#[test]
fn the_prf_matches_every_openssl_vector() {
    assert!(PRF_VECTORS.len() >= 34, "the vector table is incomplete");
    for (i, v) in PRF_VECTORS.iter().enumerate() {
        let got = run_prf(
            v.hash,
            &unhex(v.secret),
            &unhex(v.label),
            &unhex(v.seed),
            v.out.len() / 2,
        );
        assert_eq!(
            got,
            unhex(v.out),
            "vector {i} ({:?}, {} bytes out)",
            v.hash,
            v.out.len() / 2
        );
    }
}

#[test]
fn the_prf_matches_rustls_across_a_sweep() {
    let mut cases = 0;
    for hash in [Hash::Sha256, Hash::Sha384] {
        for secret_len in [1usize, 16, 32, 48, 49, 100] {
            for label in [
                &b""[..],
                b"master secret",
                b"extended master secret",
                b"key expansion",
            ] {
                for seed_len in [0usize, 1, 31, 32, 64, 65] {
                    for out_len in [
                        1usize, 11, 12, 31, 32, 33, 47, 48, 49, 64, 72, 88, 96, 97, 255,
                    ] {
                        let secret = pat(secret_len, 3);
                        let seed = pat(seed_len, 99);
                        let ours = run_prf(hash, &secret, label, &seed, out_len);
                        let mut theirs = vec![0u8; out_len];
                        rustls_prf(hash).for_secret(&mut theirs, &secret, label, &seed);
                        assert_eq!(
                            ours, theirs,
                            "{hash:?} secret={secret_len} label={label:?} seed={seed_len} out={out_len}"
                        );
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 2 * 6 * 4 * 6 * 15);
}

#[test]
fn a_seed_given_in_pieces_equals_the_same_seed_joined() {
    for hash in [Hash::Sha256, Hash::Sha384] {
        let secret = pat(48, 1);
        let (a, b, c) = (pat(32, 10), pat(32, 20), pat(5, 30));
        let joined: Vec<u8> = [&a[..], &b[..], &c[..]].concat();

        let mut pieced = [0u8; 100];
        prf(hash, &secret, b"label", &[&a, &b, &c], &mut pieced);
        assert_eq!(
            pieced.to_vec(),
            run_prf(hash, &secret, b"label", &joined, 100),
            "{hash:?}"
        );

        // Order matters: it is the whole content of the seed.
        let mut swapped = [0u8; 100];
        prf(hash, &secret, b"label", &[&b, &a, &c], &mut swapped);
        assert_ne!(
            pieced, swapped,
            "{hash:?}: seed order must change the output"
        );
    }
}

#[test]
fn a_shorter_output_is_a_prefix_of_a_longer_one() {
    for hash in [Hash::Sha256, Hash::Sha384] {
        let long = run_prf(hash, b"secret", b"label", b"seed", 200);
        for n in [0usize, 1, 12, 31, 32, 33, 48, 49, 100, 199] {
            assert_eq!(
                run_prf(hash, b"secret", b"label", b"seed", n),
                long[..n],
                "{hash:?} {n}"
            );
        }
    }
}

#[test]
fn secret_label_and_seed_each_change_the_output() {
    let base = run_prf(Hash::Sha256, b"secret", b"label", b"seed", 32);
    assert_ne!(
        base,
        run_prf(Hash::Sha256, b"secreT", b"label", b"seed", 32)
    );
    assert_ne!(
        base,
        run_prf(Hash::Sha256, b"secret", b"labeL", b"seed", 32)
    );
    assert_ne!(
        base,
        run_prf(Hash::Sha256, b"secret", b"label", b"seeD", 32)
    );
    assert_ne!(
        base,
        run_prf(Hash::Sha384, b"secret", b"label", b"seed", 32)
    );
}

// ------------------------------------------------- extended master secret

#[test]
fn the_extended_master_secret_is_the_prf_of_the_session_hash() {
    for hash in [Hash::Sha256, Hash::Sha384] {
        let pms = pat(32, 5);
        let session_hash = hash.hash(b"the handshake so far");
        let ours = extended_master_secret(hash, &pms, &session_hash).expect("derives");

        let mut theirs = [0u8; MASTER_SECRET_LEN];
        rustls_prf(hash).for_secret(&mut theirs, &pms, b"extended master secret", &session_hash);
        assert_eq!(ours.as_bytes(), &theirs, "{hash:?}");
    }
}

#[test]
fn the_extended_master_secret_depends_on_the_transcript() {
    // The point of RFC 7627: two handshakes with the same pre-master secret and
    // different transcripts must not share a master secret.
    let pms = pat(32, 5);
    let a =
        extended_master_secret(Hash::Sha256, &pms, &Hash::Sha256.hash(b"transcript A")).unwrap();
    let b =
        extended_master_secret(Hash::Sha256, &pms, &Hash::Sha256.hash(b"transcript B")).unwrap();
    assert_ne!(a.as_bytes(), b.as_bytes());
}

#[test]
fn an_empty_pre_master_secret_is_refused() {
    assert_eq!(
        extended_master_secret(Hash::Sha256, b"", &Hash::Sha256.hash(b"x")).map(|_| ()),
        Err(ScheduleError::EmptyPreMasterSecret)
    );
}

#[test]
fn a_session_hash_of_the_wrong_length_is_refused() {
    // A SHA-256 hash handed to a SHA-384 suite would otherwise derive a secret
    // the peer does not have, silently.
    let sha256 = Hash::Sha256.hash(b"x");
    let sha384 = Hash::Sha384.hash(b"x");
    assert_eq!(
        extended_master_secret(Hash::Sha384, &pat(32, 1), &sha256).map(|_| ()),
        Err(ScheduleError::HashLength {
            expected: 48,
            actual: 32
        })
    );
    assert_eq!(
        extended_master_secret(Hash::Sha256, &pat(32, 1), &sha384).map(|_| ()),
        Err(ScheduleError::HashLength {
            expected: 32,
            actual: 48
        })
    );
    for bad in [0usize, 1, 31, 33, 47, 49] {
        for hash in [Hash::Sha256, Hash::Sha384] {
            if bad == hash.len() {
                continue;
            }
            assert!(
                extended_master_secret(hash, &pat(32, 1), &vec![0u8; bad]).is_err(),
                "{hash:?} {bad}"
            );
        }
    }
}

#[test]
fn a_master_secret_never_prints_itself() {
    let ms = extended_master_secret(Hash::Sha256, &pat(32, 1), &Hash::Sha256.hash(b"x")).unwrap();
    assert_eq!(format!("{ms:?}"), "MasterSecret(<redacted>)");
}

// ------------------------------------------------------------- key block

fn master() -> MasterSecret {
    MasterSecret::from_bytes(
        <[u8; MASTER_SECRET_LEN]>::try_from(pat(MASTER_SECRET_LEN, 9)).unwrap(),
    )
}

fn randoms() -> ([u8; RANDOM_LEN], [u8; RANDOM_LEN]) {
    (
        <[u8; RANDOM_LEN]>::try_from(pat(RANDOM_LEN, 1)).unwrap(),
        <[u8; RANDOM_LEN]>::try_from(pat(RANDOM_LEN, 200)).unwrap(),
    )
}

#[test]
fn the_key_block_is_partitioned_in_rfc_order_with_the_right_sizes() {
    let (client_random, server_random) = randoms();
    for (alg, hash, key_len, iv_len) in [
        (Aead::Aes128Gcm, Hash::Sha256, 16, 4),
        (Aead::Aes256Gcm, Hash::Sha384, 32, 4),
        (Aead::ChaCha20Poly1305, Hash::Sha256, 32, 12),
    ] {
        let kb = key_block(hash, alg, &master(), &client_random, &server_random);
        assert_eq!(kb.client_write_key.len(), key_len, "{alg:?}");
        assert_eq!(kb.server_write_key.len(), key_len, "{alg:?}");
        assert_eq!(kb.client_write_iv.len(), iv_len, "{alg:?}");
        assert_eq!(kb.server_write_iv.len(), iv_len, "{alg:?}");

        // The raw PRF output, sliced by hand: client key, server key, client IV,
        // server IV, with the seed `server_random + client_random`.
        let mut raw = vec![0u8; 2 * key_len + 2 * iv_len];
        prf(
            hash,
            master().as_bytes(),
            b"key expansion",
            &[&server_random, &client_random],
            &mut raw,
        );
        assert_eq!(kb.client_write_key, raw[..key_len], "{alg:?}");
        assert_eq!(kb.server_write_key, raw[key_len..2 * key_len], "{alg:?}");
        assert_eq!(
            kb.client_write_iv,
            raw[2 * key_len..2 * key_len + iv_len],
            "{alg:?}"
        );
        assert_eq!(kb.server_write_iv, raw[2 * key_len + iv_len..], "{alg:?}");
    }
}

#[test]
fn the_key_block_seed_order_is_server_then_client() {
    // The classic mistake: reuse the master secret's "client then server" order.
    let (client_random, server_random) = randoms();
    let right = key_block(
        Hash::Sha256,
        Aead::Aes128Gcm,
        &master(),
        &client_random,
        &server_random,
    );
    let swapped = key_block(
        Hash::Sha256,
        Aead::Aes128Gcm,
        &master(),
        &server_random,
        &client_random,
    );
    assert_ne!(right.client_write_key, swapped.client_write_key);
    assert_ne!(right.server_write_key, swapped.server_write_key);
}

#[test]
fn the_key_block_never_prints_itself() {
    let (c, s) = randoms();
    let kb = key_block(Hash::Sha256, Aead::Aes128Gcm, &master(), &c, &s);
    assert_eq!(format!("{kb:?}"), "KeyBlock(<redacted>)");
}

// -------------------------------------------------------------- Finished

#[test]
fn finished_is_the_first_twelve_octets_of_the_prf() {
    for hash in [Hash::Sha256, Hash::Sha384] {
        let handshake_hash = hash.hash(b"handshake");
        for (side, label) in [
            (Side::Client, &b"client finished"[..]),
            (Side::Server, b"server finished"),
        ] {
            let got = finished_verify_data(hash, &master(), side, &handshake_hash).unwrap();
            let mut full = [0u8; 32];
            prf(
                hash,
                master().as_bytes(),
                label,
                &[&handshake_hash],
                &mut full,
            );
            assert_eq!(got.len(), VERIFY_DATA_LEN);
            assert_eq!(got, full[..VERIFY_DATA_LEN], "{hash:?} {side:?}");
        }
    }
}

#[test]
fn the_two_sides_finished_differ() {
    let h = Hash::Sha256.hash(b"handshake");
    let c = finished_verify_data(Hash::Sha256, &master(), Side::Client, &h).unwrap();
    let s = finished_verify_data(Hash::Sha256, &master(), Side::Server, &h).unwrap();
    assert_ne!(c, s, "a reflected Finished must not verify");
}

#[test]
fn verify_finished_accepts_only_the_exact_value() {
    for hash in [Hash::Sha256, Hash::Sha384] {
        let h = hash.hash(b"handshake");
        let good = finished_verify_data(hash, &master(), Side::Client, &h).unwrap();
        assert!(verify_finished(hash, &master(), Side::Client, &h, &good));

        // Every single-bit change, and the other side's value, and bad lengths.
        for i in 0..good.len() {
            for bit in 0..8 {
                let mut bad = good;
                bad[i] ^= 1 << bit;
                assert!(
                    !verify_finished(hash, &master(), Side::Client, &h, &bad),
                    "{hash:?} byte {i} bit {bit}"
                );
            }
        }
        let other = finished_verify_data(hash, &master(), Side::Server, &h).unwrap();
        assert!(
            !verify_finished(hash, &master(), Side::Client, &h, &other),
            "{hash:?}: reflected"
        );
        assert!(
            !verify_finished(hash, &master(), Side::Client, &h, &good[..11]),
            "{hash:?}: short"
        );
        assert!(
            !verify_finished(hash, &master(), Side::Client, &h, &[]),
            "{hash:?}: empty"
        );
        let mut long = good.to_vec();
        long.push(0);
        assert!(
            !verify_finished(hash, &master(), Side::Client, &h, &long),
            "{hash:?}: long"
        );
    }
}

#[test]
fn finished_over_a_different_transcript_does_not_verify() {
    let h1 = Hash::Sha256.hash(b"handshake one");
    let h2 = Hash::Sha256.hash(b"handshake two");
    let v1 = finished_verify_data(Hash::Sha256, &master(), Side::Client, &h1).unwrap();
    assert!(!verify_finished(
        Hash::Sha256,
        &master(),
        Side::Client,
        &h2,
        &v1
    ));
}

#[test]
fn a_handshake_hash_of_the_wrong_length_is_refused() {
    let wrong = Hash::Sha256.hash(b"x");
    assert_eq!(
        finished_verify_data(Hash::Sha384, &master(), Side::Client, &wrong),
        Err(ScheduleError::HashLength {
            expected: 48,
            actual: 32
        })
    );
    // verify_finished folds every refusal into `false`.
    assert!(!verify_finished(
        Hash::Sha384,
        &master(),
        Side::Client,
        &wrong,
        &[0u8; 12]
    ));
}
