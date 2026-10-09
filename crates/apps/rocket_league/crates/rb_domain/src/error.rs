//! Error types for ingestion adapters. Manual `Display`/`Error` impls: a
//! two-variant enum doesn't earn a macro-derive dependency yet (see
//! `RB-VERIFY-001`/`RB-VERIFY-002` non-goals).

use std::fmt;

#[derive(Debug)]
pub enum IngestError {
    /// The adapter exists but its parsing logic isn't implemented yet.
    /// Distinct from `Malformed` so callers (and tests) can tell "not built"
    /// from "built, but this input is bad".
    NotImplemented,
    /// The source file/stream was read but its contents don't match the
    /// expected format.
    Malformed(String),
    /// The underlying I/O operation failed.
    Io(String),
}

impl fmt::Display for IngestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IngestError::NotImplemented => write!(f, "ingestion not implemented yet"),
            IngestError::Malformed(msg) => write!(f, "malformed input: {msg}"),
            IngestError::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for IngestError {}
