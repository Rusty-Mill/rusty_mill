//! Replay-file ingestion adapter — see `RB-VERIFY-001`.
//!
//! This crate is the `boxcars`-backed implementation of
//! `rb_domain::PhysicsStateSource`. The `boxcars` dependency and actual
//! parsing logic are Phase 0 delivery work, not bootstrap scaffolding — see
//! `docs/roadmap/ROADMAP.md`. Until then this adapter exists (so the port
//! and composition root can be wired and tested) but reports
//! `IngestError::NotImplemented`.

use rb_domain::{IngestError, PhysicsFrame, PhysicsStateSource};
use std::path::PathBuf;

pub struct ReplayFileSource {
    pub path: PathBuf,
}

impl ReplayFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl PhysicsStateSource for ReplayFileSource {
    fn frames(&self) -> Result<Vec<PhysicsFrame>, IngestError> {
        Err(IngestError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_adapter_reports_not_implemented_not_a_panic() {
        let source = ReplayFileSource::new("fixtures/example.replay");
        let result = source.frames();
        assert!(matches!(result, Err(IngestError::NotImplemented)));
    }
}
