//! Hermetic tests for CRL-based revocation checking
//! (`TrustPolicy::PinnedAnchorsWithRevocation`). A plain sync rustls server
//! plays the peer (this crate has no revocation-aware server side to
//! exercise), presenting either a revoked or a live leaf certificate
//! issued by the same CA and checked against the same CRL.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertificateRevocationListParams,
    DistinguishedName, DnType, IsCa, KeyIdMethod, KeyPair, RevokedCertParams, SerialNumber,
};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use rusty_tls::{Error, TlsStream, TrustPolicy};

struct TestCa {
    cert: Certificate,
    key_pair: KeyPair,
}

impl TestCa {
    fn generate() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "rusty_tls revocation test CA");
        params.distinguished_name = dn;
        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        TestCa { cert, key_pair }
    }

    fn root_der(&self) -> CertificateDer<'static> {
        self.cert.der().clone()
    }

    fn issue_leaf(
        &self,
        hostname: &str,
        serial: u64,
    ) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let mut params = CertificateParams::new(vec![hostname.to_string()]).unwrap();
        params.serial_number = Some(SerialNumber::from(serial));
        let leaf_key = KeyPair::generate().unwrap();
        let leaf_cert = params
            .signed_by(&leaf_key, &self.cert, &self.key_pair)
            .unwrap();
        let key_der = PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
            leaf_key.serialize_der(),
        ));
        (leaf_cert.der().clone(), key_der)
    }

    /// A CRL, signed by this CA, revoking exactly `revoked_serial`.
    fn crl_revoking(&self, revoked_serial: u64) -> CertificateRevocationListDer<'static> {
        let revoked = RevokedCertParams {
            serial_number: SerialNumber::from(revoked_serial),
            revocation_time: rcgen::date_time_ymd(2020, 1, 1),
            reason_code: None,
            invalidity_date: None,
        };
        let crl = CertificateRevocationListParams {
            this_update: rcgen::date_time_ymd(2020, 1, 1),
            next_update: rcgen::date_time_ymd(2035, 1, 1),
            crl_number: SerialNumber::from(1u64),
            issuing_distribution_point: None,
            revoked_certs: vec![revoked],
            key_identifier_method: KeyIdMethod::Sha256,
        }
        .signed_by(&self.cert, &self.key_pair)
        .unwrap();
        crl.der().clone()
    }
}

fn spawn_echo_server(
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
) -> (SocketAddr, thread::JoinHandle<()>) {
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
fn rejects_a_certificate_on_the_crl() {
    let ca = TestCa::generate();
    let (leaf_der, leaf_key) = ca.issue_leaf("localhost", 42);
    let crl = ca.crl_revoking(42);
    let (addr, _server) = spawn_echo_server(leaf_der, leaf_key);

    let policy = TrustPolicy::PinnedAnchorsWithRevocation {
        roots: vec![ca.root_der()],
        crls: vec![crl],
    };
    let tcp = std::net::TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    let result = tls.write_all(b"should not be accepted");
    assert!(
        result.is_err(),
        "handshake should fail when the presented certificate is on the CRL"
    );
}

#[test]
fn accepts_a_certificate_not_on_the_crl() {
    let ca = TestCa::generate();
    // The CRL revokes serial 42; this leaf is a different serial (43), so
    // it isn't affected by the same CRL.
    let (leaf_der, leaf_key) = ca.issue_leaf("localhost", 43);
    let crl = ca.crl_revoking(42);
    let (addr, server) = spawn_echo_server(leaf_der, leaf_key);

    let policy = TrustPolicy::PinnedAnchorsWithRevocation {
        roots: vec![ca.root_der()],
        crls: vec![crl],
    };
    let tcp = std::net::TcpStream::connect(addr).unwrap();
    let mut tls = TlsStream::new(tcp, "localhost", &policy).unwrap();

    tls.write_all(b"hello, unrevoked").unwrap();
    let mut buf = [0u8; "hello, unrevoked".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello, unrevoked");

    server.join().unwrap();
}

#[test]
fn empty_roots_is_a_hard_error() {
    // No server/socket needed — config building fails before any I/O, the
    // same way `PinnedAnchors` with zero certs does.
    let policy = TrustPolicy::PinnedAnchorsWithRevocation {
        roots: vec![],
        crls: vec![],
    };
    let result = TlsStream::new(std::io::Cursor::new(Vec::<u8>::new()), "localhost", &policy);
    assert!(matches!(result, Err(Error::NoTrustAnchors)));
}
