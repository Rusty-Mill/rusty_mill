//! `rb-verify`: Phase 0 verification pipeline entry point.
//!
//! Composition root only — wires `rb_replay_ingest`/`rb_capture_ingest`
//! (adapters) to `rb_domain::divergence::score` (domain logic). Both
//! adapters are stubs until Phase 0 delivery work lands their real parsing
//! backends, so this currently reports that rather than pretending to
//! produce a score. See `docs/roadmap/ROADMAP.md` Phase 0 exit criteria.

use rb_capture_ingest::CaptureFileSource;
use rb_domain::PhysicsStateSource;
use rb_replay_ingest::ReplayFileSource;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let (Some(replay_path), Some(capture_path)) = (args.next(), args.next()) else {
        eprintln!("usage: rb-verify <replay-file> <capture-file>");
        return ExitCode::FAILURE;
    };

    let replay = ReplayFileSource::new(replay_path);
    let capture = CaptureFileSource::new(capture_path);

    match (replay.frames(), capture.frames()) {
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("ingestion not ready yet: {e}");
            ExitCode::FAILURE
        }
        (Ok(recorded), Ok(candidate)) => {
            let result = rb_domain::divergence::score(&recorded, &candidate);
            println!("{result:?}");
            ExitCode::SUCCESS
        }
    }
}
