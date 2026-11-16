//! The operator's certificate, from PEM to a [`TlsAcceptor`]. The PEM reader
//! is `ts_dial::pem`, shared with the clients that read a CA file. The mux
//! server has its own copy (`rusty_mux_server::pem`); a library cannot depend
//! on that app, and a shared reader in `rusty_tls` is a public-API change for
//! the owner to decide.

use rusty_tls::TlsAcceptor;

/// Why a certificate or key could not become an acceptor. No variant holds
/// file content, so an error can be logged or shown without leaking a key:
/// the one byte `rusty_base64` reports for a bad character is dropped.
#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("the certificate file has no CERTIFICATE block")]
    NoCertificate,
    #[error(
        "the key file has no PKCS#8 `PRIVATE KEY` block (convert with `openssl pkcs8 -topk8 -nocrypt`)"
    )]
    NoPkcs8Key,
    /// A block that is unbalanced, nested or not base64.
    #[error("{file} file: {reason}")]
    Pem {
        file: &'static str,
        reason: &'static str,
    },
    #[error("certificate: {0}")]
    Tls(#[from] rusty_tls::Error),
}

/// The DER of every PEM block in `text`, as `(label, der)`.
fn blocks(text: &str, file: &'static str) -> Result<Vec<(String, Vec<u8>)>, TlsConfigError> {
    ts_dial::pem::blocks(text, file).map_err(|e| TlsConfigError::Pem {
        file: e.file,
        reason: e.reason,
    })
}

/// A TLS acceptor serving the certificate chain in `cert_pem` with the
/// PKCS#8 key in `key_pem`.
pub fn tls_acceptor(cert_pem: &str, key_pem: &str) -> Result<TlsAcceptor, TlsConfigError> {
    let chain: Vec<_> = blocks(cert_pem, "certificate")?
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect();
    if chain.is_empty() {
        return Err(TlsConfigError::NoCertificate);
    }
    let key = blocks(key_pem, "key")?
        .into_iter()
        .find_map(|(label, der)| (label == "PRIVATE KEY").then_some(der))
        .ok_or(TlsConfigError::NoPkcs8Key)?;
    Ok(TlsAcceptor::new(chain, key)?)
}
