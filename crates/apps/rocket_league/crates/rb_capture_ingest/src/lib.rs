//! BakkesMod offline-capture ingestion adapter — see `RB-VERIFY-002`.
//!
//! This crate implements `rb_domain::PhysicsStateSource` from a capture file
//! recorded by BakkesMod plugin tooling in local/offline matches (the only
//! setting where BakkesMod is usable now that Easy Anti-Cheat blocks it
//! online — see `docs/research/RESEARCH-BACKLOG.md`). The capture file
//! format and the BakkesMod-side plugin that writes it are Phase 0 delivery
//! work, not bootstrap scaffolding. Until then this adapter reports
//! `IngestError::NotImplemented`.

use rb_domain::{IngestError, PhysicsFrame, PhysicsStateSource};
use std::path::PathBuf;

pub struct CaptureFileSource {
    pub path: PathBuf,
}

impl CaptureFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl PhysicsStateSource for CaptureFileSource {
    fn frames(&self) -> Result<Vec<PhysicsFrame>, IngestError> {
        Err(IngestError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_adapter_reports_not_implemented_not_a_panic() {
        let source = CaptureFileSource::new("fixtures/example.capture");
        let result = source.frames();
        assert!(matches!(result, Err(IngestError::NotImplemented)));
    }
}
