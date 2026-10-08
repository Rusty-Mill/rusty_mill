//! Coverage-guided fuzzing of the two-version server's first message.
//!
//! The same record framing as the `server12` target, into
//! `ServerHandshakeBoth`: the part that reads a ClientHello well enough to
//! choose a version, and then whichever machine it chose. Two properties:
//!
//! 1. **Nothing a client sends makes the server panic.** Here that includes
//!    the TLS 1.3 server, which no other target reaches.
//! 2. **No input completes a handshake.**
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run negotiate_server

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::client::CipherSuite;
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::negotiate::{ServerConfigBoth, ServerHandshakeBoth};
use rusty_tls::handrolled::server::ServerConfig;
use rusty_tls::handrolled::server12::ServerConfig12;
use rusty_tls::handrolled::sign::SigningKey;

/// A throwaway P-256 key made for these targets. It protects nothing.
const KEY: &str = "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b02010104203fc3a3d357eeafcd8a626f6e4ab1db838e110b5dd9fade44ffd4ad66c9f9bf4aa144034200047f3c6e62da38f65581fb064a8619ef5361ee6fb55c737b28cfcfd6ee98012e7dddcf4f5e89955dd1a0ca16fdcd45cee0af693857938a887256f5893f4b8cf9e6";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let Ok(key) = SigningKey::ecdsa_p256(&unhex(KEY)) else {
        return;
    };
    let certificates = vec![vec![0x30, 0x00]];
    let groups = [NamedGroup::X25519, NamedGroup::SecP256R1, NamedGroup::SecP384R1];
    let tls13 = ServerConfig {
        certificates: &certificates,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: &groups,
        client_auth: None,
        tickets: None,
    };
    let tls12 = ServerConfig12 {
        certificates: &certificates,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &groups,
        client_auth: None,
    };
    let config = ServerConfigBoth {
        tls13: &tls13,
        tls12: &tls12,
    };
    let mut server = ServerHandshakeBoth::new(&config);

    let mut rest = data;
    while let [typ, hi, lo, tail @ ..] = rest {
        let len = (usize::from(*hi) << 8 | usize::from(*lo)).min(tail.len());
        let (payload, next) = tail.split_at(len);
        let mut record = vec![20 + (typ % 4), 3, 3];
        record.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        record.extend_from_slice(payload);
        if let Err(error) = server.read_record(&record) {
            let _ = server.alert_record(&error);
        }
        rest = next;
    }
    assert!(!server.is_finished(), "arbitrary bytes completed a handshake");
});
