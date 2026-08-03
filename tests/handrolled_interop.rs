//! Interop against servers this code did not write — stage 3c-ii's remaining
//! shipping-bar item.
//!
//! ADR-0002's bar asks for "interop against real servers". Stage 3c-ii met
//! half of it: the client completes handshakes against `rustls` in memory.
//! This is the other half, over a real socket, against a stack chosen by
//! somebody else.
//!
//! # Why these tests are `#[ignore]`d
//!
//! They need the network, which makes them non-hermetic in a way every other
//! suite here is not: they can fail because of DNS, a firewall, or a server
//! that was reconfigured this morning. A CI job that flakes for reasons
//! unrelated to the code trains people to ignore it.
//!
//! `#[ignore]` rather than an environment-variable gate, deliberately. A gated
//! test that quietly passes when the variable is unset reports `ok` for a run
//! that did nothing — the exact failure mode that let a vacuous session-ticket
//! test survive a mutation in stage 3c-ii. An ignored test reports `ignored`,
//! which is the truth.
//!
//! ```text
//! cargo test --features handrolled-engine --test handrolled_interop -- --ignored --nocapture
//! ```
//!
//! # What a pass here actually proves
//!
//! More than the `rustls` interop does, in one specific way. Both are
//! independent of this code, but `rustls` shares this crate's *provenance* —
//! it is the engine this crate wraps, and its behaviour is what the
//! differential tests were written against. A server nobody here chose is a
//! third opinion.
//!
//! It also exercises paths the in-memory tests do not:
//!
//! - **Chains longer than two**, with intermediates that have to be ordered.
//! - **The real trust store**, hundreds of anchors deep, rather than a single
//!   generated root — so anchor selection has to actually select.
//! - **A real socket**, where a record arrives in pieces and the transport has
//!   to reassemble before the client ever sees one.
//!
//! It does *not*, in practice, exercise handshake-message reassembly across
//! records. That was the expectation when this file was written and the
//! measurement disagreed: every server tried sent its flight in a single
//! protected record, exactly as `rustls` does.
//! [`the_number_of_flight_records_is_reported`] prints the count rather than
//! asserting a split, because asserting one would be asserting something
//! about other people's servers. The gap is closed hermetically instead — see
//! `handrolled_client::a_flight_split_across_records_is_reassembled`.
//!
//! # An honest caveat about where this runs
//!
//! In a sandboxed environment, outbound TLS may be intercepted — the
//! certificate that arrives is issued by an egress gateway rather than by the
//! host named. [`the_peer_is_reported_so_interception_is_visible`] prints the
//! issuer for exactly this reason.
//!
//! That does not make the test worthless: the gateway is still a TLS 1.3
//! implementation nobody here wrote, and completing a handshake with it is
//! still a third opinion. It does mean a passing run is evidence about
//! *whatever answered*, and the output says which — so nobody reads a green
//! tick as proof of having reached a particular server when it is not.
//!
//! One consequence is worth spelling out, because it removes a test somebody
//! would otherwise expect to find here. An intercepting gateway mints a
//! certificate for whatever SNI it is handed — asking for
//! `not-the-server-we-dialled.invalid` returns a certificate whose SAN is
//! exactly that — so a client *correctly* accepts it, and "connect to a real
//! server under the wrong name and watch it be refused" cannot be written to
//! pass in both environments. What is checkable everywhere is the other
//! direction: whatever name the client accepted, the certificate must
//! actually carry it. That is
//! [`every_accepted_certificate_carries_the_name_that_was_asked_for`]. The
//! refusal direction is covered hermetically in `handrolled_client`, where
//! the peer's certificate can be chosen.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use platform::security::TrustAnchors;
use rusty_tls::handrolled::client::{
    record_length, CipherSuite, ClientConfig, ClientError, ClientHandshake, Connection, Incoming,
};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;

/// The hosts to try, overridable so someone can point this at their own.
fn hosts() -> Vec<String> {
    match std::env::var("RUSTY_TLS_INTEROP_HOSTS") {
        Ok(list) => list.split(',').map(|h| h.trim().to_string()).collect(),
        Err(_) => ["example.com", "www.google.com", "one.one.one.one"]
            .iter()
            .map(|h| (*h).to_string())
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Trust anchors, from the machine
// ---------------------------------------------------------------------------

/// The same backend selection `src/trust.rs` makes, for the same reason.
fn load_anchors() -> Vec<Vec<u8>> {
    #[cfg(target_os = "linux")]
    let backend = platform_linux::LinuxTrustAnchors;
    #[cfg(windows)]
    let backend = platform_windows::WindowsTrustAnchors;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    let backend = platform_bsd::BsdTrustAnchors;

    backend.load_anchors().unwrap_or_default()
}

/// Build [`TrustAnchor`]s from parsed roots.
///
/// A root's own `nameConstraints` travel with it — dropping them would
/// silently unconstrain a constrained root, which stage 2b-iii found by
/// mutation and which matters more here than anywhere else, because a real
/// trust store is where constrained roots actually live.
fn anchors<'a>(parsed: &'a [Certificate<'a>]) -> Vec<TrustAnchor<'a>> {
    parsed
        .iter()
        .map(|root| TrustAnchor {
            subject: root.subject(),
            public_key: root.subject_public_key_info(),
            name_constraints: root.extensions().name_constraints(),
        })
        .collect()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// A blocking transport
// ---------------------------------------------------------------------------

/// Reads whole TLS records off a socket.
///
/// This lives here rather than in the library on purpose. `handrolled` is not
/// the engine this crate ships, so giving it an IO adapter would grow the
/// surface of an experiment for no production benefit — and the module docs
/// already say that splitting a stream into records is the caller's job, with
/// `record_length` to find the boundaries. This is what "the caller's job"
/// looks like, in about thirty lines.
struct Transport {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl Transport {
    fn connect(host: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect((host, 443))?;
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_write_timeout(Some(Duration::from_secs(15)))?;
        Ok(Self {
            stream,
            buffer: Vec::new(),
        })
    }

    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.stream.write_all(bytes)
    }

    /// One whole record, reading more from the socket until there is one.
    fn record(&mut self) -> std::io::Result<Vec<u8>> {
        loop {
            if let Some(length) = record_length(&self.buffer) {
                if self.buffer.len() >= length {
                    return Ok(self.buffer.drain(..length).collect());
                }
            }
            let mut chunk = [0u8; 4096];
            let read = self.stream.read(&mut chunk)?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the peer closed the connection mid-record",
                ));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

/// What one interop attempt learned, for the report at the end.
struct Report {
    host: String,
    suite: CipherSuite,
    chain: usize,
    issuer: String,
    /// How many records the server's handshake flight arrived in.
    handshake_records: usize,
    body: String,
    /// The leaf, so a test can form its own opinion of what was accepted.
    leaf: Vec<u8>,
}

/// Everything that can go wrong, so a network failure reads differently from
/// a TLS failure. Conflating them would make a firewall look like a bug in
/// this code.
#[derive(Debug)]
enum InteropError {
    Network(std::io::Error),
    Tls(ClientError),
}

impl std::fmt::Display for InteropError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(err) => write!(f, "network: {err}"),
            Self::Tls(err) => write!(f, "TLS: {err}"),
        }
    }
}

/// Do a whole handshake and an HTTP request against `host`.
fn fetch(host: &str) -> Result<Report, InteropError> {
    let der = load_anchors();
    let parsed: Vec<Certificate<'_>> = der
        .iter()
        .filter_map(|bytes| Certificate::parse(bytes).ok())
        .collect();
    let anchors = anchors(&parsed);
    assert!(
        anchors.len() >= 10,
        "only {} trust anchors loaded — the machine's store is not being read",
        anchors.len()
    );

    let config = ClientConfig {
        server_name: ServerName::Dns(host),
        anchors: &anchors,
        path: PathOptions {
            time: now(),
            max_path_length: 8,
            max_signature_checks: 64,
            required_eku: None,
        },
        groups: &[
            NamedGroup::X25519,
            NamedGroup::SecP256R1,
            NamedGroup::SecP384R1,
        ],
        cipher_suites: CipherSuite::SUPPORTED,
        // No client certificate: these servers do not ask for one, and if one
        // ever did, an empty Certificate is the conforming answer.
        identity: None,
    };

    let mut transport = Transport::connect(host).map_err(InteropError::Network)?;
    let (mut client, hello) = ClientHandshake::start(&config).map_err(InteropError::Tls)?;
    transport.send(&hello).map_err(InteropError::Network)?;

    let mut handshake_records = 0usize;
    while !client.is_finished() {
        let record = transport.record().map_err(InteropError::Network)?;
        handshake_records += 1;
        let reply = client.read_record(&record).map_err(InteropError::Tls)?;
        transport.send(&reply).map_err(InteropError::Network)?;
    }

    let mut connection: Connection = client.into_connection().map_err(InteropError::Tls)?;

    let chain = connection.peer_certificates().len();
    let leaf_der = connection.peer_certificates()[0].clone();
    let suite = connection.cipher_suite();

    // A real request, because a handshake that completes and then cannot carry
    // a byte has not proved much.
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: rusty_tls-handrolled\r\n\r\n"
    );
    let record = connection
        .write(request.as_bytes())
        .map_err(InteropError::Tls)?;
    transport.send(&record).map_err(InteropError::Network)?;

    let mut body = Vec::new();
    for _ in 0..64 {
        let record = match transport.record() {
            Ok(record) => record,
            // A close at this point is the server honouring `Connection:
            // close`, not a failure.
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(InteropError::Network(err)),
        };
        match connection.read(&record) {
            Ok(Incoming::Application(data)) => {
                body.extend_from_slice(&data);
                if body.len() > 512 {
                    break;
                }
            }
            // An orderly close. Until alerts were parsed this arrived as an
            // unexpected content type and this loop broke on that, which was a
            // missing feature dressed up as correct behaviour.
            Ok(Incoming::Closed) => break,
            Ok(_) => {}
            Err(err) => return Err(InteropError::Tls(err)),
        }
    }

    Ok(Report {
        host: host.to_string(),
        suite,
        chain,
        issuer: issuer_common_name(&leaf_der),
        handshake_records,
        body: String::from_utf8_lossy(&body[..body.len().min(120)]).to_string(),
        leaf: leaf_der,
    })
}

/// The leaf issuer's CN, rendered well enough to see who actually answered.
fn issuer_common_name(der: &[u8]) -> String {
    let certificate = match Certificate::parse(der) {
        Ok(certificate) => certificate,
        Err(_) => return "<unparseable>".to_string(),
    };
    // The issuer is a DER `Name`; rather than decode it properly, pull out the
    // printable runs, which is enough to identify a gateway. This is a
    // diagnostic, not a parser.
    let issuer = certificate.issuer();
    let mut out = String::new();
    let mut run = String::new();
    for &byte in issuer {
        if byte.is_ascii_graphic() || byte == b' ' {
            run.push(byte as char);
        } else {
            if run.len() >= 4 {
                if !out.is_empty() {
                    out.push_str(", ");
                }
                out.push_str(run.trim());
            }
            run.clear();
        }
    }
    if run.len() >= 4 {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(run.trim());
    }
    out
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// A complete handshake and an HTTP exchange against a server nobody here
/// configured.
///
/// Every host must work. A suite that passed when one of three answered would
/// be reporting "the internet exists" rather than "this client interoperates".
#[test]
#[ignore = "needs the network; run with --ignored"]
fn the_client_interoperates_with_real_servers() {
    let mut failures = Vec::new();
    let mut reports = Vec::new();

    for host in hosts() {
        match fetch(&host) {
            Ok(report) => reports.push(report),
            Err(err) => failures.push(format!("  {host}: {err}")),
        }
    }

    for report in &reports {
        println!(
            "{}: {:?}, {} certs, flight in {} records, issuer [{}]\n    {}",
            report.host,
            report.suite,
            report.chain,
            report.handshake_records,
            report.issuer,
            report.body.lines().next().unwrap_or("<no body>")
        );
        assert!(
            report.body.starts_with("HTTP/1."),
            "{}: the response was not HTTP: {:?}",
            report.host,
            report.body
        );
        assert!(report.chain >= 1, "{}: an empty chain", report.host);
    }

    assert!(
        failures.is_empty(),
        "{} of {} hosts failed:\n{}",
        failures.len(),
        reports.len() + failures.len(),
        failures.join("\n")
    );
    assert!(!reports.is_empty(), "no hosts were tried");
}

/// Print who actually answered, so an intercepting proxy cannot be mistaken
/// for the host that was asked for.
///
/// This is a diagnostic rather than an assertion on purpose. Interception is
/// not a failure — a gateway is still an independent TLS 1.3 implementation,
/// which is what the interop is for — but a run that was intercepted proves
/// something different from one that was not, and the difference should not
/// have to be inferred.
#[test]
#[ignore = "needs the network; run with --ignored"]
fn the_peer_is_reported_so_interception_is_visible() {
    for host in hosts() {
        match fetch(&host) {
            Ok(report) => println!("{}: leaf issued by [{}]", report.host, report.issuer),
            Err(err) => println!("{host}: {err}"),
        }
    }
}

/// How many records each server's handshake flight arrived in — reported,
/// not asserted.
///
/// This file was written expecting real servers to split a flight across
/// records, which would have been the first thing to exercise the client's
/// reassembly buffer for real. The measurement disagreed: every server tried
/// sends its flight in one protected record, exactly as `rustls` does.
///
/// So this prints the number instead of asserting a split. Asserting one
/// would be asserting a fact about other people's servers, which can change
/// without notice and has nothing to do with whether this code is correct.
/// The reassembly path is covered deterministically in `handrolled_client`
/// instead, by a test server that splits its flight on purpose.
#[test]
#[ignore = "needs the network; run with --ignored"]
fn the_number_of_flight_records_is_reported() {
    let mut tried = 0usize;
    for host in hosts() {
        if let Ok(report) = fetch(&host) {
            tried += 1;
            println!(
                "{}: handshake arrived in {} records ({} certs, {:?})",
                report.host, report.handshake_records, report.chain, report.suite
            );
        }
    }
    assert!(tried > 0, "no host answered");
}

/// Whatever name the client accepted, the certificate it accepted must
/// actually carry that name.
///
/// The direction that can be checked in every environment. Its mirror image —
/// connect under a name the certificate cannot have, and watch the client
/// refuse — is untestable behind an intercepting gateway, which mints a
/// certificate for whatever SNI it is given. See the module docs; the refusal
/// direction is covered hermetically in `handrolled_client`.
///
/// The check here deliberately does not call this crate's own name matcher,
/// which is what the client used to decide. Re-running the same function
/// would only prove it agrees with itself.
#[test]
#[ignore = "needs the network; run with --ignored"]
fn every_accepted_certificate_carries_the_name_that_was_asked_for() {
    use rusty_tls::handrolled::x509::GeneralName;

    let mut checked = 0usize;
    for host in hosts() {
        let Ok(report) = fetch(&host) else {
            continue;
        };
        let leaf = Certificate::parse(&report.leaf).expect("the leaf parses");

        let names: Vec<String> = leaf
            .extensions()
            .subject_alt_names()
            .filter_map(|name| match name {
                Ok(GeneralName::DnsName(dns)) => Some(dns.to_ascii_lowercase()),
                _ => None,
            })
            .collect();

        let wanted = host.to_ascii_lowercase();
        // An exact name, or a wildcard covering exactly its first label —
        // spelled out here rather than delegated, so this is an independent
        // opinion about what the certificate says.
        let covered = names.iter().any(|name| {
            *name == wanted
                || name.strip_prefix("*.").is_some_and(|suffix| {
                    wanted
                        .split_once('.')
                        .is_some_and(|(_, parent)| parent == suffix)
                })
        });

        assert!(
            covered,
            "{host}: accepted a certificate whose names are {names:?}"
        );
        println!("{host}: covered by {names:?}");
        checked += 1;
    }

    assert!(checked > 0, "no host answered, so nothing was checked");
}
