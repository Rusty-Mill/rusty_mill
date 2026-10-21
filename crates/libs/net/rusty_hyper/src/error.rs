//! The one error type for connecting and serving.

use std::fmt;
use std::io;

/// Why connecting or serving failed.
#[derive(Debug)]
pub enum Error {
    /// The URL could not be parsed.
    InvalidUrl(String),
    /// The scheme is not `http` or `https`.
    UnsupportedScheme(String),
    /// The transport failed (resolve, connect, read or write).
    Io(io::Error),
    /// The TLS handshake or configuration failed.
    Tls(rusty_tls::Error),
    /// The peer did not negotiate ALPN `h2`; this crate speaks nothing else.
    AlpnNotH2,
    /// A step took longer than its configured limit; names the step.
    Timeout(&'static str),
    /// The HTTP/2 connection failed.
    H2(rusty_h2::driver::Error),
    /// The connection task was cancelled or panicked.
    Aborted,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUrl(u) => write!(f, "invalid URL `{u}`"),
            Error::UnsupportedScheme(s) => write!(f, "unsupported scheme in `{s}`"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Tls(e) => write!(f, "{e}"),
            Error::AlpnNotH2 => write!(f, "the peer did not negotiate ALPN h2"),
            Error::Timeout(step) => write!(f, "timed out: {step}"),
            Error::H2(e) => write!(f, "{e}"),
            Error::Aborted => write!(f, "the connection task was cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<rusty_tls::Error> for Error {
    fn from(e: rusty_tls::Error) -> Self {
        Error::Tls(e)
    }
}

impl From<rusty_h2::driver::Error> for Error {
    fn from(e: rusty_h2::driver::Error) -> Self {
        Error::H2(e)
    }
}
