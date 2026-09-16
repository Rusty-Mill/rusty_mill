//! The [`TrustPolicy::DangerNoVerification`](crate::TrustPolicy::DangerNoVerification)
//! escape hatch's verifier.
//!
//! Kept in its own module, named for what it does rather than for
//! convenience, per the mission's rule that this policy must "read as
//! dangerous at every call site."

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::DigitallySignedStruct;

/// Accepts every server certificate and every handshake signature, without
/// checking anything: no chain-building, no expiry check, no hostname match.
///
/// This is the verifier behind
/// [`TrustPolicy::DangerNoVerification`](crate::TrustPolicy::DangerNoVerification)
/// — see that variant's documentation for when this is (and is not)
/// appropriate. A connection using it has **no protection against an active
/// man-in-the-middle**.
#[derive(Debug, Default)]
pub(crate) struct NoServerCertVerification;

impl NoServerCertVerification {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ServerCertVerifier for NoServerCertVerification {
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

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            RSA_PKCS1_SHA1,
            ECDSA_SHA1_Legacy,
            RSA_PKCS1_SHA256,
            ECDSA_NISTP256_SHA256,
            RSA_PKCS1_SHA384,
            ECDSA_NISTP384_SHA384,
            RSA_PKCS1_SHA512,
            ECDSA_NISTP521_SHA512,
            RSA_PSS_SHA256,
            RSA_PSS_SHA384,
            RSA_PSS_SHA512,
            ED25519,
            ED448,
        ]
    }
}
