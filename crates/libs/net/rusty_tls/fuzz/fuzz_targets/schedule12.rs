//! Coverage-guided fuzzing of the TLS 1.2 key derivation.
//!
//! The derivations are pure functions of attacker-influenced bytes (the hello
//! randoms and, indirectly, the transcript), so the property is not "no panic"
//! alone but that they stay self-consistent for every input:
//!
//! 1. a seed given in pieces equals the same seed joined,
//! 2. a short output is a prefix of a longer one,
//! 3. the key block is exactly the PRF's output, partitioned,
//! 4. a `Finished` this crate computes always verifies, and any one-bit change
//!    to it never does.
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run schedule12

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::record::Aead;
use rusty_tls::handrolled::schedule::Hash;
use rusty_tls::handrolled::schedule12::{
    extended_master_secret, finished_verify_data, key_block, prf, verify_finished, Side,
    MASTER_SECRET_LEN, RANDOM_LEN,
};

fuzz_target!(|data: &[u8]| {
    let [selector, split_a, split_b, out_len, rest @ ..] = data else {
        return;
    };
    let hash = if selector & 1 == 0 { Hash::Sha256 } else { Hash::Sha384 };
    let alg = [Aead::Aes128Gcm, Aead::Aes256Gcm, Aead::ChaCha20Poly1305][usize::from(selector >> 1) % 3];

    // Carve the input into secret, label and seed pieces.
    let a = usize::from(*split_a) % (rest.len() + 1);
    let (secret, tail) = rest.split_at(a);
    let b = usize::from(*split_b) % (tail.len() + 1);
    let (label, seed) = tail.split_at(b);
    let n = usize::from(*out_len) * 3; // up to 765 octets, past several blocks

    // 1. Pieced == joined.
    let mid = seed.len() / 2;
    let mut pieced = vec![0u8; n];
    prf(hash, secret, label, &[&seed[..mid], &seed[mid..]], &mut pieced);
    let mut joined = vec![0u8; n];
    prf(hash, secret, label, &[seed], &mut joined);
    assert_eq!(pieced, joined);

    // 2. Prefix property.
    let mut shorter = vec![0u8; n / 2];
    prf(hash, secret, label, &[seed], &mut shorter);
    assert_eq!(shorter[..], joined[..n / 2]);

    // 3. The key block is the PRF's output, partitioned.
    let mut master = [0u8; MASTER_SECRET_LEN];
    for (i, byte) in master.iter_mut().enumerate() {
        *byte = rest.get(i).copied().unwrap_or(i as u8);
    }
    let ms = rusty_tls::handrolled::schedule12::MasterSecret::from_bytes(master);
    let mut client_random = [0u8; RANDOM_LEN];
    let mut server_random = [0u8; RANDOM_LEN];
    for i in 0..RANDOM_LEN {
        client_random[i] = seed.get(i).copied().unwrap_or(1);
        server_random[i] = seed.get(RANDOM_LEN + i).copied().unwrap_or(2);
    }
    let kb = key_block(hash, alg, &ms, &client_random, &server_random);
    let key_len = alg.key_len();
    let mut raw = vec![0u8; kb.client_write_key.len() * 2 + kb.client_write_iv.len() * 2];
    prf(hash, &master, b"key expansion", &[&server_random, &client_random], &mut raw);
    assert_eq!(kb.client_write_key, raw[..key_len]);
    assert_eq!(kb.server_write_key, raw[key_len..2 * key_len]);

    // 4. Finished round-trips and rejects any bit flip.
    let handshake_hash = hash.hash(rest);
    for side in [Side::Client, Side::Server] {
        let good = finished_verify_data(hash, &ms, side, &handshake_hash).expect("right-length hash");
        assert!(verify_finished(hash, &ms, side, &handshake_hash, &good));
        let mut bad = good;
        bad[usize::from(*out_len) % good.len()] ^= 1 << (selector % 8);
        assert!(!verify_finished(hash, &ms, side, &handshake_hash, &bad));
    }

    // The EMS derivation must never panic on an empty or odd pre-master secret.
    let _ = extended_master_secret(hash, secret, &handshake_hash);
});
