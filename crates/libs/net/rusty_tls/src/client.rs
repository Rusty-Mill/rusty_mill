use std::io::{self, Read, Write};
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, HandshakeKind};

use crate::error::Error;
use crate::trust::{
    build_client_config, build_client_config_with_alpn, build_client_config_with_identity,
    TrustPolicy,
};

/// A TLS client connection layered over any `Read + Write` stream.
///
/// Mirrors the shape of `rustls::StreamOwned<ClientConnection, S>` — the
/// pattern rusty_rdp's `tls.rs` already uses in production — but keeps
/// rustls types out of the public API. A caller of this crate only ever
/// names [`TlsStream`], [`TrustPolicy`](crate::TrustPolicy), and
/// [`Error`](crate::Error).
///
/// `S` must already be connected; this type never dials, and that's a hard
/// requirement rather than a convenience choice. Some protocols run a
/// plaintext exchange on the same socket *before* upgrading to TLS (e.g.
/// RDP's X.224 negotiation) — the stream handed to [`TlsStream::new`] may
/// already have been read from and written to, and this type must accept it
/// as-is.
pub struct TlsStream<S> {
    conn: ClientConnection,
    sock: S,
}

impl<S: Read + Write> TlsStream<S> {
    /// Wrap `sock` in a TLS client connection to `server_name`, trusted
    /// according to `policy`.
    ///
    /// Performs no I/O itself: the handshake runs lazily, driven by the
    /// first [`Read`]/[`Write`] call, exactly like [`rustls::StreamOwned`]
    /// (which this wraps internally).
    pub fn new(sock: S, server_name: &str, policy: &TrustPolicy) -> Result<Self, Error> {
        let config = build_client_config(policy)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, sock })
    }

    /// Wrap `sock` in a TLS client connection using an already-built
    /// `config` — the constructor [`TlsConnector`](crate::TlsConnector)
    /// uses so repeated connections share the same session-resumption
    /// cache, rather than each getting a fresh, empty one the way [`TlsStream::new`]'s
    /// per-call [`build_client_config`] would.
    pub(crate) fn from_config(
        config: Arc<ClientConfig>,
        sock: S,
        server_name: &str,
    ) -> Result<Self, Error> {
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, sock })
    }

    /// Like [`TlsStream::new`], but presents `client_cert_chain_der` (leaf
    /// first, DER-encoded) and `client_key_der` (the leaf's private key —
    /// PKCS#8, PKCS#1, or SEC1, auto-detected) to the server as a client
    /// certificate — for a server built with a `TlsAcceptor` that requests
    /// and verifies one (mTLS). Fails the same way [`TlsAcceptor::new`]
    /// does if the key isn't valid DER in a recognized format, or doesn't
    /// match the certificate.
    ///
    /// [`TlsAcceptor::new`]: crate::TlsAcceptor::new
    pub fn new_with_client_identity(
        sock: S,
        server_name: &str,
        policy: &TrustPolicy,
        client_cert_chain_der: Vec<Vec<u8>>,
        client_key_der: Vec<u8>,
    ) -> Result<Self, Error> {
        let cert_chain: Vec<CertificateDer<'static>> = client_cert_chain_der
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        let key = PrivateKeyDer::try_from(client_key_der)
            .map_err(|reason| Error::InvalidPrivateKey(reason.to_string()))?;
        let config = build_client_config_with_identity(policy, cert_chain, key)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, sock })
    }

    /// Like [`TlsStream::new`], but offers `alpn_protocols` (each entry a
    /// wire-format protocol ID, e.g. `b"h2"`) during the handshake for
    /// ALPN negotiation. See [`TlsStream::negotiated_alpn_protocol`] to
    /// read back what the server actually picked.
    pub fn new_with_alpn(
        sock: S,
        server_name: &str,
        policy: &TrustPolicy,
        alpn_protocols: Vec<Vec<u8>>,
    ) -> Result<Self, Error> {
        let config = build_client_config_with_alpn(policy, alpn_protocols)?;
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::InvalidServerName(server_name.to_string()))?;
        let conn = ClientConnection::new(config, name)?;
        Ok(Self { conn, sock })
    }

    /// The protocol negotiated via ALPN, if any — `None` until the
    /// handshake completes (see [`TlsStream::complete_handshake`]), and
    /// `None` after it if either side offered no protocols or the server
    /// accepted none of the ones offered.
    pub fn negotiated_alpn_protocol(&self) -> Option<&[u8]> {
        self.conn.alpn_protocol()
    }

    /// Whether this connection resumed a previous TLS session rather than
    /// performing a full handshake — `false` until the handshake completes
    /// (see [`TlsStream::complete_handshake`]). Only meaningful when
    /// connecting via a shared [`TlsConnector`](crate::TlsConnector):
    /// [`TlsStream::new`] builds a fresh, empty session cache on every
    /// call, so a connection made through it never has a previous session
    /// to resume.
    pub fn resumed_session(&self) -> bool {
        matches!(self.conn.handshake_kind(), Some(HandshakeKind::Resumed))
    }

    /// Whether the TLS handshake has not yet completed.
    pub fn is_handshaking(&self) -> bool {
        self.conn.is_handshaking()
    }

    /// Borrow the underlying stream. Does not touch TLS state — mainly
    /// useful for inspecting the transport (e.g. peer address).
    pub fn get_ref(&self) -> &S {
        &self.sock
    }

    /// Mutably borrow the underlying stream.
    ///
    /// Reading from or writing to it directly bypasses TLS entirely and
    /// will corrupt the session — this exists for socket-option calls
    /// (e.g. `set_nodelay`), not I/O.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.sock
    }

    /// Consume `self`, returning the underlying stream. The TLS session is
    /// discarded; the stream is left wherever the last `Read`/`Write` call
    /// left it (mid-record if one was in-flight).
    pub fn into_inner(self) -> S {
        self.sock
    }

    /// Blocks until the TLS handshake completes (or fails).
    ///
    /// Normally the handshake just runs lazily, driven by the first
    /// `Read`/`Write` call — this exists for a caller that needs
    /// handshake-derived state (e.g. [`TlsStream::peer_certificate_der`])
    /// before its own protocol logic starts reading or writing
    /// application data. RDP's CredSSP exchange is exactly this shape: it
    /// needs the server's certificate for channel binding before the
    /// CredSSP bytes themselves go over the wire.
    pub fn complete_handshake(&mut self) -> Result<(), Error> {
        if self.conn.is_handshaking() {
            self.conn.complete_io(&mut self.sock)?;
        }
        Ok(())
    }

    /// The DER-encoded end-entity certificate the peer presented during
    /// the handshake, if it has completed (see
    /// [`TlsStream::complete_handshake`]) and the peer sent one — every
    /// [`TrustPolicy`](crate::TrustPolicy) other than
    /// [`TrustPolicy::DangerNoVerification`](crate::TrustPolicy::DangerNoVerification)
    /// requires one, so `None` past the handshake only happens with that
    /// policy and a peer that chose not to present a certificate.
    ///
    /// Raw bytes rather than a parsed certificate: this crate's seam
    /// stops at "here is what the peer presented," matching the
    /// byte-oriented convention the wider ecosystem uses at boundaries
    /// like this one — parsing (e.g. extracting the `SubjectPublicKeyInfo`
    /// for a channel-binding check) is the caller's responsibility.
    pub fn peer_certificate_der(&self) -> Option<&[u8]> {
        self.conn
            .peer_certificates()
            .and_then(|certs| certs.first())
            .map(|cert| cert.as_ref())
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).read(buf)
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).flush()
    }
}
