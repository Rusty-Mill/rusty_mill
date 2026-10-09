//! The native engine against the real internet: Google's sign-in endpoints.
//!
//! These are the peers the first candidate consumer (`rleval-app`'s OIDC
//! transport) talks to, with the trust it would use (the OS anchors), so this
//! is the evidence that matters before anything is wired: not "agrees with
//! rustls on a test certificate" but "completes with a server we do not control".
//!
//! `#[ignore]`d: it needs outbound network and a readable OS trust store, and
//! a third party's server is not something a required CI job should depend on.
//! Run by hand:
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_live -- --ignored --nocapture
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

fn status_line(response: &[u8]) -> String {
    String::from_utf8_lossy(response)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
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
    while let Ok(record) = read_record(&mut socket) {
        let data = match &mut connection {
            Established::Tls13(c) => match c.read(&record).map_err(|e| format!("read: {e}"))? {
                Incoming::Application(d) => Some(d),
                Incoming::Closed => break,
                _ => None,
            },
            Established::Tls12(c) => match c.read(&record).map_err(|e| format!("read: {e}"))? {
                Incoming12::Application(d) => Some(d),
                Incoming12::Closed => break,
                _ => None,
            },
            _ => return Err("a connection kind this test does not know".into()),
        };
        if let Some(data) = data {
            response.extend_from_slice(&data);
        }
        if response.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    Ok(Outcome {
        version,
        group,
        scheme,
        status_line: status_line(&response),
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
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => response.extend_from_slice(&chunk[..n]),
        }
    }
    Ok(status_line(&response))
}

#[test]
#[ignore = "needs outbound network and the OS trust store; run by hand"]
fn the_native_engine_completes_with_googles_sign_in_endpoints() {
    let mut failures = Vec::new();
    for host in hosts() {
        let reference = through_rustls(&host);
        match (native(&host), &reference) {
            (Ok(outcome), _) => {
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
                if !outcome.status_line.starts_with("HTTP/1.") {
                    failures.push(format!("{host}: no HTTP response: {outcome:?}"));
                }
            }
            (Err(native), Ok(_)) => {
                // rustls got through and the engine did not: an engine finding.
                failures.push(format!(
                    "{host}: native engine failed ({native}); rustls ok"
                ));
            }
            (Err(native), Err(rustls)) => {
                println!("{host}: unreachable either way (native: {native}; rustls: {rustls})");
                failures.push(format!("{host}: unreachable ({rustls})"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
