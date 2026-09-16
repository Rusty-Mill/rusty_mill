//! Registry-local enumerations -- the Rust port of
//! `meshed.registry.enums.MaturityTier` (REG-012). `EventType`
//! (REG-013) lives in `rusty-meshed-core` instead, since it's shared
//! cross-cutting vocabulary the SDK and domain crates also need -- see
//! that crate's `event_type` module doc.

/// How mature a data product is in terms of governance, observability,
/// and consumer support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MaturityTier {
    /// Minimum viable — basic registration, schema, one output port.
    #[default]
    Mvp,
    /// Adds multiple ports, SLO monitoring, quality assertions.
    Enhanced,
    /// Full contract governance, SLO alerting, consumer-driven
    /// contracts.
    Mature,
}

impl MaturityTier {
    /// The wire/storage string value.
    pub fn as_str(self) -> &'static str {
        match self {
            MaturityTier::Mvp => "mvp",
            MaturityTier::Enhanced => "enhanced",
            MaturityTier::Mature => "mature",
        }
    }

    /// Parses a stored/wire string back into a tier.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "mvp" => MaturityTier::Mvp,
            "enhanced" => MaturityTier::Enhanced,
            "mature" => MaturityTier::Mature,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_mvp() {
        assert_eq!(MaturityTier::default(), MaturityTier::Mvp);
    }

    #[test]
    fn parse_round_trips_every_member() {
        for tier in [
            MaturityTier::Mvp,
            MaturityTier::Enhanced,
            MaturityTier::Mature,
        ] {
            assert_eq!(MaturityTier::parse(tier.as_str()), Some(tier));
        }
    }

    #[test]
    fn parse_rejects_unknown_values() {
        assert_eq!(MaturityTier::parse("not-a-real-tier"), None);
    }
}
