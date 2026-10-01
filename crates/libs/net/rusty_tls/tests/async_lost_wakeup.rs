//! The async adapters must never return `Pending` while they hold
//! everything needed to finish (design review Tranche 5: the Windows
//! `async_handshake` tests that hung for 600 s, then passed in 0.019 s).
//!
//! The transport here is an in-memory pipe to a real rustls peer driven in
//! the same thread: every byte the adapter writes is processed at once, so
//! the peer's reply is always already waiting. Its `poll_read` returns
//! `Pending` when empty and never wakes anyone. A `Pending` from the
//! adapter under test is therefore a hang, which a real executor would sit
//! in until the peer happened to send something else.
//!
//! What hung: a `write` before the handshake finished drove the handshake
//! with a loop that kept reading until `wants_read()` went false, which on
//! an established connection with nothing buffered it never does. After
//! the TLS 1.3 handshake the server sends session tickets; when they
//! arrived before the client's next read, the client processed them, read
//! again, found nothing, and returned `Pending` from a `write` it could
//! already complete. A server that speaks first hit the same loop.
#![cfg(feature = "rusty-tokio")]

use std::future::Future;
use std::io::{self, Read, Write};
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use rcgen::CertifiedKey;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{
    ClientConfig, ClientConnection, Connection, RootCertStore, ServerConfig, ServerConnection,
};
use rusty_tls::{AsyncTlsStream, TlsAcceptor, TrustPolicy};
use rusty_tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

/// An in-memory transport whose far end is `peer`, an echo: whatever
/// plaintext it receives, it sends back.
struct Loopback {
    peer: Connection,
    to_adapter: Vec<u8>,
}

impl Loopback {
    fn new(peer: impl Into<Connection>) -> Self {
        let mut loopback = Self {
            peer: peer.into(),
            to_adapter: Vec::new(),
        };
        loopback.pump();
        loopback
    }

    /// Echoes any plaintext the peer has, then queues its TLS output.
    fn pump(&mut self) {
        let mut plaintext = Vec::new();
        match self.peer.reader().read_to_end(&mut plaintext) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("peer read failed: {e}"),
        }
        if !plaintext.is_empty() {
            self.peer.writer().write_all(&plaintext).expect("peer echo");
        }
        while self.peer.wants_write() {
            self.peer
                .write_tls(&mut self.to_adapter)
                .expect("peer write");
        }
    }
}

impl AsyncRead for Loopback {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.to_adapter.is_empty() {
            // Nothing will ever arrive, and no one is woken.
            return Poll::Pending;
        }
        let n = buf.unfilled_mut().len().min(self.to_adapter.len());
        buf.unfilled_mut()[..n].copy_from_slice(&self.to_adapter[..n]);
        buf.advance(n);
        self.to_adapter.drain(..n);
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Loopback {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut input = buf;
        while !input.is_empty() {
            self.peer.read_tls(&mut input)?;
            self.peer
                .process_new_packets()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        }
        self.pump();
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Polls `future` once. The transport never wakes, so anything but `Ready`
/// is a hang.
fn poll_once<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("Pending with everything needed to finish: a lost wakeup"),
    }
}

fn self_signed() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
    (cert.der().clone(), key)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[test]
fn client_write_completes_when_the_session_tickets_arrive_with_the_handshake() {
    let (cert, key) = self_signed();
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .unwrap();
    let peer = ServerConnection::new(Arc::new(config)).unwrap();
    let policy = TrustPolicy::PinnedAnchors(vec![cert]);
    let mut tls = AsyncTlsStream::new(Loopback::new(peer), "localhost", &policy).unwrap();

    poll_once(tls.write_all(b"ping")).unwrap();
    let mut echoed = [0u8; 4];
    poll_once(tls.read_exact(&mut echoed)).unwrap();
    assert_eq!(&echoed, b"ping");
}

#[test]
fn server_read_returns_a_request_that_arrived_with_the_clients_finished() {
    let (mut peer, acceptor) = client_peer_and_acceptor();
    // Queued before the handshake: rustls sends it right after the
    // client's Finished, so it reaches the server in the same flight.
    peer.writer().write_all(b"ping").unwrap();
    let mut tls = acceptor.accept_async(Loopback::new(peer)).unwrap();

    let mut request = [0u8; 4];
    poll_once(tls.read_exact(&mut request)).unwrap();
    assert_eq!(&request, b"ping");
}

#[test]
fn server_that_speaks_first_completes_its_write() {
    let (peer, acceptor) = client_peer_and_acceptor();
    let mut tls = acceptor.accept_async(Loopback::new(peer)).unwrap();

    poll_once(tls.write_all(b"220 ready")).unwrap();
    let mut echoed = [0u8; 9];
    poll_once(tls.read_exact(&mut echoed)).unwrap();
    assert_eq!(&echoed, b"220 ready");
}

/// A client peer trusting a fresh self-signed certificate, and an acceptor
/// serving it.
fn client_peer_and_acceptor() -> (ClientConnection, TlsAcceptor) {
    let (cert, key) = self_signed();
    let mut roots = RootCertStore::empty();
    roots.add(cert.clone()).unwrap();
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from("localhost").unwrap();
    let peer = ClientConnection::new(Arc::new(config), name).unwrap();
    let acceptor = TlsAcceptor::new(vec![cert.to_vec()], key.secret_der().to_vec()).unwrap();
    (peer, acceptor)
}
