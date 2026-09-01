//! [`DomainEvent`] -- the trait `rusty-meshed-domains`' nine event
//! structs (DOM-002..010) implement, and `rusty-meshed-sdk`'s
//! `DataProductProducerBase`/`DataProductConsumerBase` (SDK-013..039)
//! are built against, rather than against any one concrete event type.
//!
//! `BaseEvent`'s own module doc flagged this as an open question when
//! only nine standalone event structs existed with nothing to
//! polymorphically dispatch over; `rusty-meshed-domains::events::mod`
//! flagged it again, explicitly naming the trigger: "add one if/when a
//! real caller needs to hold a `Box<dyn DomainEvent>` or a generic `E:
//! DomainEvent`." `DataProductProducerBase::publish<E: DomainEvent>`
//! and `OutputPortSpec<E>::describe` (both requiring `E: DomainEvent`)
//! are that caller.
//!
//! Lives here, not in `rusty-meshed-domains` alongside its
//! implementors: `rusty-meshed-sdk` needs to name this trait in
//! `DataProductProducerBase`/`DataProductConsumerBase`'s own bounds
//! without depending on `rusty-meshed-domains` (a sibling crate this
//! family keeps domain-agnostic on purpose -- the SDK crate has never
//! depended on the domains crate, and adding that edge just to reach a
//! trait would invert the layering); `rusty-meshed-core` is the one
//! crate both already depend on, same reasoning [`BaseEvent`] and
//! [`crate::EventType`] are here instead of in either.

use crate::avro::AvroDecodeError;
use crate::BaseEvent;

/// What a concrete domain event type (`PersonnelAssigned`,
/// `PositionFilled`, ...) provides so `rusty-meshed-sdk`'s producer/
/// consumer base types can work with it without knowing its concrete
/// shape -- the Rust stand-in for the source's `isinstance(event,
/// BaseEvent)` duck typing (SDK-022) and `event_type: type[BaseEvent]`
/// class-object parameter (SDK-032's `event_type.__name__`,
/// consumer-side `event_type.avro_schema()`/`event_type(**data)`).
///
/// `Self: Sized` (implied by the associated `const`/`fn` below having
/// no receiver) rules out `dyn DomainEvent` -- nothing in this crate
/// family needs one: every real caller (`publish<E: DomainEvent>`,
/// `OutputPortSpec<E>`) is generic per call site or per port
/// declaration, never holding a heterogeneous runtime collection of
/// events that would need trait-object erasure.
pub trait DomainEvent {
    /// The concrete Rust type's own name -- what `type(event).__name__`
    /// resolves to in the source. Used for registry contract validation
    /// (SDK-032: `event_type.__name__ not in schema_ref`), not for
    /// output-port registration (`RegistryClient::register_output_port`
    /// already collapsed that to `event_classification` alone -- see
    /// SDK-046 in `registry_client`'s own module doc).
    const EVENT_NAME: &'static str;

    /// Access to this instance's embedded lineage contract -- what
    /// `rusty-meshed-sdk`'s `DataProductProducerBase::publish` reads to
    /// build lineage headers (SDK-024) and what
    /// `LineageTracker::record_event` is called with (SDK-026).
    fn base(&self) -> &BaseEvent;

    /// This event type's Avro record schema (SDK-007/016), including
    /// [`BaseEvent`]'s four lineage fields.
    fn avro_schema() -> String
    where
        Self: Sized;

    /// Avro-encodes this instance, lineage fields first (SDK-008).
    fn serialize(&self) -> Vec<u8>;

    /// Decodes bytes produced by [`serialize`](Self::serialize)
    /// (SDK-008).
    fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError>
    where
        Self: Sized;
}
