//! The TLS 1.2 key derivation and record layer against real OpenSSL handshakes.
//!
//! TLS 1.2 has no published full-handshake trace like RFC 8448's, and a
//! derivation tested only against itself, or against another implementation
//! reached through the same reading of the RFC, would agree with a mistake.
//! So each case here is a handshake that OpenSSL really performed, captured on
//! the wire (`tests/data/tls12_openssl_traces.py`). From the hello randoms, the
//! transcript and the pre-master secret, this crate derives everything; then the
//! only test that matters is whether the result works on OpenSSL's own bytes:
//!
//! - the extended master secret equals the one OpenSSL logged (RSA key exchange
//!   only: for ECDHE the pre-master secret is not recoverable from a capture),
//! - the key block decrypts OpenSSL's encrypted `Finished` records in both
//!   directions, which only happens if the seed order, the partition and the
//!   fixed IVs are all right,
//! - the `verify_data` inside each decrypted `Finished` equals the one this crate
//!   computes, which pins the labels, the 12-octet truncation and which messages
//!   the hash covers (the server's covers the client's `Finished`), and
//! - the first application-data record in each direction decrypts to the
//!   plaintext OpenSSL sent.
//!
//! The negative controls at the bottom matter as much: they make the classic
//! mistakes on purpose and require this oracle to notice, so the suite cannot be
//! green because it never exercised the derivation.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use rusty_tls::handrolled::record::{Aead, RecordError};
use rusty_tls::handrolled::record12::Opener;
use rusty_tls::handrolled::schedule::Hash;
use rusty_tls::handrolled::schedule12::{
    extended_master_secret, finished_verify_data, key_block, prf, verify_finished, MasterSecret,
    Side, RANDOM_LEN,
};

struct Trace {
    cipher: &'static str,
    aead: Aead,
    hash: Hash,
    client_random: &'static str,
    server_random: &'static str,
    master_secret: &'static str,
    pre_master_secret: Option<&'static str>,
    transcript_to_client_key_exchange: &'static [&'static str],
    client_finished_record: &'static str,
    server_finished_record: &'static str,
    client_app_record: &'static str,
    server_app_record: &'static str,
    client_request: &'static str,
}

include!("data/tls12_openssl_traces.rs");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn random(s: &str) -> [u8; RANDOM_LEN] {
    <[u8; RANDOM_LEN]>::try_from(unhex(s)).expect("32-byte random")
}

fn master(t: &Trace) -> MasterSecret {
    MasterSecret::from_bytes(<[u8; 48]>::try_from(unhex(t.master_secret)).expect("48 bytes"))
}

fn transcript(t: &Trace) -> Vec<u8> {
    t.transcript_to_client_key_exchange
        .iter()
        .flat_map(|m| unhex(m))
        .collect()
}

#[test]
fn the_trace_table_covers_every_suite_this_stage_derives_for() {
    assert_eq!(
        TRACES.len(),
        5,
        "regenerate with tests/data/tls12_openssl_traces.py"
    );
    assert!(TRACES
        .iter()
        .any(|t| t.aead == Aead::Aes128Gcm && t.hash == Hash::Sha256));
    assert!(TRACES
        .iter()
        .any(|t| t.aead == Aead::Aes256Gcm && t.hash == Hash::Sha384));
    assert!(TRACES.iter().any(|t| t.aead == Aead::ChaCha20Poly1305));
    assert_eq!(
        TRACES
            .iter()
            .filter(|t| t.pre_master_secret.is_some())
            .count(),
        2
    );
}

/// The extended master secret, from the pre-master secret OpenSSL's client sent
/// and the hash of the transcript up to ClientKeyExchange.
#[test]
fn the_extended_master_secret_equals_the_one_openssl_logged() {
    let mut checked = 0;
    for t in TRACES {
        let Some(pms) = t.pre_master_secret else {
            continue;
        };
        let session_hash = t.hash.hash(&transcript(t));
        let ours = extended_master_secret(t.hash, &unhex(pms), &session_hash).expect("derives");
        assert_eq!(
            ours.as_bytes().to_vec(),
            unhex(t.master_secret),
            "{}",
            t.cipher
        );
        checked += 1;
    }
    assert_eq!(checked, 2, "both RSA key exchange traces must be checked");
}

/// Decrypt a record from the capture with the keys this crate derives.
fn open(t: &Trace, side: Side, seq: u64, record: &str) -> Result<Vec<u8>, RecordError> {
    let kb = key_block(
        t.hash,
        t.aead,
        &master(t),
        &random(t.client_random),
        &random(t.server_random),
    );
    let (key, iv) = match side {
        Side::Client => (kb.client_write_key, kb.client_write_iv),
        Side::Server => (kb.server_write_key, kb.server_write_iv),
    };
    let mut opener = Opener::new_at(t.aead, &key, &iv, seq).expect("opener builds");
    opener.open(&unhex(record)).map(|o| o.fragment)
}

fn split_finished(message: &[u8]) -> &[u8] {
    assert_eq!(
        &message[..4],
        &[0x14, 0, 0, 12],
        "a Finished handshake header"
    );
    assert_eq!(message.len(), 16);
    &message[4..]
}

#[test]
fn the_key_block_decrypts_openssls_finished_in_both_directions() {
    for t in TRACES {
        let client = open(t, Side::Client, 0, t.client_finished_record)
            .unwrap_or_else(|e| panic!("{}: client Finished: {e}", t.cipher));
        let server = open(t, Side::Server, 0, t.server_finished_record)
            .unwrap_or_else(|e| panic!("{}: server Finished: {e}", t.cipher));
        assert_eq!(client.len(), 16, "{}", t.cipher);
        assert_eq!(server.len(), 16, "{}", t.cipher);
    }
}

#[test]
fn the_verify_data_equals_what_openssl_sent_in_both_directions() {
    for t in TRACES {
        let ms = master(t);
        let up_to_cke = transcript(t);

        // Client: the transcript up to and including ClientKeyExchange.
        let client_msg = open(t, Side::Client, 0, t.client_finished_record).expect("opens");
        let expected =
            finished_verify_data(t.hash, &ms, Side::Client, &t.hash.hash(&up_to_cke)).unwrap();
        assert_eq!(
            split_finished(&client_msg),
            expected,
            "{}: client verify_data",
            t.cipher
        );
        assert!(verify_finished(
            t.hash,
            &ms,
            Side::Client,
            &t.hash.hash(&up_to_cke),
            split_finished(&client_msg)
        ));

        // Server: the same transcript plus the client's Finished message.
        let with_client_finished: Vec<u8> = [up_to_cke.as_slice(), client_msg.as_slice()].concat();
        let server_msg = open(t, Side::Server, 0, t.server_finished_record).expect("opens");
        let h = t.hash.hash(&with_client_finished);
        let expected = finished_verify_data(t.hash, &ms, Side::Server, &h).unwrap();
        assert_eq!(
            split_finished(&server_msg),
            expected,
            "{}: server verify_data",
            t.cipher
        );
        assert!(verify_finished(
            t.hash,
            &ms,
            Side::Server,
            &h,
            split_finished(&server_msg)
        ));
    }
}

#[test]
fn application_data_decrypts_to_what_was_sent() {
    for t in TRACES {
        // Sequence 1: the Finished record was number 0 in each direction.
        let request = open(t, Side::Client, 1, t.client_app_record)
            .unwrap_or_else(|e| panic!("{}: client data: {e}", t.cipher));
        assert_eq!(request, unhex(t.client_request), "{}", t.cipher);

        let response = open(t, Side::Server, 1, t.server_app_record)
            .unwrap_or_else(|e| panic!("{}: server data: {e}", t.cipher));
        assert!(
            response.starts_with(b"HTTP/1.0 200"),
            "{}: {:?}",
            t.cipher,
            String::from_utf8_lossy(&response)
        );
    }
}

// ------------------------------------------------------------ negative controls

#[test]
fn the_oracle_notices_the_wrong_key_block_seed_order() {
    // Derive the key block with the master secret's "client then server" seed
    // order. Every trace must then fail to decrypt, or the test above proves
    // nothing about the order.
    for t in TRACES {
        let ms = master(t);
        let (cr, sr) = (random(t.client_random), random(t.server_random));
        let key_len = t.aead.key_len();
        let iv_len = rusty_tls::handrolled::record12::fixed_iv_len(t.aead);
        let mut raw = vec![0u8; 2 * key_len + 2 * iv_len];
        prf(
            t.hash,
            ms.as_bytes(),
            b"key expansion",
            &[&cr, &sr],
            &mut raw,
        ); // wrong order
        let key = &raw[..key_len];
        let iv = &raw[2 * key_len..2 * key_len + iv_len];
        let mut opener = Opener::new_at(t.aead, key, iv, 0).unwrap();
        assert_eq!(
            opener.open(&unhex(t.client_finished_record)),
            Err(RecordError::Decrypt),
            "{}",
            t.cipher
        );
    }
}

#[test]
fn the_oracle_notices_a_transcript_that_leaves_out_client_key_exchange() {
    for t in TRACES {
        let Some(pms) = t.pre_master_secret else {
            continue;
        };
        let all = t.transcript_to_client_key_exchange;
        let without_cke: Vec<u8> = all[..all.len() - 1].iter().flat_map(|m| unhex(m)).collect();
        let wrong =
            extended_master_secret(t.hash, &unhex(pms), &t.hash.hash(&without_cke)).unwrap();
        assert_ne!(
            wrong.as_bytes().to_vec(),
            unhex(t.master_secret),
            "{}",
            t.cipher
        );
    }
}

#[test]
fn the_oracle_notices_a_finished_over_the_wrong_messages() {
    for t in TRACES {
        let ms = master(t);
        let client_msg = open(t, Side::Client, 0, t.client_finished_record).unwrap();
        let received = split_finished(&client_msg);
        // The server's hash includes the client's Finished; the client's does not.
        let wrong_hash = t.hash.hash(&[transcript(t), client_msg.clone()].concat());
        assert!(
            !verify_finished(t.hash, &ms, Side::Client, &wrong_hash, received),
            "{}",
            t.cipher
        );
        // And the right hash with the wrong label.
        assert!(
            !verify_finished(
                t.hash,
                &ms,
                Side::Server,
                &t.hash.hash(&transcript(t)),
                received
            ),
            "{}",
            t.cipher
        );
    }
}

#[test]
fn the_wrong_prf_hash_cannot_be_used_by_accident() {
    // A SHA-256 transcript hash handed to a SHA-384 suite is refused outright
    // rather than deriving a secret the peer does not share.
    for t in TRACES.iter().filter(|t| t.hash == Hash::Sha384) {
        let wrong = Hash::Sha256.hash(&transcript(t));
        assert!(
            finished_verify_data(t.hash, &master(t), Side::Client, &wrong).is_err(),
            "{}",
            t.cipher
        );
        if let Some(pms) = t.pre_master_secret {
            assert!(
                extended_master_secret(t.hash, &unhex(pms), &wrong).is_err(),
                "{}",
                t.cipher
            );
        }
    }
}
