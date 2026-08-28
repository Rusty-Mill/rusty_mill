//! Domain types and ports for `rusty_bullet`.
//!
//! This crate holds logic the rest of the workspace depends on but that must
//! stay free of I/O: physics frame data, the divergence metric the whole
//! project tunes against (see `RB-VERIFY-003`), and the `PhysicsStateSource`
//! port that replay/capture ingestion adapters implement (see
//! `RB-VERIFY-001`, `RB-VERIFY-002`). Nothing here reads a file, opens a
//! socket, or spawns a process — that belongs in an adapter crate.

pub mod divergence;
pub mod error;
pub mod port;
pub mod state;

pub use divergence::DivergenceScore;
pub use error::IngestError;
pub use port::PhysicsStateSource;
pub use state::{BallState, CarState, PhysicsFrame};
