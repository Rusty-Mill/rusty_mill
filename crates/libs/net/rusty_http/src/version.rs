use crate::error::{Error, Result};
use std::fmt;

/// The HTTP version named on a request/status line. Only 1.0 and 1.1 --
/// this crate is an HTTP/1.1 implementation (see `ARCHITECTURE.md`'s
/// non-goals); HTTP/1.0 is recognized because a real peer still sends it
/// and its close-delimited-by-default framing (see [`crate::body`])
/// differs from 1.1's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// `HTTP/1.0`
    Http10,
    /// `HTTP/1.1`
    Http11,
}

impl Version {
    pub(crate) fn parse(token: &str) -> Result<Version> {
        match token {
            "HTTP/1.0" => Ok(Version::Http10),
            "HTTP/1.1" => Ok(Version::Http11),
            other => Err(Error::InvalidHead(format!(
                "unsupported HTTP version `{other}`"
            ))),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_versions() {
        assert_eq!(Version::parse("HTTP/1.1").unwrap(), Version::Http11);
        assert_eq!(Version::parse("HTTP/1.0").unwrap(), Version::Http10);
    }

    #[test]
    fn rejects_other_versions() {
        assert!(Version::parse("HTTP/2.0").is_err());
        assert!(Version::parse("garbage").is_err());
    }
}
