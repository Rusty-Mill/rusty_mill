//! [`EventType`] -- the Rust port of `meshed.registry.enums.EventType`
//! (REG-013). Lives in this shared crate rather than in
//! `rusty-meshed-registry` because it's cross-cutting vocabulary: the
//! registry's `OutputPort` model, the SDK's `OutputPortSpec`
//! (SDK-009), and every domain event's classification (DOM-001..011)
//! all need it, and `rusty-meshed-registry`/`rusty-meshed-sdk`/
//! `rusty-meshed-domains` are siblings with no dependency edge between
//! them -- `rusty-meshed-core` is the one crate all three already
//! depend on.

/// Semantic classification of events published on a data-product output
/// port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    /// An incremental change (create/update/delete) to an entity.
    Delta,
    /// A full snapshot of an entity's current state.
    State,
    /// A metric/observation with no entity-lifecycle meaning of its
    /// own (e.g. `UnitReadinessAssessed`, DOM-010).
    Measurement,
}

impl EventType {
    /// The wire-format string value, matching
    /// `meshed.registry.enums.EventType`'s `str` mixin (`"delta"`,
    /// `"state"`, `"measurement"`) -- used wherever the Python source
    /// serializes `.value` (e.g. SDK-020's output-port registration
    /// call).
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Delta => "delta",
            EventType::State => "state",
            EventType::Measurement => "measurement",
        }
    }

    /// Parses a wire-format string back into an `EventType`. `None` for
    /// anything other than the three valid member values -- callers at
    /// an API boundary (e.g. REG-033's `OutputPortCreate.event_type`)
    /// turn that into their own "422 invalid value" error.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "delta" => EventType::Delta,
            "state" => EventType::State,
            "measurement" => EventType::Measurement,
            _ => return None,
        })
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches_the_python_enum_values() {
        assert_eq!(EventType::Delta.as_str(), "delta");
        assert_eq!(EventType::State.as_str(), "state");
        assert_eq!(EventType::Measurement.as_str(), "measurement");
    }

    #[test]
    fn parse_round_trips_every_member() {
        for event_type in [EventType::Delta, EventType::State, EventType::Measurement] {
            assert_eq!(EventType::parse(event_type.as_str()), Some(event_type));
        }
    }

    #[test]
    fn parse_rejects_unknown_values() {
        assert_eq!(EventType::parse("not-a-real-event-type"), None);
    }
}
