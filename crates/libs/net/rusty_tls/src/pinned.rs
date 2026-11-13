//! The verifier behind
//! [`TrustPolicy::PinnedCertSha256`](crate::TrustPolicy::PinnedCertSha256):
//! trust a server because its certificate is one the caller pinned, not
//! because a CA vouches for it.
//!
//! For a server whose certificate the user supplied and the client pinned on
//! first use (rusty_mux's TLS listener, D236), where no CA is involved.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};

/// The SHA-256 fingerprint of a DER-encoded certificate: the value
/// [`TrustPolicy::PinnedCertSha256`](crate::TrustPolicy::PinnedCertSha256)
/// pins.
///
/// To pin on first use, connect once, take
/// [`TlsStream::peer_certificate_der`](crate::TlsStream::peer_certificate_der),
/// show this fingerprint to the user, and store it.
pub fn certificate_sha256(der: &[u8]) -> [u8; 32] {
    // SHA-256 from the ring provider rustls already runs on, so this adds no
    // dependency and keeps the crate's rust-version. The suite is fixed at
    // compile time and is a TLS 1.3 suite whose hash is SHA-256.
    let suite = rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256
        .tls13()
        .expect("TLS13_AES_128_GCM_SHA256 is a TLS 1.3 suite");
    let digest = suite.common.hash_provider.hash(der);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// Accepts a server whose end-entity certificate's SHA-256 fingerprint is
/// one of `pins`, and still verifies that the server holds that
/// certificate's private key: the handshake signature is checked against
/// the certificate's public key, exactly as for a CA-issued certificate.
/// Without that check, anyone who had seen the (public) certificate could
/// present it.
///
/// What it does not check, on purpose: the chain (a pinned certificate is
/// its own anchor), the validity period (a user-made certificate's dates
/// mean nothing the pin doesn't already say), and the server name (the pin
/// is per host already, stored by the caller under that host).
#[derive(Debug)]
pub(crate) struct PinnedCertVerifier {
    pins: Vec<[u8; 32]>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedCertVerifier {
    pub(crate) fn new(pins: Vec<[u8; 32]>, algorithms: WebPkiSupportedAlgorithms) -> Self {
        Self { pins, algorithms }
    }
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint = certificate_sha256(end_entity.as_ref());
        if self.pins.contains(&fingerprint) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
