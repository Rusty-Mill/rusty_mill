//! The digital transformation simulator -- the Rust port of
//! `meshed.transformation` (models, engine, seed data; the HTTP router
//! and SSE event stream are a separate, not-yet-implemented piece, see
//! `capability-manifest.md` rows XFM-027..035).

mod engine;
mod enums;
mod seed;

pub use engine::{
    advance_quarter, ensure_schema, get_or_create_clock, get_state, queue_decision, DecisionRef,
    LegacySystem, MaturityPoint, TransformationState, DUAL_WRITE_MIN_QUARTERS,
};
pub use enums::{CapabilityDimension, DecisionType, SystemStatus};
pub use seed::seed_transformation_state;
