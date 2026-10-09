//! Replay-file ingestion adapter — see `RB-VERIFY-001`.
//!
//! This crate is the `boxcars` + `subtr-actor`-backed implementation of
//! `rb_domain::PhysicsStateSource`. `boxcars` parses the raw replay/network
//! stream; `subtr_actor::ReplayDataCollector` resolves that into
//! frame-indexed ball/player rigid-body state (actor-graph tracking that
//! would otherwise have to be reimplemented by hand — see `Cargo.toml`'s
//! dependency comment). `convert` maps that into `rb_domain::PhysicsFrame`.

mod convert;

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
        let bytes = std::fs::read(&self.path).map_err(|e| IngestError::Io(e.to_string()))?;

        let replay = boxcars::ParserBuilder::new(&bytes)
            .must_parse_network_data()
            .on_error_check_crc()
            .parse()
            .map_err(|e| IngestError::Malformed(e.to_string()))?;

        // SubtrActorError itself only derives Debug (it wraps a Display-able
        // variant plus a backtrace) — the variant is the human-readable part.
        let replay_data = subtr_actor::ReplayDataCollector::new()
            .get_replay_data(&replay)
            .map_err(|e| IngestError::Malformed(e.variant.to_string()))?;

        Ok(convert::to_physics_frames(&replay_data))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reports_io_error_not_a_panic() {
        let source = ReplayFileSource::new("fixtures/does-not-exist.replay");
        let result = source.frames();
        assert!(matches!(result, Err(IngestError::Io(_))));
    }

    #[test]
    fn malformed_file_reports_malformed_not_a_panic() {
        let dir = std::env::temp_dir().join("rb_replay_ingest_test_malformed");
        std::fs::write(&dir, b"not a replay file").unwrap();
        let source = ReplayFileSource::new(dir.clone());
        let result = source.frames();
        std::fs::remove_file(&dir).ok();
        assert!(matches!(result, Err(IngestError::Malformed(_))));
    }

    #[test]
    fn real_fixture_replay_parses_into_a_nonempty_frame_sequence() {
        let source = ReplayFileSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/subtr-actor-sample.replay"
        ));
        let frames = source
            .frames()
            .expect("fixture replay should parse cleanly");

        assert!(
            !frames.is_empty(),
            "expected at least one frame with ball data"
        );

        // Soccar's field is roughly 8200x10300x2000 (uu) with a comfortable
        // margin around it; every frame's ball position should be within
        // that, which would catch a gross unit/axis conversion bug without
        // pinning to a specific manually-verified value (that verification
        // still needs the owner's own replay — see fixtures/README.md).
        for frame in &frames {
            let p = frame.ball.position;
            assert!(
                p.x.abs() < 6000.0,
                "ball x out of plausible field bounds: {p:?}"
            );
            assert!(
                p.y.abs() < 12000.0,
                "ball y out of plausible field bounds: {p:?}"
            );
            assert!(
                (-500.0..3000.0).contains(&p.z),
                "ball z out of plausible field bounds: {p:?}"
            );
        }

        let frames_with_cars = frames.iter().filter(|f| !f.cars.is_empty()).count();
        assert!(
            frames_with_cars > 0,
            "expected at least some frames with car data"
        );
    }
}
