//! Hermetic tests for session resumption (`TlsConnector`, plus the
//! server-side ticketer fix in `TlsAcceptor`). The whole point of this gap
//! is that resumption only actually triggers when the *same* config (and
//! its cache) is reused across connections — so these tests specifically
//! make two sequential connections and check the second one resumed,
//! rather than just checking a single connection succeeds.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rusty_tls::{TlsAcceptor, TlsConnector, TlsServerStream, TrustPolicy};

fn self_signed_leaf(hostname: &str) -> (Vec<u8>, Vec<u8>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![hostname.to_string()]).unwrap();
    (cert.der().to_vec(), key_pair.serialize_der())
}

/// One connection through `connector`/`acceptor` — a full round trip, then
/// report whether the *client* considers the session resumed. `listener`
/// is shared (and bound once, by the caller) so successive calls are
/// genuinely separate, sequential TCP connections to the same server.
fn connect_once(
    listener: &Arc<TcpListener>,
    addr: SocketAddr,
    acceptor: &TlsAcceptor,
    connector: &TlsConnector,
) -> bool {
    let listener = listener.clone();
    let acceptor = acceptor.clone();
    let server = thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tls: TlsServerStream<TcpStream> = acceptor.accept(tcp).unwrap();
        let mut buf = [0u8; "ping".len()];
        tls.read_exact(&mut buf).unwrap();
        tls.write_all(b"pong").unwrap();
    });

    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = connector.connect(tcp, "localhost").unwrap();
    tls.write_all(b"ping").unwrap();
    let mut buf = [0u8; "pong".len()];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"pong");
    let resumed = tls.resumed_session();

    server.join().unwrap();
    resumed
}

#[test]
fn second_connection_through_a_shared_connector_resumes() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();
    let connector = TlsConnector::new(&TrustPolicy::DangerNoVerification).unwrap();

    let listener = Arc::new(TcpListener::bind("127.0.0.1:0").unwrap());
    let addr = listener.local_addr().unwrap();

    let first_resumed = connect_once(&listener, addr, &acceptor, &connector);
    assert!(
        !first_resumed,
        "the first connection to a server has no prior session to resume"
    );

    let second_resumed = connect_once(&listener, addr, &acceptor, &connector);
    assert!(
        second_resumed,
        "the second connection through the same TlsConnector should resume the first's session"
    );
}

#[test]
fn separate_connectors_never_resume() {
    let (cert_der, key_der) = self_signed_leaf("localhost");
    let acceptor = TlsAcceptor::new(vec![cert_der], key_der).unwrap();

    let listener = Arc::new(TcpListener::bind("127.0.0.1:0").unwrap());
    let addr = listener.local_addr().unwrap();

    // Two independent `TlsConnector`s (mirroring what two independent
    // `TlsStream::new` calls would do — each builds its own fresh config)
    // never share a resumption cache, so neither connection resumes.
    let connector_a = TlsConnector::new(&TrustPolicy::DangerNoVerification).unwrap();
    connect_once(&listener, addr, &acceptor, &connector_a);

    let connector_b = TlsConnector::new(&TrustPolicy::DangerNoVerification).unwrap();
    let resumed = connect_once(&listener, addr, &acceptor, &connector_b);
    assert!(
        !resumed,
        "a second connection through a different (fresh) config should not resume"
    );
}
