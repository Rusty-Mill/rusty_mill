//! The native engine against the real internet: Google's sign-in endpoints.
//!
//! These are the peers the first candidate consumer (`rleval-app`'s OIDC
//! transport) talks to, with the trust it would use (the OS anchors), so this
//! is the evidence that matters before anything is wired: not "agrees with
//! rustls on a test certificate" but "completes with a server we do not control".
//!
//! `#[ignore]`d: it needs outbound network and a readable OS trust store, and
//! a third party's server is not something a required CI job should depend on.
//! Run by hand. The cfg is required: without it the crate-level gate removes
//! every test and cargo reports a successful run of zero tests, so check that
//! the output says `1 passed`:
//!
//! ```text
//! RUSTFLAGS='--cfg rusty_tls_handrolled' cargo test -p rusty_tls \
//!     --features handrolled-engine --test handrolled_live -- --ignored --nocapture
//! ```
//!
//! `RUSTY_TLS_LIVE_HOSTS` (comma-separated) replaces the default host list.

#![cfg(all(
    feature = "handrolled-engine",
    rusty_tls_handrolled,
    target_os = "linux"
))]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use platform::security::TrustAnchors;
use rusty_tls::handrolled::client::{CipherSuite, ClientConfig, Incoming};
use rusty_tls::handrolled::client12::{CipherSuite12, Incoming12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, Version,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;
use rusty_tls::{TlsStream, TrustPolicy};

const DEFAULT_HOSTS: &[&str] = &[
    "accounts.google.com",
    "oauth2.googleapis.com",
    "www.googleapis.com",
];

const GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

fn hosts() -> Vec<String> {
    match std::env::var("RUSTY_TLS_LIVE_HOSTS") {
        Ok(list) => list.split(',').map(|h| h.trim().to_string()).collect(),
        Err(_) => DEFAULT_HOSTS.iter().map(|h| h.to_string()).collect(),
    }
}

fn now() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs();
    i64::try_from(secs).expect("the clock fits an i64")
}

fn request(host: &str) -> Vec<u8> {
    format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: */*\r\n\r\n")
        .into_bytes()
}

fn read_record(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let mut record = header.to_vec();
    record.resize(5 + length, 0);
    stream.read_exact(&mut record[5..])?;
    Ok(record)
}

/// What the engine reports about one completed conversation.
#[derive(Debug)]
struct Outcome {
    version: Version,
    group: NamedGroup,
    scheme: Option<u16>,
    status_line: String,
    /// The leaf names a TLS-intercepting egress proxy as its issuer. Then this
    /// shows the engine against the proxy's server, not the host asked for.
    intercepted: bool,
}

/// The OS trust anchors, as DER, from the same backend `TrustPolicy::System` uses.
fn system_anchors_der() -> Vec<Vec<u8>> {
    platform_linux::LinuxTrustAnchors
        .load_anchors()
        .expect("the OS trust store is readable")
}

/// The status line of a response whose headers are complete.
///
/// A short read, an EOF or a timeout before the blank line is an error and not
/// an empty status: two connections that both said nothing must not look alike.
fn response_status(response: &[u8], ended: &str) -> Result<String, String> {
    if !response.windows(4).any(|w| w == b"\r\n\r\n") {
        return Err(format!(
            "the response ended before complete headers ({} bytes; {ended})",
            response.len()
        ));
    }
    let line = String::from_utf8_lossy(response)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    if !line.starts_with("HTTP/1.") {
        return Err(format!("not an HTTP response: {line:?}"));
    }
    Ok(line)
}

/// Whether the native result and the rustls reference agree on a real response.
///
/// Two failures, or two empty answers, are not parity.
fn differential(
    native: &Result<String, String>,
    reference: &Result<String, String>,
) -> Result<(), String> {
    match (native, reference) {
        (Ok(n), Ok(r)) if n == r && n.starts_with("HTTP/1.") => Ok(()),
        (Ok(n), Ok(r)) if n == r => Err(format!("equal but not an HTTP response: {n:?}")),
        (Ok(n), Ok(r)) => Err(format!("native got {n:?}, rustls got {r:?}")),
        (Ok(n), Err(r)) => Err(format!(
            "no reference (rustls failed: {r}); native got {n:?}"
        )),
        (Err(n), Ok(_)) => Err(format!("native engine failed ({n}); rustls ok")),
        (Err(n), Err(r)) => Err(format!(
            "failed on both, which is not parity (native: {n}; rustls: {r})"
        )),
    }
}

/// One full conversation with `host` on the native engine.
fn native(host: &str) -> Result<Outcome, String> {
    let ders = system_anchors_der();
    // Anchors the parser cannot read are skipped, as a trust store with an odd
    // entry should not stop the rest from being used.
    let certificates: Vec<Certificate<'_>> = ders
        .iter()
        .filter_map(|d| Certificate::parse(d).ok())
        .collect();
    let anchors: Vec<TrustAnchor<'_>> = certificates
        .iter()
        .map(TrustAnchor::from_certificate)
        .collect();
    assert!(
        anchors.len() > 50,
        "only {} usable anchors: is the trust store present?",
        anchors.len()
    );

    let tls13 = ClientConfig {
        server_name: ServerName::Dns(host),
        anchors: &anchors,
        path: PathOptions {
            time: now(),
            ..PathOptions::default()
        },
        groups: GROUPS,
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
        alpn: &[],
    };
    let config = ClientConfigBoth::new(tls13, CipherSuite12::SUPPORTED);

    let mut socket = TcpStream::connect((host, 443)).map_err(|e| format!("connect: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;

    let (mut handshake, hello) =
        ClientHandshakeBoth::start(&config).map_err(|e| format!("start: {e}"))?;
    socket.write_all(&hello).map_err(|e| e.to_string())?;
    while !handshake.is_finished() {
        let record = read_record(&mut socket).map_err(|e| format!("handshake read: {e}"))?;
        let reply = handshake
            .read_record(&record)
            .map_err(|e| format!("handshake: {e}"))?;
        if !reply.is_empty() {
            socket.write_all(&reply).map_err(|e| e.to_string())?;
        }
    }
    let version = handshake.version().expect("finished");
    let mut connection = handshake
        .into_connection()
        .map_err(|e| format!("connection: {e}"))?;
    let leaf = match &connection {
        Established::Tls13(c) => c.peer_certificates().first().cloned(),
        Established::Tls12(c) => c.peer_certificates().first().cloned(),
        _ => None,
    };
    let intercepted = leaf
        .as_deref()
        .is_some_and(|der| der.windows(14).any(|w| w == b"Egress Gateway"));
    let group = connection.key_exchange_group();
    let scheme = connection.peer_signature_scheme().map(|s| s.0);

    let sealed = match &mut connection {
        Established::Tls13(c) => c.write(&request(host)),
        Established::Tls12(c) => c.write(&request(host)),
        _ => return Err("a connection kind this test does not know".into()),
    }
    .map_err(|e| format!("write: {e}"))?;
    socket.write_all(&sealed).map_err(|e| e.to_string())?;

    let mut response = Vec::new();
    let mut ended = String::from("headers complete");
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        let record = match read_record(&mut socket) {
            Ok(record) => record,
            Err(e) => {
                ended = format!("transport: {e}");
                break;
            }
        };
        let data = match &mut connection {
            Established::Tls13(c) => match c.read(&record).map_err(|e| format!("read: {e}"))? {
                Incoming::Application(d) => Some(d),
                Incoming::Closed => {
                    ended = "close_notify".into();
                    break;
                }
                _ => None,
            },
            Established::Tls12(c) => match c.read(&record).map_err(|e| format!("read: {e}"))? {
                Incoming12::Application(d) => Some(d),
                Incoming12::Closed => {
                    ended = "close_notify".into();
                    break;
                }
                _ => None,
            },
            _ => return Err("a connection kind this test does not know".into()),
        };
        if let Some(data) = data {
            response.extend_from_slice(&data);
        }
    }
    let status_line = response_status(&response, &ended)?;
    Ok(Outcome {
        version,
        group,
        scheme,
        status_line,
        intercepted,
    })
}

/// The same conversation through the shipped rustls-backed `TlsStream`, so a
/// failure of the native engine can be told apart from an unreachable host.
fn through_rustls(host: &str) -> Result<String, String> {
    let tcp = TcpStream::connect((host, 443)).map_err(|e| format!("connect: {e}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;
    let mut tls =
        TlsStream::new(tcp, host, &TrustPolicy::System).map_err(|e| format!("tls: {e}"))?;
    tls.write_all(&request(host)).map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut ended = String::from("headers complete");
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut chunk) {
            Ok(0) => {
                ended = "eof".into();
                break;
            }
            Err(e) => {
                ended = format!("transport: {e}");
                break;
            }
            Ok(n) => response.extend_from_slice(&chunk[..n]),
        }
    }
    response_status(&response, &ended)
}

#[test]
#[ignore = "needs outbound network and the OS trust store; run by hand"]
fn the_native_engine_completes_with_googles_sign_in_endpoints() {
    let mut failures = Vec::new();
    for host in hosts() {
        let reference = through_rustls(&host);
        let outcome = native(&host);
        if let Ok(outcome) = &outcome {
            println!(
                "{host}: native {:?}, group {:?}, peer scheme {:?}, {:?}; rustls {reference:?}",
                outcome.version, outcome.group, outcome.scheme, outcome.status_line
            );
            if outcome.intercepted {
                println!(
                    "{host}: NOTE the certificate came from an intercepting proxy; \
                     this is not evidence about {host} itself"
                );
            }
        }
        let status = outcome
            .as_ref()
            .map(|o| o.status_line.clone())
            .map_err(String::clone);
        if let Err(why) = differential(&status, &reference) {
            failures.push(format!("{host}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// The harness itself. These run without a network: a live check that can pass
// on two empty answers proves nothing.
// ---------------------------------------------------------------------------

#[test]
fn a_response_without_complete_headers_is_an_error_not_an_empty_status() {
    for partial in [
        &b""[..],
        b"HTTP/1.1 200 OK\r\nHost: x",
        b"HTTP/1.1 200 OK\r\n",
    ] {
        assert!(
            response_status(partial, "eof").is_err(),
            "{:?} must not yield a status",
            String::from_utf8_lossy(partial)
        );
    }
    assert!(response_status(b"SSH-2.0-x\r\n\r\n", "headers complete").is_err());
    assert_eq!(
        response_status(b"HTTP/1.1 200 OK\r\nA: b\r\n\r\nbody", "headers complete"),
        Ok("HTTP/1.1 200 OK".to_string())
    );
}

#[test]
fn equal_empty_answers_and_matching_failures_are_not_parity() {
    let ok = |s: &str| -> Result<String, String> { Ok(s.to_string()) };
    let err = |s: &str| -> Result<String, String> { Err(s.to_string()) };
    assert!(differential(&ok(""), &ok("")).is_err(), "empty == empty");
    assert!(differential(&ok("junk"), &ok("junk")).is_err(), "not HTTP");
    assert!(
        differential(&err("eof"), &err("eof")).is_err(),
        "both failed"
    );
    assert!(differential(&ok("HTTP/1.1 200 OK"), &err("eof")).is_err());
    assert!(differential(&err("eof"), &ok("HTTP/1.1 200 OK")).is_err());
    assert!(differential(&ok("HTTP/1.1 200 OK"), &ok("HTTP/1.1 404 Not Found")).is_err());
    assert_eq!(
        differential(&ok("HTTP/1.1 200 OK"), &ok("HTTP/1.1 200 OK")),
        Ok(())
    );
}
