//! Errors produced while parsing a URL.

use core::fmt;

/// Errors that can occur during URL parsing.
///
/// This may be extended in the future so exhaustive matching is forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The host is empty, but the URL's scheme requires a non-empty host
    /// (e.g. `http://` or `file://`).
    EmptyHost,
    /// The domain failed IDNA processing (invalid international domain
    /// name).
    IdnaError,
    /// A domain contained a character that is not allowed by the URL
    /// Standard's domain code-point rules.
    InvalidDomainCharacter,
    /// The host looked like an IPv4 address but was not a valid one.
    InvalidIpv4Address,
    /// The host was a bracketed `[...]` literal but was not a valid IPv6
    /// address.
    InvalidIpv6Address,
    /// The port could not be parsed as a `u16`.
    InvalidPort,
    /// The resulting URL string would be more than 4 GB, which this crate
    /// does not support.
    Overflow,
    /// A relative reference (e.g. `path/to/resource`) was parsed against a
    /// base URL that cannot be a base (an opaque-path URL like
    /// `data:text/plain,hello`).
    RelativeUrlWithCannotBeABaseBase,
    /// A relative reference was parsed without supplying a base URL.
    RelativeUrlWithoutBase,
    /// Attempted to set the host on a URL that cannot be a base.
    SetHostOnCannotBeABaseUrl,
}

impl ParseError {
    fn description(&self) -> &'static str {
        match self {
            ParseError::EmptyHost => "empty host",
            ParseError::IdnaError => "invalid international domain name",
            ParseError::InvalidDomainCharacter => "invalid domain character",
            ParseError::InvalidIpv4Address => "invalid IPv4 address",
            ParseError::InvalidIpv6Address => "invalid IPv6 address",
            ParseError::InvalidPort => "invalid port number",
            ParseError::Overflow => "URLs more than 4 GB are not supported",
            ParseError::RelativeUrlWithCannotBeABaseBase => {
                "relative URL with a cannot-be-a-base base"
            }
            ParseError::RelativeUrlWithoutBase => "relative URL without a base",
            ParseError::SetHostOnCannotBeABaseUrl => {
                "a cannot-be-a-base URL doesn\u{2019}t have a host to set"
            }
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl std::error::Error for ParseError {}

impl From<idna::Errors> for ParseError {
    fn from(_: idna::Errors) -> Self {
        ParseError::IdnaError
    }
}

#[cfg(test)]
mod tests {
    use super::ParseError;

    #[test]
    fn display_matches_documented_message() {
        assert_eq!(ParseError::EmptyHost.to_string(), "empty host");
        assert_eq!(
            ParseError::IdnaError.to_string(),
            "invalid international domain name"
        );
        assert_eq!(
            ParseError::InvalidDomainCharacter.to_string(),
            "invalid domain character"
        );
        assert_eq!(
            ParseError::InvalidIpv4Address.to_string(),
            "invalid IPv4 address"
        );
        assert_eq!(
            ParseError::InvalidIpv6Address.to_string(),
            "invalid IPv6 address"
        );
        assert_eq!(ParseError::InvalidPort.to_string(), "invalid port number");
        assert_eq!(
            ParseError::Overflow.to_string(),
            "URLs more than 4 GB are not supported"
        );
        assert_eq!(
            ParseError::RelativeUrlWithCannotBeABaseBase.to_string(),
            "relative URL with a cannot-be-a-base base"
        );
        assert_eq!(
            ParseError::RelativeUrlWithoutBase.to_string(),
            "relative URL without a base"
        );
        assert_eq!(
            ParseError::SetHostOnCannotBeABaseUrl.to_string(),
            "a cannot-be-a-base URL doesn\u{2019}t have a host to set"
        );
    }

    #[test]
    fn is_clone_copy_eq() {
        let err = ParseError::EmptyHost;
        let cloned = err;
        assert_eq!(err, cloned);
        assert_ne!(ParseError::EmptyHost, ParseError::InvalidPort);
    }

    #[test]
    fn implements_std_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        assert_error(&ParseError::Overflow);
    }
}
