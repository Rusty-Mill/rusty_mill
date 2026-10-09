//! `bc-validate <file.replay> <ballchasing.json>` — the **exact-match validator**.
//!
//! Decodes the replay through our clone, then diffs every field of the resulting
//! ballchasing-shaped document against a real `GET /replays/{id}` document
//! (captured by `scripts/ballchasing_fetch.py --full-stats`). Prints a per-field
//! agreement table and exits non-zero if any gated (exact / core) field fails —
//! turning ballchasing into a precise regression oracle for the clone.

use bc_clone::{ballchasing_document, compare::compare_documents};
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use serde_json::Value;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: bc-validate <file.replay> <ballchasing.json>";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let replay = args.next().ok_or(USAGE)?;
    let truth_path = args.next().ok_or(USAGE)?;

    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let id = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let ours = serde_json::to_value(ballchasing_document(&build_canonical(&decoded, id)))?;
    let theirs: Value = serde_json::from_slice(&std::fs::read(&truth_path)?)?;

    let report = compare_documents(&ours, &theirs);
    eprintln!(
        "paired {}/{} players (ours {}, theirs {})",
        report.players_paired,
        report.players_ours.max(report.players_theirs),
        report.players_ours,
        report.players_theirs
    );
    eprintln!(
        "{:<10} {:<34} {:>3} {:>9} {:>11}",
        "group", "field", "n", "med_rel", "max_abs"
    );
    let mut diffs = report.diffs.clone();
    diffs.sort_by(|a, b| {
        b.median_rel_err
            .total_cmp(&a.median_rel_err)
            .then(a.field.cmp(&b.field))
    });
    for d in &diffs {
        eprintln!(
            "{:<10} {:<34} {:>3} {:>9.3} {:>11.2}{}",
            d.group,
            d.field,
            d.n,
            d.median_rel_err,
            d.max_abs_err,
            if d.exact { "  [exact]" } else { "" },
        );
    }

    let fails = report.failures();
    if fails.is_empty() {
        eprintln!("\nPASS: every gated field agrees");
        Ok(true)
    } else {
        eprintln!("\nFAIL ({} gated mismatches):", fails.len());
        for f in &fails {
            eprintln!("  - {f}");
        }
        Ok(false)
    }
}
