use rusty_err::BoxError;

/// Shared error type for the `rusty_hister` crate cluster.
///
/// Kept small and open-ended (a catch-all [`HisterError::Other`] boxes any
/// sovereign [`rusty_err::Error`]) since most call sites in this bootstrap
/// phase are extractor/config validation, not I/O — new named variants get
/// added as concrete failure modes need distinguishing, not speculatively.
#[derive(Debug, rusty_err::Error)]
pub enum HisterError {
    /// An extractor's [`crate::ExtractorConfig`] failed validation (e.g. an
    /// unknown option key — Hister's own "unknown option key = hard error"
    /// pattern, capability inventory §4.6).
    #[error("invalid extractor configuration: {0}")]
    InvalidConfig(String),
    /// An extractor could not produce a result for reasons worth reporting
    /// distinctly from a plain fallback (e.g. a malformed document it
    /// matched but cannot actually parse).
    #[error("extraction failed: {0}")]
    Extraction(String),
    /// Catch-all for a boxed lower-level error (I/O, parsing, a future
    /// backend's own error type) that doesn't yet warrant its own variant.
    #[error("{0}")]
    Other(BoxError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_config_message_includes_detail() {
        let err = HisterError::InvalidConfig("unknown option `foo`".to_string());
        assert_eq!(
            err.to_string(),
            "invalid extractor configuration: unknown option `foo`"
        );
    }

    #[test]
    fn extraction_message_includes_detail() {
        let err = HisterError::Extraction("empty document body".to_string());
        assert_eq!(err.to_string(), "extraction failed: empty document body");
    }

    #[derive(Debug)]
    struct StubError;

    impl core::fmt::Display for StubError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "stub failure")
        }
    }

    impl core::error::Error for StubError {}

    #[test]
    fn other_wraps_and_displays_boxed_error() {
        let err = HisterError::Other(BoxError::new(StubError));
        assert_eq!(err.to_string(), "stub failure");
    }

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Send + Sync>() {}

    #[test]
    fn hister_error_is_send_and_sync() {
        // Matches rusty_err's own regression test for BoxError-backed
        // variants: this must hold for HisterError to be usable as the
        // error type of Send futures later (e.g. an async extractor path).
        assert_send::<HisterError>();
        assert_sync::<HisterError>();
    }
}
