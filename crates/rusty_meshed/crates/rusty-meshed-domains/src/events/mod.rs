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
//! No shared `DomainEvent` trait unifies these nine: nothing in this
//! pass needs to treat them polymorphically (no producer/consumer
//! generic dispatch has landed yet, DOM-012 onward), so introducing
//! one now would be an abstraction with no caller -- exactly what this
//! crate family avoids elsewhere (see e.g.
//! `rusty-meshed-observability::slo`'s `check_freshness`/
//! `check_completeness` staying separate despite near-identical
//! bodies). Add one if/when a real caller needs to hold a
//! `Box<dyn DomainEvent>` or a generic `E: DomainEvent`.

mod personnel;
mod position;
mod readiness;

pub use personnel::{PersonnelAssigned, PersonnelPromoted, PersonnelSeparated, StatusChanged};
pub use position::{
    PositionAuthorizationChanged, PositionFilled, PositionModified, PositionVacated,
};
pub use readiness::UnitReadinessAssessed;
