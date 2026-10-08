//! Coverage-guided fuzzing of the TLS 1.2 server handshake.
//!
//! The input is a sequence of records, each introduced by a type octet and a
//! length, fed to a fresh server in order. Two properties:
//!
//! 1. **Nothing a client sends makes the server panic.** A server answers
//!    whoever connects, so a panic here is a denial of service reachable by
//!    anyone who can open a socket. The records reach the ClientHello parser,
//!    the negotiation, the reassembly buffer and, with client authentication
//!    on, the whole X.509 parser and path validator.
//! 2. **No input completes a handshake.** Finishing needs a `Finished` that
//!    verifies, which needs the server's ephemeral key, which the fuzzer never
//!    sees. If any input ever completes, the server has accepted something it
//!    should not.
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run server12

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::ClientAuth;
use rusty_tls::handrolled::server12::{ServerConfig12, ServerHandshake12};
use rusty_tls::handrolled::sign::SigningKey;

/// A throwaway P-256 key made for this target. It protects nothing.
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
    // The server never parses its own certificate, so any bytes will do.
    let certificates = vec![vec![0x30, 0x00]];
    let anchors: [TrustAnchor<'_>; 0] = [];
    let auth = ClientAuth {
        anchors: &anchors,
        path: PathOptions {
            time: 1_800_000_000,
            ..PathOptions::default()
        },
        required: false,
    };
    // The first input byte chooses whether client authentication is on, so
    // both state machines get covered.
    let (flag, data) = data.split_first().map_or((0, &[][..]), |(f, d)| (*f, d));
    let config = ServerConfig12 {
        certificates: &certificates,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1, NamedGroup::SecP384R1],
        client_auth: (flag & 1 == 1).then_some(&auth),
    };
    let Ok(mut server) = ServerHandshake12::new(&config) else {
        return;
    };

    // Records: [type][len_hi][len_lo][payload...]. Wrapped in a TLS header with
    // a plausible version so the content, not the framing, is what is parsed.
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
