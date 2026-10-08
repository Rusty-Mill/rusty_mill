//! A BoGo shim for the hand-rolled TLS engine.
//!
//! BoringSSL's test runner (`ssl/test/runner`) drives a TLS stack through a
//! small command-line program: it listens on a loopback port, starts the shim
//! with flags describing the scenario, and plays the other end with a Go TLS
//! implementation that can be told to misbehave in hundreds of specific ways.
//! That makes it a source of hostile-peer cases written by people who have
//! spent years breaking TLS stacks, which is the thing this engine's own tests
//! cannot be.
//!
//! # Honest about what it does not do
//!
//! A flag this shim does not implement makes it exit with status 89, which
//! the runner reads as "unimplemented" and skips. That is how the supported
//! subset is selected, and it means a green run proves nothing about the
//! skipped tests: the report counts them.
//!
//! The engine has no insecure mode, so there is no way to turn off name or
//! chain verification. A client scenario that needs one is skipped, not
//! faked.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use rusty_tls::handrolled::client::{
    CipherSuite, ClientConfig, ClientError, ClientHandshake, Incoming,
};
use rusty_tls::handrolled::client12::{CipherSuite12, ClientConfig12, ClientHandshake12, Incoming12};
use rusty_tls::handrolled::kx::NamedGroup;
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::negotiate::{
    ClientConfigBoth, ClientHandshakeBoth, Established, ServerConfigBoth, ServerHandshakeBoth,
};
use rusty_tls::handrolled::path::{PathOptions, TrustAnchor};
use rusty_tls::handrolled::server::{ServerConfig, ServerHandshake};
use rusty_tls::handrolled::server12::{ServerConfig12, ServerHandshake12};
use rusty_tls::handrolled::sign::SigningKey;
use rusty_tls::handrolled::x509::Certificate;

/// The exit status the runner reads as "this shim cannot do that".
const UNIMPLEMENTED: i32 = 89;

const GROUPS: &[NamedGroup] = &[
    NamedGroup::X25519,
    NamedGroup::SecP256R1,
    NamedGroup::SecP384R1,
];

/// The groups to offer or accept: the scenario's `-curves`, else all three.
fn groups(config: &Config) -> &[NamedGroup] {
    if config.curves.is_empty() {
        GROUPS
    } else {
        &config.curves
    }
}

fn unimplemented(why: &str) -> ! {
    eprintln!("unimplemented: {why}");
    // Which flags keep tests out of the run is the question that decides what
    // to build next, so a run can be asked to record it.
    if let Ok(path) = std::env::var("BOGO_UNIMPLEMENTED_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{why}");
        }
    }
    exit(UNIMPLEMENTED)
}

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ERROR: {why}");
    exit(1)
}

// ---------------------------------------------------------------------------
// Flags
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Config {
    port: u16,
    shim_id: u64,
    ipv6: bool,
    server: bool,
    cert_file: Option<String>,
    key_file: Option<String>,
    trust_cert: Option<String>,
    host_name: Option<String>,
    min_version: u16,
    max_version: u16,
    expect_version: Option<u16>,
    no_tls: [bool; 4], // 1.0, 1.1, 1.2, 1.3
    /// `-curves`, in order; empty means the default list.
    curves: Vec<NamedGroup>,
    check_close_notify: bool,
}

const TLS12: u16 = 0x0303;
const TLS13: u16 = 0x0304;

fn parse_args() -> Config {
    let mut config = Config {
        min_version: 0x0301,
        max_version: TLS13,
        ..Config::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .unwrap_or_else(|| fail(format!("{name} needs a value")))
        };
        match flag.as_str() {
            // The runner asks whether an external handshaker is supported.
            "-is-handshaker-supported" => {
                println!("No");
                exit(0)
            }
            "-port" => config.port = value("-port").parse().unwrap_or_else(|_| fail("bad port")),
            "-shim-id" => config.shim_id = value("-shim-id").parse().unwrap_or(0),
            "-ipv6" => config.ipv6 = true,
            "-server" => config.server = true,
            "-cert-file" => config.cert_file = Some(value("-cert-file")),
            "-key-file" => config.key_file = Some(value("-key-file")),
            "-trust-cert" => config.trust_cert = Some(value("-trust-cert")),
            "-host-name" => config.host_name = Some(value("-host-name")),
            "-min-version" => config.min_version = value("-min-version").parse().unwrap_or(0),
            "-max-version" => config.max_version = value("-max-version").parse().unwrap_or(0),
            "-expect-version" => config.expect_version = value("-expect-version").parse().ok(),
            "-no-tls1" => config.no_tls[0] = true,
            "-no-tls11" => config.no_tls[1] = true,
            "-no-tls12" => config.no_tls[2] = true,
            "-no-tls13" => config.no_tls[3] = true,
            // The runner's asynchronous-callback modes change *how* a stack
            // gets its answers, not what it must conclude; this shim has no
            // callbacks, so there is nothing to defer and the outcome is the
            // one the runner checks.
            "-async" => {}
            "-check-close-notify" => config.check_close_notify = true,
            "-curves" => {
                let id = value("-curves").parse().unwrap_or(0);
                // A group the engine lacks (P-521, the hybrids) must not be
                // dropped quietly: the test would then be about a different
                // negotiation than the one it names.
                let group = NamedGroup::from_u16(id)
                    .unwrap_or_else(|| unimplemented(&format!("curve {id}")));
                config.curves.push(group);
            }
            other => unimplemented(&format!("flag {other}")),
        }
    }
    config
}

/// Which of TLS 1.2 and 1.3 the scenario allows. Anything older is outside the
/// engine, so a scenario that needs it is skipped.
fn versions(config: &Config) -> (bool, bool) {
    let allowed = |v: u16, off: bool| !off && config.min_version <= v && v <= config.max_version;
    (allowed(TLS12, config.no_tls[2]), allowed(TLS13, config.no_tls[3]))
}

// ---------------------------------------------------------------------------
// PEM and keys
// ---------------------------------------------------------------------------

fn base64(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// Every PEM block in a file: its label and its DER.
fn pem_blocks(path: &str) -> Vec<(String, Vec<u8>)> {
    // A scenario whose credential has no file (a deliberately unparseable
    // certificate, say) cannot be expressed through the engine's API.
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| unimplemented(&format!("{path}: {e}")));
    let mut out = Vec::new();
    let mut label = None::<String>;
    let mut body = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("-----BEGIN ") {
            label = Some(rest.trim_end_matches('-').to_string());
            body.clear();
        } else if line.starts_with("-----END ") {
            if let Some(label) = label.take() {
                out.push((label, base64(&body)));
            }
        } else if label.is_some() {
            body.push_str(line);
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A signing key from a PKCS#8 file, with the algorithm read from its OID.
fn signing_key(path: &str) -> SigningKey {
    let Some((_, der)) = pem_blocks(path).into_iter().next() else {
        fail("no key in the key file")
    };
    let head = &der[..der.len().min(64)];
    let key = if contains(head, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]) {
        SigningKey::rsa(&der)
    } else if contains(head, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]) {
        SigningKey::ecdsa_p256(&der)
    } else if contains(head, &[0x2b, 0x81, 0x04, 0x00, 0x22]) {
        SigningKey::ecdsa_p384(&der)
    } else if contains(head, &[0x2b, 0x65, 0x70]) {
        SigningKey::ed25519(&der)
    } else {
        unimplemented("a key type the engine does not sign with")
    };
    key.unwrap_or_else(|e| unimplemented(&format!("the engine rejects this key: {e}")))
}

fn certificates(path: &str) -> Vec<Vec<u8>> {
    pem_blocks(path)
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn path_options() -> PathOptions {
    PathOptions {
        time: now(),
        max_path_length: 8,
        max_signature_checks: 64,
        required_eku: None,
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

fn connect(config: &Config) -> TcpStream {
    let address = if config.ipv6 {
        format!("[::1]:{}", config.port)
    } else {
        format!("127.0.0.1:{}", config.port)
    };
    let mut stream = TcpStream::connect(&address).unwrap_or_else(|e| fail(format!("connect {address}: {e}")));
    stream
        .write_all(&config.shim_id.to_le_bytes())
        .unwrap_or_else(|e| fail(e));
    stream
}

fn read_record(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).ok()?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let mut record = header.to_vec();
    record.resize(5 + length, 0);
    stream.read_exact(&mut record[5..]).ok()?;
    Some(record)
}

/// Report a failure after sending any alert: close our side, then let the
/// peer finish reading before the process exits. Exiting at once resets the
/// connection, and a reset can discard the alert the runner is about to read.
fn fail_on(stream: &mut TcpStream, why: impl std::fmt::Display) -> ! {
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut sink = [0u8; 4096];
    while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
    fail(why)
}

fn send(stream: &mut TcpStream, bytes: &[u8]) {
    if !bytes.is_empty() {
        // The runner may already have closed after a refusal; that is its
        // verdict to give, not an error here.
        let _ = stream.write_all(bytes);
    }
}

// ---------------------------------------------------------------------------
// The handshake, whichever version and role
// ---------------------------------------------------------------------------

/// One side of a handshake in progress, erased over version and role.
trait Handshake {
    fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>, (String, Option<Vec<u8>>)>;
    fn is_finished(&self) -> bool;
    fn into_connection(self: Box<Self>) -> Result<Established, String>;
}

/// Erase one handshake machine behind [`Handshake`]. The six machines differ
/// only in type, in how a finished one becomes an [`Established`], and in
/// whether its alert encoder needs `&mut self`.
macro_rules! machine {
    ($wrapper:ident, $inner:ident, |$m:ident| $into:expr) => {
        struct $wrapper<'a>($inner<'a>);
        impl Handshake for $wrapper<'_> {
            fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>, (String, Option<Vec<u8>>)> {
                self.0.read_record(record).map_err(|error| {
                    let alert = self.0.alert_record(&error);
                    (format!("{error:?}: {error}"), alert)
                })
            }
            fn is_finished(&self) -> bool {
                self.0.is_finished()
            }
            fn into_connection(self: Box<Self>) -> Result<Established, String> {
                let $m = self.0;
                $into
            }
        }
    };
}

machine!(ServerSide, ServerHandshakeBoth, |m| m.into_connection().map_err(|e| e.to_string()));
machine!(ClientSide, ClientHandshakeBoth, |m| m.into_connection().map_err(|e| e.to_string()));
machine!(Server13, ServerHandshake, |m| m.into_connection().map(Established::Tls13).map_err(|e| e.to_string()));
machine!(Server12, ServerHandshake12, |m| m.into_connection().map(Established::Tls12).map_err(|e| e.to_string()));
machine!(Client13, ClientHandshake, |m| m.into_connection().map(Established::Tls13).map_err(|e| e.to_string()));
machine!(Client12, ClientHandshake12, |m| m.into_connection().map(Established::Tls12).map_err(|e| e.to_string()));

fn run_handshake(stream: &mut TcpStream, mut handshake: Box<dyn Handshake + '_>) -> Established {
    while !handshake.is_finished() {
        let Some(record) = read_record(stream) else {
            fail("the peer closed during the handshake")
        };
        match handshake.read_record(&record) {
            Ok(reply) => send(stream, &reply),
            Err((error, alert)) => {
                if let Some(alert) = alert {
                    send(stream, &alert);
                }
                fail_on(stream, error)
            }
        }
    }
    handshake.into_connection().unwrap_or_else(|e| fail(e))
}

// ---------------------------------------------------------------------------
// After the handshake: echo every byte, flipped, until the peer closes
// ---------------------------------------------------------------------------

enum Got {
    Data(Vec<u8>),
    Reply(Vec<u8>),
    Closed,
    Nothing,
}

fn read_app(connection: &mut Established, record: &[u8]) -> Result<Got, ClientError> {
    Ok(match connection {
        Established::Tls13(c) => match c.read(record)? {
            Incoming::Application(data) => Got::Data(data),
            Incoming::Reply(bytes) => Got::Reply(bytes),
            Incoming::Closed => Got::Closed,
            _ => Got::Nothing,
        },
        Established::Tls12(c) => match c.read(record)? {
            Incoming12::Application(data) => Got::Data(data),
            Incoming12::Reply(bytes) => Got::Reply(bytes),
            Incoming12::Closed => Got::Closed,
            _ => Got::Nothing,
        },
        _ => unimplemented("a connection kind this shim does not know"),
    })
}

fn write_app(connection: &mut Established, data: &[u8]) -> Result<Vec<u8>, String> {
    match connection {
        Established::Tls13(c) => c.write(data).map_err(|e| e.to_string()),
        Established::Tls12(c) => c.write(data).map_err(|e| e.to_string()),
        _ => unimplemented("a connection kind this shim does not know"),
    }
}

fn close(connection: &mut Established) -> Result<Vec<u8>, String> {
    match connection {
        Established::Tls13(c) => c.close().map_err(|e| e.to_string()),
        Established::Tls12(c) => c.close().map_err(|e| e.to_string()),
        _ => unimplemented("a connection kind this shim does not know"),
    }
}

fn exchange(config: &Config, stream: &mut TcpStream, mut connection: Established) {
    let mut closed = false;
    while let Some(record) = read_record(stream) {
        match read_app(&mut connection, &record) {
            Ok(Got::Data(mut data)) => {
                data.iter_mut().for_each(|b| *b ^= 0xff);
                match write_app(&mut connection, &data) {
                    Ok(bytes) => send(stream, &bytes),
                    Err(error) => fail(error),
                }
            }
            Ok(Got::Reply(bytes)) => send(stream, &bytes),
            Ok(Got::Closed) => {
                closed = true;
                break;
            }
            Ok(Got::Nothing) => {}
            Err(error) => {
                if let Some(alert) = connection.alert_record(&error) {
                    send(stream, &alert);
                }
                fail_on(stream, format!("{error:?}: {error}"))
            }
        }
    }
    if config.check_close_notify && !closed {
        fail("the peer ended the connection without a close_notify");
    }
    if let Ok(bytes) = close(&mut connection) {
        send(stream, &bytes);
    }
}

// ---------------------------------------------------------------------------

fn main() {
    let config = parse_args();
    let (tls12, tls13) = versions(&config);
    if !tls12 && !tls13 {
        unimplemented("a scenario that allows neither TLS 1.2 nor 1.3");
    }
    if config.server {
        serve(&config, tls12, tls13);
    } else {
        connect_client(&config, tls12, tls13);
    }
}

fn expect_version(config: &Config, connection: &Established) {
    let got = match connection {
        Established::Tls13(_) => TLS13,
        Established::Tls12(_) => TLS12,
        _ => 0,
    };
    if let Some(want) = config.expect_version {
        if want != got {
            fail(format!("wrong version: expected {want:#06x}, negotiated {got:#06x}"));
        }
    }
}

fn serve(config: &Config, tls12: bool, tls13: bool) {
    let cert = config
        .cert_file
        .as_deref()
        .unwrap_or_else(|| unimplemented("a server scenario without a certificate"));
    let key_file = config.key_file.as_deref().unwrap_or_else(|| unimplemented("no key"));
    let chain = certificates(cert);
    let key = signing_key(key_file);

    let server_13 = ServerConfig {
        certificates: &chain,
        key: &key,
        cipher_suites: CipherSuite::SUPPORTED,
        groups: groups(config),
        client_auth: None,
        tickets: None,
    };
    let server_12 = ServerConfig12 {
        certificates: &chain,
        key: &key,
        cipher_suites: CipherSuite12::SUPPORTED,
        groups: groups(config),
        client_auth: None,
    };
    let both = ServerConfigBoth {
        tls13: &server_13,
        tls12: &server_12,
    };

    let mut stream = connect(config);
    // A server restricted to one version is that version's standalone machine;
    // reaching the other would mean claiming an ability the scenario denied it.
    let handshake: Box<dyn Handshake + '_> = match (tls12, tls13) {
        (true, true) => Box::new(ServerSide(ServerHandshakeBoth::new(&both))),
        (false, true) => Box::new(Server13(ServerHandshake::new(&server_13))),
        _ => Box::new(Server12(
            ServerHandshake12::new(&server_12).unwrap_or_else(|e| fail(e)),
        )),
    };
    let connection = run_handshake(&mut stream, handshake);
    expect_version(config, &connection);
    exchange(config, &mut stream, connection);
}

fn connect_client(config: &Config, tls12: bool, tls13: bool) {
    if config.cert_file.is_some() || config.key_file.is_some() {
        unimplemented("a client certificate");
    }
    let trust = config
        .trust_cert
        .as_deref()
        .unwrap_or_else(|| unimplemented("a client scenario that does not verify the server"));
    let roots: Vec<Vec<u8>> = certificates(trust);
    let parsed: Vec<Certificate<'_>> = roots
        .iter()
        .map(|der| Certificate::parse(der).unwrap_or_else(|e| fail(format!("trust anchor: {e}"))))
        .collect();
    let anchors: Vec<TrustAnchor<'_>> = parsed.iter().map(TrustAnchor::from_certificate).collect();
    let name = config.host_name.as_deref().unwrap_or("test");

    let tls13_config = || ClientConfig {
        server_name: ServerName::Dns(name),
        anchors: &anchors,
        path: path_options(),
        groups: groups(config),
        cipher_suites: CipherSuite::SUPPORTED,
        identity: None,
        resumption: None,
    };
    let tls12_config = ClientConfig12 {
        server_name: ServerName::Dns(name),
        anchors: &anchors,
        path: path_options(),
        groups: groups(config),
        cipher_suites: CipherSuite12::SUPPORTED,
    };
    let tls13_single = tls13_config();
    let both = ClientConfigBoth::new(tls13_config(), CipherSuite12::SUPPORTED);
    let mut stream = connect(config);
    let (handshake, hello): (Box<dyn Handshake + '_>, Vec<u8>) = match (tls12, tls13) {
        (true, true) => {
            let (m, hello) = ClientHandshakeBoth::start(&both).unwrap_or_else(|e| fail(e));
            (Box::new(ClientSide(m)), hello)
        }
        (false, true) => {
            let (m, hello) = ClientHandshake::start(&tls13_single).unwrap_or_else(|e| fail(e));
            (Box::new(Client13(m)), hello)
        }
        _ => {
            let (m, hello) = ClientHandshake12::start(&tls12_config).unwrap_or_else(|e| fail(e));
            (Box::new(Client12(m)), hello)
        }
    };
    send(&mut stream, &hello);
    let connection = run_handshake(&mut stream, handshake);
    expect_version(config, &connection);
    exchange(config, &mut stream, connection);
}
