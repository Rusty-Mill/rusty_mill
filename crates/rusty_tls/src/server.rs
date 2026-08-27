//! Server-side TLS: accept connections presenting a certificate and
//! private key, optionally requiring and verifying a client certificate
//! (mTLS), or offering ALPN protocols, in turn.

use std::io::{self, Read, Write};
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection};

use crate::error::Error;

/// Finishes a `ServerConfig` and wraps it for reuse across every accepted
/// connection. Sets a real session-ticket producer — `rustls::ServerConfig`
/// otherwise defaults to `NeverProducesTickets`, which silently disables
/// TLS 1.3 session resumption even though `TlsAcceptor` already builds its
/// config once and reuses it (`Arc`-backed) across connections, exactly the
/// pattern resumption needs. Stateful (session-ID/ticket-lookup) resumption
/// already works via `ServerConfig`'s own default `session_storage`; this
/// is the other half, for TLS 1.3's ticket-based resumption specifically.
fn finish_config(mut config: ServerConfig) -> Result<Arc<ServerConfig>, Error> {
    config.ticketer = rustls::crypto::ring::Ticketer::new()?;
    Ok(Arc::new(config))
}

/// The TLS configuration a server accepts connections with: a certificate
/// chain and its private key. Build once and reuse — accepting a
/// connection only clones an `Arc`, not the underlying config.
#[derive(Clone)]
pub struct TlsAcceptor {
    config: Arc<ServerConfig>,
}

impl TlsAcceptor {
    /// `cert_chain_der` is the leaf certificate followed by any
    /// intermediates, each DER-encoded, leaf first. `private_key_der` is
    /// the leaf's private key, DER-encoded — PKCS#8, PKCS#1, or SEC1,
    /// auto-detected from the DER structure.
    pub fn new(cert_chain_der: Vec<Vec<u8>>, private_key_der: Vec<u8>) -> Result<Self, Error> {
        let cert_chain: Vec<CertificateDer<'static>> = cert_chain_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let key = PrivateKeyDer::try_from(private_key_der)
            .map_err(|reason| Error::InvalidPrivateKey(reason.to_string()))?;
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?;
        Ok(Self {
            config: finish_config(config)?,
        })
    }

    /// Like [`TlsAcceptor::new`], but also requires and verifies a client
    /// certificate (mTLS): `client_ca_roots_der` are the DER-encoded CA
    /// certificates a presented client certificate must chain to. A
    /// connection from a client that doesn't present a certificate, or
    /// presents one that doesn't chain to any of these roots, fails the
    /// handshake. Pairs with a client built via
    /// [`TlsStream::new_with_client_identity`](crate::TlsStream::new_with_client_identity)
    /// or its async counterpart.
    pub fn new_with_client_auth(
        cert_chain_der: Vec<Vec<u8>>,
        private_key_der: Vec<u8>,
        client_ca_roots_der: Vec<Vec<u8>>,
    ) -> Result<Self, Error> {
        let cert_chain: Vec<CertificateDer<'static>> = cert_chain_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let key = PrivateKeyDer::try_from(private_key_der)
            .map_err(|reason| Error::InvalidPrivateKey(reason.to_string()))?;

        let mut client_ca_roots = RootCertStore::empty();
        for der in client_ca_roots_der {
            client_ca_roots.add(CertificateDer::from(der))?;
        }
        let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_ca_roots))
            .build()
            .map_err(|e| Error::InvalidClientCaRoots(e.to_string()))?;

        let config = ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(cert_chain, key)?;
        Ok(Self {
            config: finish_config(config)?,
        })
    }

    /// Like [`TlsAcceptor::new`], but offers `alpn_protocols` (each entry a
    /// wire-format protocol ID, e.g. `b"h2"`) during the handshake. See
    /// [`TlsServerStream::negotiated_alpn_protocol`] to read back what was
    /// actually negotiated with a given connection.
    pub fn new_with_alpn(
        cert_chain_der: Vec<Vec<u8>>,
        private_key_der: Vec<u8>,
        alpn_protocols: Vec<Vec<u8>>,
    ) -> Result<Self, Error> {
        let cert_chain: Vec<CertificateDer<'static>> = cert_chain_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let key = PrivateKeyDer::try_from(private_key_der)
            .map_err(|reason| Error::InvalidPrivateKey(reason.to_string()))?;
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?;
        config.alpn_protocols = alpn_protocols;
        Ok(Self {
            config: finish_config(config)?,
        })
    }

    /// Wrap `sock` (already accepted) in a TLS server connection.
    ///
    /// Performs no I/O itself — the handshake runs lazily, driven by the
    /// first `Read`/`Write` call, exactly like
    /// [`TlsStream::new`](crate::TlsStream::new).
    pub fn accept<S: Read + Write>(&self, sock: S) -> Result<TlsServerStream<S>, Error> {
        let conn = ServerConnection::new(self.config.clone())?;
        Ok(TlsServerStream { conn, sock })
    }

    /// Wrap `io` (already accepted) in an async TLS server connection — the
    /// async counterpart to [`TlsAcceptor::accept`]. Behind the
    /// `rusty-tokio` feature, driving the same sans-IO `ServerConnection`
    /// over `rusty_tokio`'s `AsyncRead`/`AsyncWrite` instead of blocking I/O,
    /// the way [`AsyncTlsStream`](crate::AsyncTlsStream) does on the client
    /// side.
    #[cfg(feature = "rusty-tokio")]
    pub fn accept_async<S: rusty_tokio::io::AsyncRead + rusty_tokio::io::AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<crate::async_server::AsyncTlsServerStream<S>, Error> {
        crate::async_server::AsyncTlsServerStream::new(self.config.clone(), io)
    }
}

/// A TLS server connection layered over any `Read + Write` stream — the
/// server counterpart to [`crate::TlsStream`]. Built via
/// [`TlsAcceptor::accept`]; keeps rustls types out of the public API the
/// same way the client adapter does.
pub struct TlsServerStream<S> {
    conn: ServerConnection,
    sock: S,
}

impl<S: Read + Write> TlsServerStream<S> {
    /// Whether the TLS handshake has not yet completed.
    pub fn is_handshaking(&self) -> bool {
        self.conn.is_handshaking()
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.sock
    }

    /// Mutably borrow the underlying stream. See
    /// [`TlsStream::get_mut`](crate::TlsStream::get_mut)'s docs — the same
    /// caution applies.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.sock
    }

    /// Consume `self`, returning the underlying stream. See
    /// [`TlsStream::into_inner`](crate::TlsStream::into_inner)'s docs.
    pub fn into_inner(self) -> S {
        self.sock
    }

    /// Blocks until the TLS handshake completes (or fails), without
    /// requiring the caller to send or expect application data first. See
    /// [`TlsStream::complete_handshake`](crate::TlsStream::complete_handshake)'s
    /// docs for why this exists.
    pub fn complete_handshake(&mut self) -> Result<(), Error> {
        if self.conn.is_handshaking() {
            self.conn.complete_io(&mut self.sock)?;
        }
        Ok(())
    }

    /// The protocol negotiated via ALPN, if any. See
    /// [`TlsStream::negotiated_alpn_protocol`](crate::TlsStream::negotiated_alpn_protocol)'s
    /// docs for when this is (and isn't) populated.
    pub fn negotiated_alpn_protocol(&self) -> Option<&[u8]> {
        self.conn.alpn_protocol()
    }
}

impl<S: Read + Write> Read for TlsServerStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).read(buf)
    }
}

impl<S: Read + Write> Write for TlsServerStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).flush()
    }
}
