//! Coverage-guided fuzzing of the two-version client's first message.
//!
//! The same record framing as the `client12` target, into
//! `ClientHandshakeBoth`: the part that reads a ServerHello well enough to
//! choose a version, and then whichever machine it chose. Nothing a server
//! sends may make it panic, and no input may complete a handshake.
//!
//!   RUSTFLAGS='--cfg rusty_tls_handrolled' cargo +nightly fuzz run negotiate_client

#![no_main]

use libfuzzer_sys::fuzz_target;
use rusty_tls::handrolled::client::{CipherSuite, ClientConfig};
use rusty_tls::handrolled::client12::CipherSuite12;
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{ClientConfigBoth, ClientHandshakeBoth};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};

fuzz_target!(|data: &[u8]| {
    let anchors: [TrustAnchor<'_>; 0] = [];
    let tls13 = ClientConfig {
        server_name: ServerName::Dns("fuzz.example"),
        anchors: &anchors,
        path: PathOptions {
            time: 1_800_000_000,
            ..PathOptions::default()
        },
        groups: &[NamedGroup::X25519, NamedGroup::SecP256R1, NamedGroup::SecP384R1],
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
        alpn: &[],
    };
    let config = ClientConfigBoth::new(tls13, CipherSuite12::SUPPORTED);
    let Ok((mut client, _hello)) = ClientHandshakeBoth::start(&config) else {
        return;
    };

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
