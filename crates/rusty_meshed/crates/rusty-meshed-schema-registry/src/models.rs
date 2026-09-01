//! Data models for Schema Registry compatibility enforcement -- the
//! Rust port of `meshed.schema_registry.models`.

use rusty_err::Error;

/// All Schema Registry compatibility modes (REG-150). Values match the
/// string form the Schema Registry REST API expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompatibilityMode {
    Backward,
    BackwardTransitive,
    Forward,
    ForwardTransitive,
    Full,
    FullTransitive,
    None,
}

impl CompatibilityMode {
    /// Every member, in the Python enum's declaration order -- used to
    /// build the "valid modes" list in an invalid-mode error message.
    pub const ALL: [CompatibilityMode; 7] = [
        CompatibilityMode::Backward,
        CompatibilityMode::BackwardTransitive,
        CompatibilityMode::Forward,
        CompatibilityMode::ForwardTransitive,
        CompatibilityMode::Full,
        CompatibilityMode::FullTransitive,
        CompatibilityMode::None,
    ];

    /// The wire-format string value.
    pub fn as_str(self) -> &'static str {
        match self {
            CompatibilityMode::Backward => "BACKWARD",
            CompatibilityMode::BackwardTransitive => "BACKWARD_TRANSITIVE",
            CompatibilityMode::Forward => "FORWARD",
            CompatibilityMode::ForwardTransitive => "FORWARD_TRANSITIVE",
            CompatibilityMode::Full => "FULL",
            CompatibilityMode::FullTransitive => "FULL_TRANSITIVE",
            CompatibilityMode::None => "NONE",
        }
    }

    /// Parses a wire-format string into a mode, or `None` if it isn't
    /// one of the 7 valid values.
    pub fn parse(value: &str) -> Option<Self> {
        CompatibilityMode::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
    }
}

impl std::fmt::Display for CompatibilityMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raised when a schema registration is rejected due to a compatibility
/// violation. A one-variant enum rather than a tuple struct:
/// `rusty_err`'s `#[derive(Error)]` only supports enums today.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompatibilityViolation {
    /// `{0}` is the subject that rejected the schema; `{1}` is the
    /// registry's original error message.
    #[error("Schema incompatible with {0}: {1}")]
    Violation(String, String),
}

impl CompatibilityViolation {
    /// Builds a violation from the rejecting subject and the registry's
    /// original error message.
    pub fn new(subject: impl Into<String>, message: impl Into<String>) -> Self {
        CompatibilityViolation::Violation(subject.into(), message.into())
    }

    /// The Schema Registry subject name that rejected the schema.
    pub fn subject(&self) -> &str {
        let CompatibilityViolation::Violation(subject, _) = self;
        subject
    }

    /// The original error message from the Schema Registry.
    pub fn message(&self) -> &str {
        let CompatibilityViolation::Violation(_, message) = self;
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_the_python_enum_values() {
        assert_eq!(CompatibilityMode::Backward.as_str(), "BACKWARD");
        assert_eq!(
            CompatibilityMode::BackwardTransitive.as_str(),
            "BACKWARD_TRANSITIVE"
        );
        assert_eq!(CompatibilityMode::Forward.as_str(), "FORWARD");
        assert_eq!(
            CompatibilityMode::ForwardTransitive.as_str(),
            "FORWARD_TRANSITIVE"
        );
        assert_eq!(CompatibilityMode::Full.as_str(), "FULL");
        assert_eq!(
            CompatibilityMode::FullTransitive.as_str(),
            "FULL_TRANSITIVE"
        );
        assert_eq!(CompatibilityMode::None.as_str(), "NONE");
    }

    #[test]
    fn parse_round_trips_every_member() {
        for mode in CompatibilityMode::ALL {
            assert_eq!(CompatibilityMode::parse(mode.as_str()), Some(mode));
        }
    }

    #[test]
    fn parse_rejects_unknown_string() {
        assert_eq!(CompatibilityMode::parse("INVALID"), None);
    }

    #[test]
    fn compatibility_violation_formats_as_expected() {
        let violation =
            CompatibilityViolation::new("my-subject", "Schema being registered is incompatible");
        assert_eq!(
            violation.to_string(),
            "Schema incompatible with my-subject: Schema being registered is incompatible"
        );
        assert_eq!(violation.subject(), "my-subject");
    }
}
