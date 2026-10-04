//! Pure domain core of `rusty_rsi`, an AIDE²-style recursive
//! self-improvement harness (ADR-0005).
//!
//! Nothing here performs I/O, reads a clock or spawns work: every function
//! is deterministic given its arguments, which is what lets a lineage entry
//! be replayed. Adapters that touch the outside world live in `rsi-runtime`.
//!
//! - [`score`]: [`Score`] and [`Grade`], both confined to `[0, 1]`.
//! - [`budget`]: [`Budget`], [`CostUsage`] and the hard-stop [`CostMeter`].
//! - [`noise`]: the calibrated [`NoiseBand`] and the [`Margin`] it yields.
//! - [`accept`]: the two-stage, fresh-seed accept gate.
//! - [`search`]: greedy, UCB1 and softmax selection helpers.
//! - [`rng`]: the seeded [`SplitMix64`] generator and seed derivation.
//! - [`lineage`]: lineage entry types and the SHA-256 hash chain.

pub mod accept;
pub mod budget;
pub mod error;
pub mod lineage;
pub mod noise;
pub mod rng;
pub mod score;
pub mod search;

pub use accept::{confirm, screen, Challenger, Decision, Evaluation, Rejection, Screen};
pub use budget::{Budget, BudgetExhausted, CostMeter, CostUsage};
pub use error::CoreError;
pub use lineage::{
    BlobId, CandidateId, ChainError, ChainRecord, CommitSha, EvaluationRecord, LineageEntry,
    ModelId, TaskId, TaskResult,
};
pub use noise::{Margin, NoiseBand};
pub use rng::{derive_seed, seed_set, Seed, SplitMix64};
pub use score::{Grade, Score};
pub use search::{argmax, softmax_sample, ucb1, Arm};
