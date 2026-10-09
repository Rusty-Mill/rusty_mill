//! The port both ingestion adapters implement. Two real implementations
//! (`rb_replay_ingest`, `rb_capture_ingest`) exist at design time, which is
//! what justifies this trait existing now rather than being deferred until
//! a second call site shows up.

use crate::error::IngestError;
use crate::state::PhysicsFrame;

/// A source of ground-truth physics frames, in timestamp order.
///
/// Implemented by `rb_replay_ingest` (real online/offline matches via
/// `boxcars`, no raw inputs) and `rb_capture_ingest` (BakkesMod offline
/// captures, raw inputs + physics state, local play only). See
/// `RB-VERIFY-001` and `RB-VERIFY-002` for why neither alone is sufficient
/// and why both are needed.
pub trait PhysicsStateSource {
    fn frames(&self) -> Result<Vec<PhysicsFrame>, IngestError>;
}
