//! Timestamp handling.
//!
//! Phase-1 decision (DESIGN.md): timestamps stay as validated-by-provenance
//! *strings*. The status CLI does no time arithmetic, Go emits RFC3339 with
//! variable sub-second precision, and preserving the exact byte
//! representation keeps round-trips faithful. Revisit when Phase 2 needs
//! key-expiry math.

/// An RFC3339 timestamp kept in its original string form.
///
/// Go's zero `time.Time` marshals as `"0001-01-01T00:00:00Z"`; see
/// [`Rfc3339::is_zero`].
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Rfc3339(pub String);

impl Rfc3339 {
    /// True for Go's zero time (or an empty/absent value).
    pub fn is_zero(&self) -> bool {
        self.0.is_empty() || self.0 == "0001-01-01T00:00:00Z"
    }
}

#[cfg(test)]
mod tests {
    use super::Rfc3339;

    #[test]
    fn zero_detection() {
        assert!(Rfc3339::default().is_zero());
        assert!(Rfc3339("0001-01-01T00:00:00Z".into()).is_zero());
        assert!(!Rfc3339("2026-07-09T01:58:01.695423681Z".into()).is_zero());
    }
}
