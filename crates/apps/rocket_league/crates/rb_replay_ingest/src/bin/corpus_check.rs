//! Optional local corpus health-check for `rb_replay_ingest`.
//!
//! Runs the real parse pipeline (`boxcars` + `subtr-actor` + `convert`)
//! against every `.replay` file in a local directory (default `replays/`
//! at the workspace root — already `.gitignore`d, since real match
//! replays are the owner's personal data and never belong in this repo).
//! This is a local/with-corpus gate, not a CI check: a checkout with no
//! `replays/` directory is a deliberate no-op, matching the same pattern
//! `RLEvalSystem`'s `batch-recon-check` uses for its own gitignored
//! corpus. See `RB-VERIFY-001`'s Verification plan.
//!
//! Usage: `cargo run -p rb_replay_ingest --bin corpus_check [dir]`

use rb_domain::PhysicsStateSource;
use rb_replay_ingest::ReplayFileSource;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("replays"));

    if !dir.is_dir() {
        println!(
            "no corpus directory at {} — nothing to check (this is expected on a fresh checkout)",
            dir.display()
        );
        return ExitCode::SUCCESS;
    }

    let mut replay_paths: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("replay"))
            })
            .collect(),
        Err(e) => {
            eprintln!("failed to read {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    replay_paths.sort();

    if replay_paths.is_empty() {
        println!("{} has no .replay files — nothing to check", dir.display());
        return ExitCode::SUCCESS;
    }

    let mut failures = Vec::new();
    for path in &replay_paths {
        match check_one(path) {
            Ok(summary) => println!("ok    {summary}"),
            Err(e) => {
                println!("FAIL  {}: {e}", file_name(path));
                failures.push(path.clone());
            }
        }
    }

    println!(
        "\n{} / {} replays parsed cleanly",
        replay_paths.len() - failures.len(),
        replay_paths.len()
    );
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        for f in &failures {
            eprintln!("  offender: {}", f.display());
        }
        ExitCode::FAILURE
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn check_one(path: &Path) -> Result<String, String> {
    let source = ReplayFileSource::new(path);
    let frames = source.frames().map_err(|e| e.to_string())?;

    let Some(last) = frames.last() else {
        return Err("parsed but produced zero frames".to_string());
    };

    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;
    let mut max_cars = 0usize;
    for frame in &frames {
        min_z = min_z.min(frame.ball.position.z);
        max_z = max_z.max(frame.ball.position.z);
        max_cars = max_cars.max(frame.cars.len());
    }

    Ok(format!(
        "{}: {} frames, {:.1}s, up to {} cars, ball z [{:.0}, {:.0}]",
        file_name(path),
        frames.len(),
        last.timestamp_secs,
        max_cars,
        min_z,
        max_z
    ))
}
