//! Boundary error rendering for the Tauri adapter (PRD 06 / IPC contract §4).
//!
//! Every failing `invoke` rejects with `{ kind, message }` where `kind` is one of
//! the six [`BoundaryError`] surface kinds from the single contract SSOT
//! (`rk_app::contract`). The frontend renders the rejection uniformly and unlocks
//! the composer — a failed turn never emits `rk://turn_complete`, so the `catch`
//! is the only path that clears the lock on failure.

use rk_app::contract::BoundaryError;
use serde::Serialize;

/// The `invoke` rejection payload: the taxonomy kind plus a human-readable
/// message. Serializes to `{ "kind": "<snake_case>", "message": "…" }`.
#[derive(Debug, Clone, Serialize)]
pub struct BoundaryErrorPayload {
    /// The closed-vocabulary surface kind (serialized snake_case).
    pub kind: BoundaryError,
    /// A human-readable detail for the banner.
    pub message: String,
}

impl BoundaryErrorPayload {
    /// Build a payload from a kind and message.
    pub fn new(kind: BoundaryError, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A bug / unexpected-state rejection.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(BoundaryError::Internal, message)
    }

    /// A policy decision that ends the turn (distinct from a recoverable
    /// per-tool block, which stays inside the reply as a `BLOCKED` outcome).
    pub fn policy_block(message: impl Into<String>) -> Self {
        Self::new(BoundaryError::PolicyBlock, message)
    }
}

/// Collapse a turn-level error into a boundary kind. The kernel today surfaces
/// provider failures as `KernelError::Model(..)`; we read the message to separate
/// the auth / rate-limit / timeout sub-cases the taxonomy distinguishes, falling
/// back to `internal` for anything unrecognised.
pub fn classify(err: &anyhow::Error) -> BoundaryErrorPayload {
    let msg = err.to_string();
    let low = msg.to_ascii_lowercase();
    let kind = if low.contains("approvaldenied") || low.contains("policy") {
        BoundaryError::PolicyBlock
    } else if low.contains("401") || low.contains("403") || low.contains("unauthorized") {
        BoundaryError::AuthError
    } else if low.contains("429") || low.contains("rate limit") || low.contains("rate-limit") {
        BoundaryError::RateLimited
    } else if low.contains("timeout") || low.contains("timed out") {
        BoundaryError::Timeout
    } else if low.contains("model error") || low.contains("provider") {
        BoundaryError::ProviderError
    } else {
        BoundaryError::Internal
    };
    BoundaryErrorPayload::new(kind, msg)
}
