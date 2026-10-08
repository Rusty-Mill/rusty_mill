//! Coverage-guided fuzzing of the TLS 1.2 client handshake.
//!
//! The input is a sequence of records, each introduced by a type octet and a
//! length, fed to a fresh handshake in order. Two properties:
//!
//! 1. **Nothing a peer sends makes the client panic.** The remote controls every
//!    byte, so a panic here is a denial of service reachable by anyone who can
//!    complete a TCP handshake. The records reach ServerHello, Certificate (and
//!    with it the whole X.509 parser and path validator), ServerKeyExchange and
//!    the reassembly buffer.
//! 2. **No input completes a handshake.** Finishing needs a `Finished` that
//!    verifies, which needs keys derived from a key exchange with a private key
//!    the fuzzer does not hold. If any input ever completes, the client has
//!    accepted something it should not.
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run client12

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::client12::{CipherSuite12, ClientConfig12, ClientHandshake12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};

fuzz_target!(|data: &[u8]| {
    let anchors: [TrustAnchor<'_>; 0] = [];
    let config = ClientConfig12 {
        server_name: ServerName::Dns("fuzz.example"),
        anchors: &anchors,
        path: PathOptions {
            time: 1_800_000_000,
            ..PathOptions::default()
        },
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1, NamedGroup::SecP384R1],
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    let Ok((mut client, _hello)) = ClientHandshake12::start(&config) else {
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
        let _ = client.read_record(&record);
        rest = next;
    }
    assert!(!client.is_finished(), "arbitrary bytes completed a handshake");
});
