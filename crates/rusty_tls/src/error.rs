use std::fmt;
use std::io;

/// Everything that can go wrong building a TLS configuration or connection.
///
/// Deliberately flat rather than nested per-cause enums (e.g. no separate
/// `TrustError`) — callers of this crate handle "TLS didn't work," not a
/// taxonomy of why.
#[derive(Debug)]
pub enum Error {
    /// The underlying stream returned an I/O error.
    Io(io::Error),
    /// rustls rejected the handshake, a certificate, or a config value.
    Tls(rustls::Error),
    /// The hostname passed to [`crate::TlsStream::new`] is not a valid DNS
    /// name or IP address.
    InvalidServerName(String),
    /// [`crate::TrustPolicy::System`] or [`crate::TrustPolicy::PinnedAnchors`]
    /// ended up with zero usable trust anchors — fail closed rather than
    /// silently running with a `RootCertStore` that trusts nothing (which
    /// would make every handshake fail anyway, just later and less clearly).
    NoTrustAnchors,
    /// A private key handed to [`crate::TlsAcceptor::new`] or a
    /// client-identity constructor isn't valid DER in any recognized format
    /// (PKCS#8, PKCS#1, or SEC1).
    InvalidPrivateKey(String),
    /// The client-CA root certificates handed to
    /// [`crate::TlsAcceptor::new_with_client_auth`] couldn't be turned into
    /// a client-certificate verifier — most commonly because none were
    /// supplied, or none were valid DER.
    InvalidClientCaRoots(String),
    /// The roots or CRLs handed to
    /// [`crate::TrustPolicy::PinnedAnchorsWithRevocation`] couldn't be
    /// turned into a certificate verifier — most commonly an unparseable
    /// CRL.
    InvalidRevocationConfig(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Tls(e) => write!(f, "TLS error: {e}"),
            Error::InvalidServerName(name) => write!(f, "invalid server name: {name}"),
            Error::NoTrustAnchors => {
                write!(f, "no trust anchors could be loaded; refusing to connect")
            }
            Error::InvalidPrivateKey(reason) => write!(f, "invalid private key: {reason}"),
            Error::InvalidClientCaRoots(reason) => {
                write!(f, "invalid client CA roots: {reason}")
            }
            Error::InvalidRevocationConfig(reason) => {
                write!(f, "invalid revocation config: {reason}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Tls(e) => Some(e),
            Error::InvalidServerName(_)
            | Error::NoTrustAnchors
            | Error::InvalidPrivateKey(_)
            | Error::InvalidClientCaRoots(_)
            | Error::InvalidRevocationConfig(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<rustls::Error> for Error {
    fn from(e: rustls::Error) -> Self {
        Error::Tls(e)
    }
}

/// Turn any [`Error`] into an [`io::Error`], for callers threading this
/// through a `Read`/`Write` implementation that must return `io::Result`.
impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => e,
            other => io::Error::other(other),
        }
    }
}
