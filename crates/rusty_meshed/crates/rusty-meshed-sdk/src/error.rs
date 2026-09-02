//! SDK exception types -- the Rust port of `meshed.sdk.exceptions`.
//! Raised by SDK components when platform contracts are violated or
//! remote registry calls fail.

use rusty_err::Error;

/// Raised when the consumed schema version does not match the expected
/// contract. A one-variant enum rather than a tuple struct:
/// `rusty_err`'s `#[derive(Error)]` only supports enums today.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContractVersionMismatch {
    /// `{0}` is the contract version the consumer declared; `{1}` is
    /// the version resolved from the Data Product Registry. Formatted
    /// with single quotes to match the Python source's `!r` (`repr`)
    /// formatting of plain ASCII strings.
    #[error("Contract version mismatch: expected '{0}', got '{1}'")]
    Mismatch(String, String),
}

impl ContractVersionMismatch {
    /// Builds a mismatch error from the expected and actual contract
    /// versions.
    pub fn new(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        ContractVersionMismatch::Mismatch(expected.into(), actual.into())
    }

    /// The contract version the consumer declared.
    pub fn expected(&self) -> &str {
        let ContractVersionMismatch::Mismatch(expected, _) = self;
        expected
    }

    /// The version resolved from the Data Product Registry.
    pub fn actual(&self) -> &str {
        let ContractVersionMismatch::Mismatch(_, actual) = self;
        actual
    }
}

/// Raised when a Data Product Registry HTTP call fails. A one-variant
/// enum for the same `rusty_err`-derive reason as
/// [`ContractVersionMismatch`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Human-readable description of the failure.
    #[error("{0}")]
    Message(String),
}

impl RegistryError {
    /// Builds a registry error from a message.
    pub fn new(message: impl Into<String>) -> Self {
        RegistryError::Message(message.into())
    }

    /// The human-readable failure description.
    pub fn message(&self) -> &str {
        let RegistryError::Message(message) = self;
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_version_mismatch_formats_with_single_quotes() {
        let err = ContractVersionMismatch::new("1.0.0", "2.0.0");
        assert_eq!(
            err.to_string(),
            "Contract version mismatch: expected '1.0.0', got '2.0.0'"
        );
        assert_eq!(err.expected(), "1.0.0");
        assert_eq!(err.actual(), "2.0.0");
    }

    #[test]
    fn registry_error_formats_as_the_bare_message() {
        let err = RegistryError::new("Failed to register product 'foo': HTTP 500");
        assert_eq!(
            err.to_string(),
            "Failed to register product 'foo': HTTP 500"
        );
        assert_eq!(err.message(), "Failed to register product 'foo': HTTP 500");
    }
}
