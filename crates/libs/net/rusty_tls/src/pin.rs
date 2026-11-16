//! The [`TrustPolicy::PinnedFingerprints`](crate::TrustPolicy::PinnedFingerprints)
//! verifier: trust the server's own certificate by its SHA-256 fingerprint,
//! with no chain, no clock and no hostname.
//!
//! This is the trust model of SSH's `known_hosts` and of a self-signed
//! certificate the user supplied: the pin *is* the identity. What it does not
//! skip is the handshake signature: a peer that holds only the public
//! certificate (which is not secret) cannot prove it owns the key, so the
//! `CertificateVerify` signature is still checked against that certificate.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::hash::HashAlgorithm;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, SignatureScheme};

/// A SHA-256 certificate fingerprint.
pub(crate) type Fingerprint = [u8; 32];

/// The SHA-256 of `der`, by the provider's own hash (found through its TLS 1.3
/// suites, so no hash crate is added).
pub(crate) fn fingerprint(provider: &CryptoProvider, der: &[u8]) -> Option<Fingerprint> {
    let hash = provider
        .cipher_suites
        .iter()
        .filter_map(|suite| suite.tls13())
        .map(|suite| suite.common.hash_provider)
        .find(|hash| hash.algorithm() == HashAlgorithm::SHA256)?;
    hash.hash(der).as_ref().try_into().ok()
}

/// The SHA-256 fingerprint of a DER certificate, as
/// [`TrustPolicy::PinnedFingerprints`](crate::TrustPolicy::PinnedFingerprints)
/// compares it: what a client stores to pin a server, and what
/// [`TlsStream::peer_certificate_der`](crate::TlsStream::peer_certificate_der)
/// lets it show the user on first use.
pub fn fingerprint_sha256(der: &[u8]) -> Result<[u8; 32], crate::Error> {
    fingerprint(&crate::provider::ring_provider(), der).ok_or_else(|| {
        crate::Error::Tls(rustls::Error::General(
            "no SHA-256 in the crypto provider".into(),
        ))
    })
}

/// Accepts a server certificate only if its SHA-256 is one of `pins`.
#[derive(Debug)]
pub(crate) struct PinnedFingerprintVerifier {
    pins: Vec<Fingerprint>,
    provider: Arc<CryptoProvider>,
}

impl PinnedFingerprintVerifier {
    pub(crate) fn new(pins: Vec<Fingerprint>, provider: Arc<CryptoProvider>) -> Self {
        Self { pins, provider }
    }
}

impl ServerCertVerifier for PinnedFingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let seen = fingerprint(&self.provider, end_entity.as_ref())
            .ok_or_else(|| Error::General("no SHA-256 in the crypto provider".into()))?;
        match self.pins.contains(&seen) {
            true => Ok(ServerCertVerified::assertion()),
            // The fingerprint is public, so comparing it needs no constant time.
            false => Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
