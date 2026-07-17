//! Optional TLS connector for the enhanced-security path (feature `tls`).
//!
//! This is the crate's one concession to a third-party dependency, and it is
//! entirely opt-in: nothing here is compiled unless the `tls` feature is
//! enabled, so the default build stays dependency-free. A TLS stack is the one
//! piece that cannot be hand-rolled responsibly, so [`connect_tls`] wraps
//! [`rustls`] to upgrade the TCP stream before handing off to
//! [`RdpTransport::establish_enhanced`].
//!
//! The RDP-over-TLS *protocol* logic still lives in the dependency-free core
//! ([`crate::net`]) — this module only supplies the bytes-on-wire TLS. If you
//! would rather not take the rustls dependency, wrap your `TcpStream` in any
//! TLS implementation yourself and call [`RdpTransport::new_enhanced`] +
//! [`RdpTransport::establish_enhanced`] directly.
//!
//! ## Certificate verification
//!
//! RDP servers overwhelmingly present self-signed certificates and rely on
//! out-of-band trust, so [`connect_tls`] does **not** verify the server
//! certificate. That means it does not protect against an active
//! man-in-the-middle. If you need verification, build your own [`rustls`]
//! stream with a real verifier and use the [`RdpTransport::new_enhanced`] path.

use std::io;
use std::net::TcpStream;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};

use crate::nego::SecurityProtocols;
use crate::net::{EstablishConfig, RdpSession, RdpTransport};

/// The TLS-wrapped stream type an established enhanced-security session runs
/// over.
pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// Connect to an RDP server over TLS and drive the enhanced-security bring-up.
///
/// Opens a TCP connection to `addr` (a `host:port` string), performs the
/// X.224 negotiation requesting `requested`, upgrades the stream to TLS, and
/// runs [`RdpTransport::establish_enhanced`]. Returns the live transport and
/// its [`RdpSession`].
///
/// Only the plain TLS (`SSL`) selection is followed through: if the server
/// insists on CredSSP/NLA (`HYBRID`) or falls back to standard RDP security,
/// this returns an error describing which path to use instead.
///
/// The server certificate is **not** verified — see the [module
/// docs](self#certificate-verification).
pub fn connect_tls(
    addr: &str,
    config: &EstablishConfig,
    requested: SecurityProtocols,
) -> io::Result<(RdpTransport<TlsStream>, RdpSession)> {
    // Host portion of `host:port`, for the TLS server name.
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);

    // 1. X.224 negotiation happens on the raw TCP connection, before TLS.
    let tcp = TcpStream::connect(addr)?;
    let mut pre = RdpTransport::new(tcp);
    let selected = pre.negotiate(requested, Some(&config.username))?;
    match selected {
        s if s == SecurityProtocols::SSL => {}
        s if s == SecurityProtocols::HYBRID || s == SecurityProtocols::HYBRID_EX => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server requires CredSSP/NLA (HYBRID), which is not implemented",
            ));
        }
        s if s == SecurityProtocols::RDP => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server selected standard RDP security; use RdpTransport::establish instead",
            ));
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("server selected unsupported protocol {other:?}"),
            ));
        }
    }
    let tcp = pre.into_inner();

    // 2. Upgrade the same TCP connection to TLS.
    let tls_config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("bad server name: {e}"))
    })?;
    let conn = ClientConnection::new(Arc::new(tls_config), server_name)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("TLS setup failed: {e}")))?;
    let tls = StreamOwned::new(conn, tcp);

    // 3. Everything from MCS onward runs inside the TLS tunnel.
    let mut transport = RdpTransport::new_enhanced(tls);
    let session = transport.establish_enhanced(config, selected)?;
    Ok((transport, session))
}

/// A certificate verifier that accepts any server certificate.
///
/// Appropriate for RDP's typical self-signed-certificate deployments where
/// trust is established out of band; it provides no protection against an
/// active man-in-the-middle.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}
