//! `TrustPolicy::System` end-to-end, through `platform::security::
//! TrustAnchors` (rusty_tls#24).
//!
//! Before #24, `System` had **no test at all** — it read the developer's
//! own machine trust store, which is neither hermetic nor assertable, so
//! the one `TrustPolicy` variant every real consumer uses was the one
//! variant nothing covered. Swapping its implementation out from under
//! `rustls-native-certs` made that gap worth closing rather than
//! inheriting.
//!
//! `SSL_CERT_FILE` is what makes it closeable: `platform`'s Linux and
//! BSD backends honor it ahead of every distro path, so a test can point
//! the OS trust store at a CA it generated itself and then assert a real
//! handshake against it. That exercises the whole chain the swap
//! touched — env probe, PEM decode, DER across the `platform` boundary,
//! `RootCertStore`, chain verification — with no dependence on what this
//! machine happens to trust.
//!
//! ## Why this file is single-threaded
//!
//! `SSL_CERT_FILE` is process-global. `cargo test` runs a file's tests on
//! parallel threads by default, so two tests here could race on it. Every
//! test in this file therefore lives behind one `#[test]` — the whole
//! file is one test — rather than relying on a mutex that a future
//! contributor might not notice they need.
//!
//! Windows and macOS are excluded: neither honors `SSL_CERT_FILE` (they
//! read the ROOT store and Security.framework respectively), so there is
//! no way to stage a hermetic anchor set for them here. Their real
//! backends are covered by rustils' own parity suite on real runners.

#![cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{TlsStream, TrustPolicy};

/// PEM-encode a DER certificate — the format an OS trust store holds,
/// and therefore what `platform` expects to decode back.
fn pem(der: &CertificateDer<'static>) -> String {
    const WRAP: usize = 64;
    let b64 = base64(der.as_ref());
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(WRAP) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// Standard-alphabet base64. Hand-rolled for the same reason `platform`'s
/// decoder is: this crate's dependency set doesn't include one, and a
/// test helper is not worth a new dependency.
fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn spawn_echo_server(
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .expect("valid test cert/key"),
    );
    let handle = thread::spawn(move || {
        let Ok((tcp, _)) = listener.accept() else {
            return;
        };
        let Ok(conn) = ServerConnection::new(config) else {
            return;
        };
        let mut tls = StreamOwned::new(conn, tcp);
        let mut buf = [0u8; 1024];
        if let Ok(n) = tls.read(&mut buf) {
            let _ = tls.write_all(&buf[..n]);
        }
    });
    (addr, handle)
}

#[test]
fn system_policy_loads_anchors_through_platform_and_verifies_with_them() {
    // A CA this test invented, which no real machine trusts.
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "rusty_tls system-trust test CA");
    params.distinguished_name = dn;
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = params.self_signed(&ca_key).unwrap();
    let ca_der = ca_cert.der().clone();

    let leaf_params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
    let leaf_der = leaf_cert.der().clone();
    let leaf_key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

    // Stage it as *the* OS trust store for this process.
    let bundle =
        std::env::temp_dir().join(format!("rusty-tls-system-trust-{}.pem", std::process::id()));
    std::fs::write(&bundle, pem(&ca_der)).unwrap();
    std::env::set_var("SSL_CERT_FILE", &bundle);

    // 1. The happy path: `System` must find the staged CA and verify a
    //    leaf issued by it. Before #24 this asserted nothing about
    //    `platform`; now a break anywhere in env probe → PEM decode →
    //    DER → RootCertStore fails here.
    let (addr, server) = spawn_echo_server(leaf_der, leaf_key_der);
    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &TrustPolicy::System)
        .expect("System must verify against the anchors SSL_CERT_FILE names");
    tls.write_all(b"hello, system trust").unwrap();
    let mut buf = [0u8; "hello, system trust".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello, system trust");
    server.join().unwrap();

    // 2. The rejection path, which matters more: the staged store must be
    //    the *only* anchor set in play. A leaf from a different CA has to
    //    fail — otherwise `System` would be falling back to the machine's
    //    real store, and a passing test 1 would have proven nothing.
    let rogue = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let rogue_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(rogue.key_pair.serialize_der()));
    let (addr2, server2) = spawn_echo_server(rogue.cert.der().clone(), rogue_key);
    let tcp2 = TcpStream::connect(addr2).unwrap();
    let result = TlsStream::new(tcp2, "localhost", &TrustPolicy::System)
        .and_then(|mut s| s.write_all(b"x").map_err(Into::into));
    assert!(
        result.is_err(),
        "a certificate outside the staged anchor set must be rejected"
    );
    let _ = server2.join();

    // 3. An `SSL_CERT_FILE` naming no usable certificate must fail closed
    //    — never silently fall back to the machine's real trust store,
    //    which is the failure mode that would turn an operator's explicit
    //    override into a no-op.
    let empty = std::env::temp_dir().join(format!(
        "rusty-tls-system-trust-empty-{}.pem",
        std::process::id()
    ));
    std::fs::write(&empty, b"not a certificate\n").unwrap();
    std::env::set_var("SSL_CERT_FILE", &empty);
    let (addr3, server3) = spawn_echo_server(
        ca_cert.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ca_key.serialize_der())),
    );
    let tcp3 = TcpStream::connect(addr3).unwrap();
    assert!(
        TlsStream::new(tcp3, "localhost", &TrustPolicy::System).is_err(),
        "an SSL_CERT_FILE with no usable anchors must fail, not fall back"
    );
    let _ = server3.join();

    std::env::remove_var("SSL_CERT_FILE");
    let _ = std::fs::remove_file(&bundle);
    let _ = std::fs::remove_file(&empty);
}
