//! Domain event types for the meshed manpower bounded context -- the
//! Rust port of `meshed.domains.events` (DOM-002..011).
//!
//! Nine events across three files, mirroring the source's own
//! `personnel.py`/`position.py`/`readiness.py` split. Each event
//! embeds a `base: BaseEvent` field (composition, not inheritance --
//! see `rusty_meshed_core::BaseEvent`'s module doc for why) plus its
//! own typed fields, and hand-implements its own
//! `avro_schema()`/`serialize()`/`deserialize()` built on
//! `rusty_meshed_core::avro`'s primitives and
//! `BaseEvent::avro_record_schema`/`encode_into`/`decode_from`.
//!
//! All nine implement `rusty_meshed_core::DomainEvent`, a thin trait
//! delegating straight to each event's own inherent
//! `avro_schema()`/`serialize()`/`deserialize()` (unchanged, still the
//! real implementations) plus a `base()` accessor and an `EVENT_NAME`
//! const -- added once `rusty-meshed-sdk`'s producer/consumer bases
//! (SDK-013..039) needed to hold a generic `E: DomainEvent` rather than
//! one concrete event type per call site. See that trait's own module
//! doc in `rusty_meshed_core` for why it lives there instead of here.

mod personnel;
mod position;
mod readiness;

pub use personnel::{PersonnelAssigned, PersonnelPromoted, PersonnelSeparated, StatusChanged};
pub use position::{
    PositionAuthorizationChanged, PositionFilled, PositionModified, PositionVacated,
};
pub use readiness::UnitReadinessAssessed;
