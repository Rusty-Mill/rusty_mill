//! SDK types for declaring output ports on meshed data products -- the
//! Rust port of `meshed.sdk.types`. Output port declarations are the
//! primary mechanism through which producers advertise their event
//! contracts to the platform registry and consumers.

use rusty_meshed_core::{DomainEvent, EventType};
use std::marker::PhantomData;

/// Immutable specification for a data product output port. Generic
/// over `E`, the event type this port publishes -- ties a port
/// declaration to its event schema type at compile time, the same role
/// `event_type: type[BaseEvent]` plays in the Python source.
///
/// Immutability here (SDK-010, `@dataclass(frozen=True)` in the
/// source) is structural: fields are private with no setters, so a
/// mutation attempt is a compile error rather than the Python source's
/// runtime `FrozenInstanceError` -- a stronger guarantee, not a weaker
/// one.
///
/// `Clone`/`Debug`/`PartialEq`/`Eq` are implemented by hand rather than
/// derived: a `#[derive(...)]` on a type with a bare `PhantomData<E>`
/// field adds a spurious `E: Clone`/`E: Debug`/etc. bound even though
/// `PhantomData<E>` itself never needs one (the same pitfall this
/// crate family already hit once in `rusty-meshed-governance`'s
/// `GovernanceEngine`).
pub struct OutputPortSpec<E> {
    name: String,
    topic: String,
    event_classification: EventType,
    _event_type: PhantomData<E>,
}

impl<E> OutputPortSpec<E> {
    /// Builds a new output port spec. `E` is inferred from context (a
    /// type ascription or the surrounding function's return type) since
    /// nothing about the constructor's arguments names it.
    pub fn new(
        name: impl Into<String>,
        topic: impl Into<String>,
        event_classification: EventType,
    ) -> Self {
        OutputPortSpec {
            name: name.into(),
            topic: topic.into(),
            event_classification,
            _event_type: PhantomData,
        }
    }

    /// Human-readable port identifier (e.g. `"assignments"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Kafka topic name events on this port are published to.
    pub fn topic(&self) -> &str {
        &self.topic
    }

    /// Semantic classification of events on this port (delta, state,
    /// or measurement).
    pub fn event_classification(&self) -> EventType {
        self.event_classification
    }
}

impl<E: DomainEvent> OutputPortSpec<E> {
    /// Type-erases this port declaration into a [`PortDescriptor`],
    /// resolving `E::avro_schema()` in the process -- what
    /// `DataProductProducerBase::new` takes instead of
    /// `OutputPortSpec<E>` directly, since a producer's output ports
    /// span multiple, generally *different* concrete event types (the
    /// source's own `list[OutputPortSpec]` is heterogeneous the same
    /// way), which a single `Vec<OutputPortSpec<E>>` can't express in
    /// Rust the way Python's runtime typing can.
    pub fn describe(&self) -> PortDescriptor {
        PortDescriptor {
            name: self.name.clone(),
            topic: self.topic.clone(),
            event_classification: self.event_classification,
            schema: E::avro_schema(),
        }
    }
}

/// A type-erased [`OutputPortSpec`] -- everything
/// `DataProductProducerBase::startup` needs from a declared output
/// port (SDK-013..021) without needing to know its event type at the
/// producer's own struct level. Built via [`OutputPortSpec::describe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDescriptor {
    pub name: String,
    pub topic: String,
    pub event_classification: EventType,
    /// `E::avro_schema()`, resolved at `describe()` time.
    pub schema: String,
}

impl<E> Clone for OutputPortSpec<E> {
    fn clone(&self) -> Self {
        OutputPortSpec {
            name: self.name.clone(),
            topic: self.topic.clone(),
            event_classification: self.event_classification,
            _event_type: PhantomData,
        }
    }
}

impl<E> PartialEq for OutputPortSpec<E> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.topic == other.topic
            && self.event_classification == other.event_classification
    }
}

impl<E> Eq for OutputPortSpec<E> {}

impl<E> std::fmt::Debug for OutputPortSpec<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputPortSpec")
            .field("name", &self.name)
            .field("topic", &self.topic)
            .field("event_classification", &self.event_classification)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in event type for tests that only exercise
    /// `OutputPortSpec`'s own fields -- `OutputPortSpec<E>` itself
    /// doesn't require anything of `E` beyond being a type.
    /// [`DescribableEvent`] below is the stand-in for `describe()`'s
    /// own test, the one place here that does require `E: DomainEvent`.
    struct SampleEvent;

    struct DescribableEvent {
        base: rusty_meshed_core::BaseEvent,
    }

    impl DomainEvent for DescribableEvent {
        const EVENT_NAME: &'static str = "DescribableEvent";

        fn base(&self) -> &rusty_meshed_core::BaseEvent {
            &self.base
        }

        fn avro_schema() -> String {
            r#"{"type":"record","name":"DescribableEvent","fields":[]}"#.to_string()
        }

        fn serialize(&self) -> Vec<u8> {
            Vec::new()
        }

        fn deserialize(_bytes: &[u8]) -> Result<Self, rusty_meshed_core::AvroDecodeError> {
            unimplemented!("describe() never calls this")
        }

        fn to_json(&self) -> rusty_json::Value {
            unimplemented!("describe() never calls this")
        }
    }

    #[test]
    fn constructs_with_the_given_fields() {
        let spec: OutputPortSpec<SampleEvent> = OutputPortSpec::new(
            "assignments",
            "manpower.personnel-lifecycle.assignments",
            EventType::Delta,
        );
        assert_eq!(spec.name(), "assignments");
        assert_eq!(spec.topic(), "manpower.personnel-lifecycle.assignments");
        assert_eq!(spec.event_classification(), EventType::Delta);
    }

    #[test]
    fn clone_produces_an_equal_independent_copy() {
        let spec: OutputPortSpec<SampleEvent> = OutputPortSpec::new(
            "assessments",
            "manpower.readiness-reporting.assessments",
            EventType::Measurement,
        );
        let cloned = spec.clone();
        assert_eq!(spec, cloned);
    }

    #[test]
    fn equality_compares_all_three_fields() {
        let a: OutputPortSpec<SampleEvent> = OutputPortSpec::new("a", "t", EventType::Delta);
        let b: OutputPortSpec<SampleEvent> = OutputPortSpec::new("a", "t", EventType::State);
        assert_ne!(a, b);
    }

    #[test]
    fn describe_type_erases_the_port_and_resolves_the_schema() {
        let spec: OutputPortSpec<DescribableEvent> = OutputPortSpec::new(
            "assignments",
            "manpower.personnel-lifecycle.assignments",
            EventType::Delta,
        );
        let descriptor = spec.describe();
        assert_eq!(descriptor.name, "assignments");
        assert_eq!(descriptor.topic, "manpower.personnel-lifecycle.assignments");
        assert_eq!(descriptor.event_classification, EventType::Delta);
        assert_eq!(descriptor.schema, DescribableEvent::avro_schema());
    }
}
