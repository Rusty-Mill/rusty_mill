//! `observe`'s error enum (ADR-0023; error-handling §2). Wraps `ConfigError`
//! from below the DAG via `#[from]`.

/// Errors from the observe layer (tracer, and the evidence journal in Phase 2).
#[derive(Debug, thiserror::Error)]
pub enum ObserveError {
    /// Filesystem failure writing/reading a journal.
    #[error("observe io error: {0}")]
    Io(#[from] std::io::Error),
    /// A record failed to (de)serialize.
    #[error("observe serde error: {0}")]
    Serde(#[from] serde_json::Error),
    /// A torn / partial append-only record at `line` (recovered by skipping).
    #[error("torn record at line {line}")]
    TornRecord {
        /// 1-based line number of the torn record.
        line: usize,
    },
    /// Configuration error surfaced through observe.
    #[error(transparent)]
    Config(#[from] rk_config::ConfigError),
}
