//! `rb-verify --self --write-candidate` (ADR-0085): the saved candidate reads
//! back through the capture adapter and scores exactly as the in-memory run,
//! and it never writes over the capture it was simulated from.
#![allow(clippy::expect_used)]

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

/// Each alias of the capture is refused, and the capture is left byte for byte.
fn assert_refused_and_unchanged(capture: &std::path::Path, out: &std::path::Path) {
    let before = std::fs::read(capture).expect("read capture");
    let result = write_candidate(capture, SeedFrame::default(), out);
    assert!(result.is_err(), "{} was accepted", out.display());
    assert_eq!(std::fs::read(capture).expect("reread capture"), before);
}

fn scratch_capture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rb_verify_cli_alias_{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let capture = dir.join("capture.jsonl");
    std::fs::copy(fixture(), &capture).expect("copy fixture");
    capture
}

#[test]
fn writing_over_the_capture_itself_is_refused() {
    let capture = scratch_capture("same");
    assert_refused_and_unchanged(&capture, &capture);
}

#[cfg(unix)]
#[test]
fn writing_through_a_symlink_or_hard_link_to_the_capture_is_refused() {
    let capture = scratch_capture("links");
    let dir = capture.parent().expect("dir").to_path_buf();
    let symlink = dir.join("symlink.jsonl");
    std::os::unix::fs::symlink(&capture, &symlink).expect("symlink");
    assert_refused_and_unchanged(&capture, &symlink);
    let hard = dir.join("hard.jsonl");
    std::fs::hard_link(&capture, &hard).expect("hard link");
    assert_refused_and_unchanged(&capture, &hard);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_bad_argument_fails_before_any_file_is_written() {
    let capture = scratch_capture("bad_arg");
    let out = capture.with_file_name("candidate.jsonl");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_rb-verify"))
        .arg("--self")
        .arg(&capture)
        .arg("not-a-number")
        .arg("--write-candidate")
        .arg(&out)
        .output()
        .expect("run rb-verify")
        .status;
    assert!(!status.success());
    assert!(
        !out.exists(),
        "a candidate was written despite the bad argument"
    );
}
