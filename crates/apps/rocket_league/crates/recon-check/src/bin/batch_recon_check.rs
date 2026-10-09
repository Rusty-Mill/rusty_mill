//! `batch-recon-check` — run the Phase-2 reconstruction cross-check across the
//! whole corpus and aggregate, so recon-check earns its keep as a **bug-finder**:
//! either confirm our reconstruction agrees with an independent decoder
//! everywhere, or surface the specific replays where it doesn't (= real
//! reconstruction bugs to investigate — own-goal/celebration windows, odd maps).
//!
//! This is a **local/with-corpus gate** (like `reconcile`), not a no-corpus CI
//! check: the 180 corpus `.replay` files are gitignored — fetch them by manifest
//! id with `assets/corpus/refresh_corpus_replays.py` (needs `BALLCHASING_API_KEY`).
//! Missing files are reported and skipped, so it runs (as a no-op) without them.
//!
//! Usage:
//!   batch-recon-check [--manifest <path>] [--dir <path>] [--limit N] [--gate]
//!
//! Run in **`--release`** — subtr-actor is slow in debug.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use recon_check::{cross_check_replay, ReconReport, BALL_AGREE_TOL_UU};
use serde_json::Value;

const USAGE: &str =
    "usage: batch-recon-check [--manifest <path>] [--dir <path>] [--limit N] [--gate]";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut manifest = PathBuf::from("assets/corpus/manifest.json");
    let mut dir: Option<PathBuf> = None;
    let mut limit: Option<usize> = None;
    let mut gate = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--manifest" => manifest = it.next().ok_or("--manifest needs a path")?.into(),
            "--dir" => dir = Some(it.next().ok_or("--dir needs a path")?.into()),
            "--limit" => {
                limit = Some(
                    it.next()
                        .ok_or("--limit needs a number")?
                        .parse()
                        .map_err(|_| "--limit must be a number")?,
                )
            }
            "--gate" => gate = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(format!("unexpected arg {other}\n{USAGE}").into()),
        }
    }

    // Replays sit next to the manifest unless --dir overrides (the refresh script
    // lands them in assets/corpus/ by default).
    let dir = dir.unwrap_or_else(|| {
        manifest
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });

    let raw = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("read manifest {}: {e}", manifest.display()))?;
    let entries: Vec<Value> = serde_json::from_str(&raw)?;
    let total = entries.len();

    let mut reports: Vec<ReconReport> = Vec::new();
    let mut missing = 0usize;
    let mut errors = 0usize;
    let mut considered = 0usize;

    for e in &entries {
        if limit.is_some_and(|n| considered >= n) {
            break;
        }
        considered += 1;
        let id = e.get("id").and_then(Value::as_str).unwrap_or("?");
        let file = match e.get("file").and_then(Value::as_str) {
            Some(f) => f,
            None => {
                eprintln!("skip {id}: manifest entry has no `file`");
                errors += 1;
                continue;
            }
        };
        let path = dir.join(file);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                missing += 1;
                continue;
            }
        };
        match cross_check_replay(&bytes, id) {
            Ok(r) => {
                let mark = if r.tier1_ok() { "ok  " } else { "FAIL" };
                println!(
                    "  {mark} {id}  ball median {:>6.1} uu  agree {:>5.1}%  cov {:>5.1}%",
                    r.ball_median_uu,
                    r.ball_agree_rate * 100.0,
                    r.coverage * 100.0
                );
                reports.push(r);
            }
            Err(err) => {
                eprintln!("  ERR  {id}: {err}");
                errors += 1;
            }
        }
    }

    let ran = reports.len();
    let fails: Vec<&ReconReport> = reports.iter().filter(|r| !r.tier1_ok()).collect();
    let passed = ran - fails.len();

    println!(
        "\nbatch recon-check: {total} in manifest | considered {considered}, ran {ran}, \
         missing {missing}, errors {errors}"
    );
    if ran > 0 {
        let (mn, md, mx, mean) = stats(reports.iter().map(|r| r.ball_median_uu).collect());
        println!("  R1: PASS {passed} / FAIL {}", fails.len());
        println!("  ball median uu  : min {mn:.1} / med {md:.1} / max {mx:.1} / mean {mean:.1}");
        let (amn, amd, amx, amean) =
            stats(reports.iter().map(|r| r.ball_agree_rate * 100.0).collect());
        println!(
            "  agree % (≤{:.0} uu): min {amn:.1} / med {amd:.1} / max {amx:.1} / mean {amean:.1}",
            BALL_AGREE_TOL_UU
        );
        if fails.is_empty() {
            println!("  offenders: none — all reconstructions agree within tolerance ✔");
        } else {
            println!("  offenders (R1 fail) — investigate, don't loosen thresholds:");
            for r in &fails {
                let why = r
                    .tier1
                    .iter()
                    .map(|f| format!("[{}] {}", f.field, f.detail))
                    .collect::<Vec<_>>()
                    .join("; ");
                println!(
                    "    {} ball median {:.1} uu agree {:.1}% — {why}",
                    r.replay_id,
                    r.ball_median_uu,
                    r.ball_agree_rate * 100.0
                );
            }
        }
    } else {
        println!(
            "  no replays found under {} — fetch with assets/corpus/refresh_corpus_replays.py",
            dir.display()
        );
    }

    // Gate only on replays that actually ran: a corpus-less checkout can't fail it.
    if gate && !fails.is_empty() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// (min, median, max, mean) of a sample; zeros if empty.
fn stats(mut v: Vec<f32>) -> (f32, f32, f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let mean = v.iter().sum::<f32>() / n as f32;
    (v[0], v[n / 2], v[n - 1], mean)
}
