//! Known-answer tests for the TLS 1.2 record layer.
//!
//! Every vector below was produced by a different implementation than the one
//! under test: Python `cryptography` (OpenSSL), building each record from the
//! text of RFC 5246 §6.2.3.3, RFC 5288 and RFC 7905. Nothing here is derived
//! from `rusty_tls`, so a misreading of the RFCs that this crate shares with
//! rustls (the differential suite's blind spot) would still show up.
//!
//! These are also the only record-level coverage of AES-128-GCM: rustls'
//! `AeadKey` cannot be built at 16 bytes from outside the crate, so
//! `handrolled_record12_differential` cannot reach it.
//!
//! The vectors can be regenerated: see the generator at
//! `tests/data/record12_vectors.py`.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::record::{Aead, ContentType};
use rusty_tls::handrolled::record12::{Opener, Sealer};

struct Vector {
    alg: Aead,
    key: &'static str,
    fixed_iv: &'static str,
    seq: u64,
    typ: u8,
    plaintext: &'static str,
    record: &'static str,
    /// True when the explicit nonce on the wire is not the sequence number.
    foreign_explicit_nonce: bool,
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

const VECTORS: &[Vector] = &[
    Vector {
        alg: Aead::Aes128Gcm,
        key: "10171e252c333a41484f565d646b7279",
        fixed_iv: "a0a7aeb5",
        seq: 0,
        typ: 22,
        plaintext: "",
        record: "16030300180000000000000000d24898528a1fc25fd7aca55b44fc8586",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes128Gcm,
        key: "10171e252c333a41484f565d646b7279",
        fixed_iv: "a0a7aeb5",
        seq: 1,
        typ: 23,
        plaintext: "68656c6c6f2c20746c7320312e32",
        record: "17030300260000000000000001c27ac2a17142c91975e1be87a2fd2a19f7f91e0ac05d8bf1691db29bc591",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes128Gcm,
        key: "10171e252c333a41484f565d646b7279",
        fixed_iv: "a0a7aeb5",
        seq: 72623859790382856,
        typ: 21,
        plaintext: "0228",
        record: "150303001a0102030405060708d31474f75ebf74e51f023ef41ad7ec121bbb",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes128Gcm,
        key: "10171e252c333a41484f565d646b7279",
        fixed_iv: "a0a7aeb5",
        seq: 255,
        typ: 23,
        plaintext: "030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8",
        record: "170303007c00000000000000ff34ec7d25d2aa874b6d9e0e4843a9d6c1964c8ba88f2773a3378b1923d962e9885a3ced86367fa42054d6dc938b9172c799cfb38ea8a96850b7067a9a822c65453e5ee3fdb32144e5d23e4837fb7bd60b1c98d412eab7fffcb61dc2926d4c8caeed03146498742104c7eee3c3070533714e51865e",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes256Gcm,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5",
        seq: 0,
        typ: 22,
        plaintext: "",
        record: "160303001800000000000000001c4a12c2c0e3b486009253733095ed6a",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes256Gcm,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5",
        seq: 1,
        typ: 23,
        plaintext: "68656c6c6f2c20746c7320312e32",
        record: "17030300260000000000000001706c83539af1cd11c35e5e39153296a1656152da0d4008c3095954b37551",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes256Gcm,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5",
        seq: 72623859790382856,
        typ: 21,
        plaintext: "0228",
        record: "150303001a010203040506070887b9d239a3ac6fa4119f90d88d816cc03848",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes256Gcm,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5",
        seq: 255,
        typ: 23,
        plaintext: "030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8",
        record: "170303007c00000000000000ff6536cbf0f6999fb992b51af6801e76a54ccefa1cd5ff0d29ecbedc723e45be8a309d5d4a4d1f566a48841c9c6f68c930d879eb2cb27bb899a0ef300ed81c791573e7a9ac9c1d4e1c89377f26fd60f73cf86ea1c3244095cb3dab226569daae685e87fa79490d4e1411bd0a06d2c23dcd24cc0eae",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::ChaCha20Poly1305,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5bcc3cad1d8dfe6ed",
        seq: 0,
        typ: 22,
        plaintext: "",
        record: "1603030010b27fd43d63e0df45cd74dae4b295fb73",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::ChaCha20Poly1305,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5bcc3cad1d8dfe6ed",
        seq: 1,
        typ: 23,
        plaintext: "68656c6c6f2c20746c7320312e32",
        record: "170303001e364fbf99d14110bded1d17f6473fe6da1e6912779ac2edbf5749900b5261",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::ChaCha20Poly1305,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5bcc3cad1d8dfe6ed",
        seq: 72623859790382856,
        typ: 21,
        plaintext: "0228",
        record: "15030300126d915163ac29b39d3425085a84f10dc83612",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::ChaCha20Poly1305,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5bcc3cad1d8dfe6ed",
        seq: 255,
        typ: 23,
        plaintext: "030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8",
        record: "1703030074d54d30a2c4495cedfc5bcc1c27c2eeeca24282532aa26060cee0d4e844ca1931d3919c7183af8e1dd51903eb7b208c223764d0833fc8664d452e83ac6639ec2012affd27a15407b9dd2e6bf656cd31b46294ce65a13b263c2b75e59e632b5493b00bea068bba61c69e152fed8db6f19734999a21",
        foreign_explicit_nonce: false,
    },
    Vector {
        alg: Aead::Aes128Gcm,
        key: "10171e252c333a41484f565d646b7279",
        fixed_iv: "a0a7aeb5",
        seq: 5,
        typ: 23,
        plaintext: "6578706c69636974206e6f6e6365206973207468652073656e64657227732063686f696365",
        record: "170303003ddeadbeefcafebabe9a7a685b2cce7f87f4482896266851e5700b4fec1b7f38015641818b0165e8164f54848b46a0ffe5f5915d43201c427670f88bc6fe",
        foreign_explicit_nonce: true,
    },
    Vector {
        alg: Aead::Aes256Gcm,
        key: "10171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9",
        fixed_iv: "a0a7aeb5",
        seq: 5,
        typ: 23,
        plaintext: "6578706c69636974206e6f6e6365206973207468652073656e64657227732063686f696365",
        record: "170303003ddeadbeefcafebabe3d015d9d533b459f88bf63800ff050fca16ec17d21968591b7cb0b7a550e249654270a26e0e05de06fa664a13770f641d6bfb3c48f",
        foreign_explicit_nonce: true,
    },
];

/// Opening an independently produced record recovers its plaintext and type.
#[test]
fn we_open_every_independent_vector() {
    for (i, v) in VECTORS.iter().enumerate() {
        let mut opener =
            Opener::new_at(v.alg, &unhex(v.key), &unhex(v.fixed_iv), v.seq).expect("opener builds");
        let opened = opener
            .open(&unhex(v.record))
            .unwrap_or_else(|e| panic!("vector {i} ({:?}, seq {}): {e}", v.alg, v.seq));
        assert_eq!(opened.typ, ContentType::from_u8(v.typ), "vector {i}: type");
        assert_eq!(opened.fragment, unhex(v.plaintext), "vector {i}: plaintext");
        assert_eq!(opener.sequence(), v.seq.checked_add(1), "vector {i}: seq");
    }
}

/// Sealing reproduces the independent bytes exactly, wherever the sender's
/// choices (here, the explicit nonce) are the ones this crate makes.
#[test]
fn we_seal_every_vector_we_would_have_produced_byte_for_byte() {
    let mut compared = 0;
    for (i, v) in VECTORS
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.foreign_explicit_nonce)
    {
        let mut sealer =
            Sealer::new_at(v.alg, &unhex(v.key), &unhex(v.fixed_iv), v.seq).expect("sealer builds");
        let sealed = sealer
            .seal(ContentType::from_u8(v.typ), &unhex(v.plaintext))
            .expect("seals");
        assert_eq!(
            sealed,
            unhex(v.record),
            "vector {i} ({:?}, seq {}) diverges from the independent implementation",
            v.alg,
            v.seq
        );
        compared += 1;
    }
    // A filter that matches nothing would pass vacuously.
    assert_eq!(
        compared,
        VECTORS.iter().filter(|v| !v.foreign_explicit_nonce).count()
    );
    assert!(compared >= 12, "only {compared} vectors compared");
}

/// The vectors with a foreign explicit nonce exist to prove the opener reads
/// that field from the wire rather than assuming it equals the sequence number.
#[test]
fn a_foreign_explicit_nonce_is_read_from_the_wire() {
    let foreign: Vec<_> = VECTORS
        .iter()
        .filter(|v| v.foreign_explicit_nonce)
        .collect();
    assert!(foreign.len() >= 2, "the foreign-nonce vectors are missing");
    for v in foreign {
        let record = unhex(v.record);
        let wire_nonce = &record[5..13];
        assert_ne!(
            wire_nonce,
            &v.seq.to_be_bytes(),
            "vector does not exercise the case"
        );

        let mut opener =
            Opener::new_at(v.alg, &unhex(v.key), &unhex(v.fixed_iv), v.seq).expect("opener builds");
        let opened = opener.open(&record).expect("opens");
        assert_eq!(opened.fragment, unhex(v.plaintext));
    }
}
