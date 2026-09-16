//! The fixed failure taxonomy (ADR-0021) and the structured attribution record.

use serde::{Deserialize, Serialize};

/// The paper's fixed eight-member failure taxonomy (ADR-0021). snake_case on the
/// wire (ADR-0025): `f_context`, `f_tool`, … The authoritative encoding lives in
/// data-model §5/§7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureType {
    /// Wrong/missing context fed to the model.
    FContext,
    /// Tool errored, was blocked, or behaved incorrectly.
    FTool,
    /// Tool result/observation not surfaced or misread.
    FFeedback,
    /// Verification absent, skipped, or judged unavailable.
    FVerify,
    /// Failed to recover after an error (loop, no retry).
    FRecovery,
    /// Maintenance burden introduced (tests weakened, residue).
    FEntropy,
    /// Model reasoning/output defect with an adequate harness.
    FModel,
    /// Unattributable.
    FUnknown,
}

/// Why a turn failed verification, drawn from the frozen `(category, layer)` →
/// `FailureType` matrix (PRD 05). Structured so attribution aggregates and never
/// requires parsing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// The check that produced this attribution.
    pub check: String,
    /// Fixed taxonomy bucket (ADR-0021).
    pub failure_type: FailureType,
    /// Frozen category vocabulary (matrix).
    pub category: String,
    /// Frozen layer vocabulary (matrix).
    pub layer: String,
    /// Human-readable evidence for the attribution.
    pub evidence: String,
}
