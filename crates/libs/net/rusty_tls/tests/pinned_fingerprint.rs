//! `TrustPolicy::PinnedFingerprints`: the server is trusted by its own
//! certificate's SHA-256, whatever the chain, clock or hostname say, and the
//! handshake signature is still checked.
#![cfg(feature = "rusty-tokio")]

use rusty_tls::{fingerprint_sha256, AsyncTlsStream, Error, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncReadExt, AsyncWriteExt, TcpListener, TcpStream};

fn leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

/// Serves one connection with `acceptor`, answering "ok" once it reads "hi".
async fn serve(acceptor: TlsAcceptor) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    rusty_tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let Ok(mut tls) = acceptor.accept_async(tcp) else {
            return;
        };
        let mut buf = [0u8; 2];
        if tls.read_exact(&mut buf).await.is_ok() {
            let _ = tls.write_all(b"ok").await;
        }
    });
    addr
}

/// Connects to `addr` pinning `pins`, under whatever name; whether "hi" is answered.
async fn answered(addr: std::net::SocketAddr, name: &str, pins: Vec<[u8; 32]>) -> bool {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let Ok(mut tls) = AsyncTlsStream::new(tcp, name, &TrustPolicy::PinnedFingerprints(pins)) else {
        return false;
    };
    if tls.write_all(b"hi").await.is_err() {
        return false;
    }
    let mut buf = [0u8; 2];
    tls.read_exact(&mut buf).await.is_ok() && &buf == b"ok"
}

#[rusty_tokio::test]
async fn the_pinned_certificate_is_trusted_whatever_name_the_client_uses() {
    let (cert, key) = leaf("server.example");
    let pin = fingerprint_sha256(&cert).unwrap();
    let addr = serve(TlsAcceptor::new(vec![cert], key).unwrap()).await;
    // Not the certificate's name, and self-signed: the pin is the identity.
    assert!(answered(addr, "localhost", vec![[0; 32], pin]).await);
}

#[rusty_tokio::test]
async fn another_certificate_is_refused() {
    let (cert, key) = leaf("localhost");
    let (other, _) = leaf("localhost");
    let addr = serve(TlsAcceptor::new(vec![cert], key).unwrap()).await;
    assert!(!answered(addr, "localhost", vec![fingerprint_sha256(&other).unwrap()]).await);
}

/// A server that presents `cert` but signs with `key`, a key that is not the
/// certificate's own (rustls refuses to build that pair through `TlsAcceptor`,
/// so it is made here with rustls's own resolver).
fn impostor(cert: Vec<u8>, key: Vec<u8>) -> std::net::SocketAddr {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::sign::CertifiedKey;

    #[derive(Debug)]
    struct Fixed(std::sync::Arc<CertifiedKey>);
    impl rustls::server::ResolvesServerCert for Fixed {
        fn resolve(
            &self,
            _: rustls::server::ClientHello<'_>,
        ) -> Option<std::sync::Arc<CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
    let signer = provider.key_provider.load_private_key(key).unwrap();
    let certified = CertifiedKey::new(vec![CertificateDer::from(cert)], signer);
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(std::sync::Arc::new(Fixed(std::sync::Arc::new(certified))));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        let mut conn = rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap();
        // The client aborts the handshake; whatever this returns is fine.
        let _ = conn.complete_io(&mut tcp);
    });
    addr
}

#[test]
fn the_pinned_certificate_without_its_key_is_refused() {
    // The certificate is public: an impostor that presents it but signs with
    // another key must fail the handshake signature, not just the pin.
    let (cert, _) = leaf("localhost");
    let (_, other_key) = leaf("localhost");
    let pin = fingerprint_sha256(&cert).unwrap();
    let addr = impostor(cert, other_key);
    let tcp = std::net::TcpStream::connect(addr).unwrap();
    let mut tls = rusty_tls::TlsStream::new(
        tcp,
        "localhost",
        &TrustPolicy::PinnedFingerprints(vec![pin]),
    )
    .unwrap();
    let err = std::io::Write::write_all(&mut tls, b"hi").expect_err("the impostor was trusted");
    assert!(!err.to_string().is_empty());
}

#[test]
fn no_pins_is_an_error_not_a_policy_that_trusts_nothing_silently() {
    let runtime = rusty_tokio::Builder::new_current_thread().build().unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let err = AsyncTlsStream::new(tcp, "localhost", &TrustPolicy::PinnedFingerprints(vec![]))
            .err()
            .expect("an empty pin list is refused");
        assert!(matches!(err, Error::NoTrustAnchors), "{err:?}");
    });
}

#[test]
fn the_fingerprint_is_the_sha256_of_the_der() {
    // SHA-256("abc"), FIPS 180-4's own example.
    let expected = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    assert_eq!(fingerprint_sha256(b"abc").unwrap(), expected);
}
