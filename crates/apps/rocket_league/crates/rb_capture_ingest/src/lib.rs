//! BakkesMod offline-capture ingestion adapter — see `RB-VERIFY-002`.
//!
//! Parses the JSON-Lines capture file format decided in ADR-0005: one JSON
//! object per physics tick, `{"timestamp_secs", "ball", "cars"}` (each car
//! entry carrying an `"input"` object), written by BakkesMod plugin tooling
//! in local/offline matches — the only setting where BakkesMod is usable now
//! that Easy Anti-Cheat blocks it online (see
//! `docs/research/RESEARCH-BACKLOG.md`, RB-RESEARCH-S005). `wire` holds the
//! serde types and pure conversion into `rb_domain::state`.
//!
//! The BakkesMod-side plugin that actually writes this format has not been
//! built yet — this sandbox has no Rocket League/BakkesMod/Windows
//! environment to build or run it in (the same practical blocker as
//! `RB-RESEARCH-O002`, see `RB-VERIFY-002`'s open questions). This crate is
//! tested against a synthetic, hand-authored fixture (see
//! `fixtures/README.md`), not a real capture.

mod wire;

use rb_domain::{IngestError, PhysicsFrame, PhysicsStateSource};
use std::io::BufRead;
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
        let file = std::fs::File::open(&self.path).map_err(|e| IngestError::Io(e.to_string()))?;
        let reader = std::io::BufReader::new(file);

        let mut frames = Vec::new();
        for (line_no, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| IngestError::Io(e.to_string()))?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let frame = wire::parse_line(line)
                .map_err(|e| IngestError::Malformed(format!("line {}: {e}", line_no + 1)))?;
            frames.push(frame);
        }

        Ok(frames)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reports_io_error_not_a_panic() {
        let source = CaptureFileSource::new("fixtures/does-not-exist.capture.jsonl");
        let result = source.frames();
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn malformed_line_reports_malformed_not_a_panic() {
        let dir = std::env::temp_dir().join("rb_capture_ingest_test_malformed.jsonl");
        std::fs::write(&dir, b"not a json line\n").unwrap();
        let source = CaptureFileSource::new(dir.clone());
        let result = source.frames();
        std::fs::remove_file(&dir).ok();
        assert!(matches!(result, Err(IngestError::Malformed(_))));
    }

    #[test]
    fn blank_lines_are_skipped_not_treated_as_malformed() {
        let dir = std::env::temp_dir().join("rb_capture_ingest_test_blank_lines.jsonl");
        let line = r#"{"timestamp_secs":0.0,"ball":{"position":{"x":0.0,"y":0.0,"z":93.0},"rotation":{"x":0.0,"y":0.0,"z":0.0,"w":1.0},"velocity":{"x":0.0,"y":0.0,"z":0.0},"angular_velocity":{"x":0.0,"y":0.0,"z":0.0}}}"#;
        std::fs::write(&dir, format!("\n{line}\n\n{line}\n")).unwrap();
        let source = CaptureFileSource::new(dir.clone());
        let frames = source.frames();
        std::fs::remove_file(&dir).ok();
        assert_eq!(frames.unwrap().len(), 2);
    }

    #[test]
    fn synthetic_fixture_parses_into_a_nonempty_frame_sequence_with_input() {
        let source = CaptureFileSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/example.capture.jsonl"
        ));
        let frames = source
            .frames()
            .expect("synthetic fixture should parse cleanly");

        assert!(!frames.is_empty());
        assert!(
            frames
                .windows(2)
                .all(|w| w[0].timestamp_secs <= w[1].timestamp_secs),
            "frames should be chronologically ordered"
        );

        let frames_with_input = frames
            .iter()
            .flat_map(|f| &f.cars)
            .filter(|c| c.input.is_some())
            .count();
        assert!(
            frames_with_input > 0,
            "unlike rb_replay_ingest, capture frames should always carry input"
        );
    }
}
