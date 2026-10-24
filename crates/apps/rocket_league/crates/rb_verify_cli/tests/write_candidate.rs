//! `rb-verify --self --write-candidate` (ADR-0085): the saved candidate reads
//! back through the capture adapter and scores exactly as the in-memory run.

use std::error::Error;
use std::path::PathBuf;

use rb_capture_ingest::CaptureFileSource;
use rb_domain::PhysicsStateSource;
use rb_verify_cli::{
    score_capture_against_candidate, write_candidate, SeedFrame, DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rb_capture_ingest/fixtures/human/clean-dodge.capture.jsonl")
}

#[test]
fn saved_candidate_scores_like_the_in_memory_run() -> Result<(), Box<dyn Error>> {
    let out = std::env::temp_dir().join("rb_verify_cli_write_candidate.jsonl");
    let written = write_candidate(fixture(), SeedFrame::default(), &out)?;
    let candidate = CaptureFileSource::new(&out).frames();
    std::fs::remove_file(&out).ok();
    let candidate = candidate?;
    assert_eq!(candidate.len(), written);
    assert!(candidate
        .iter()
        .all(|f| f.cars.iter().all(|c| c.input.is_some())));

    let recorded = CaptureFileSource::new(fixture()).frames()?;
    let seed_t = candidate.first().ok_or("empty candidate")?.timestamp_secs;
    let recorded: Vec<_> = recorded
        .into_iter()
        .filter(|f| f.timestamp_secs >= seed_t)
        .collect();
    let from_file =
        rb_domain::divergence::score(&recorded, &candidate, DEFAULT_MAX_TIMESTAMP_DELTA_SECS);
    let in_memory = score_capture_against_candidate(
        fixture(),
        SeedFrame::default(),
        DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
    )?;
    assert_eq!(from_file, in_memory);
    Ok(())
}
