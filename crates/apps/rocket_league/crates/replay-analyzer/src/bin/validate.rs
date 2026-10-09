//! Offline external-validation harness.
//!
//! Loads the committed ballchasing ground-truth fixture, runs the analyzer over
//! the corpus replays, and reports per-channel agreement (Spearman/Pearson +
//! median relative error) between our reconstruction and ballchasing's. With
//! `--gate` it exits non-zero if agreement falls below the locked tolerances
//! ([`replay_analyzer::analyze::validate`]) — the regression gate that keeps the
//! reconstruction honest.
//!
//! ```text
//! usage: validate [--fixture <stats.json>] [--corpus <dir>] [--limit N] [--gate]
//! ```
//!
//! The corpus `.replay` files are large and gitignored; run this where they are
//! present (the fixture and this harness are committed, so the *method* is
//! reproducible without them).

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::validate::{evaluate, pair_replay, Agreement, GroundTruth, Samples};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::model::CanonicalMatch;

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

fn run() -> Result<bool, Box<dyn Error>> {
    let mut fixture = PathBuf::from("assets/corpus/ballchasing_stats.json");
    let mut corpus = PathBuf::from("assets/corpus");
    let mut limit: Option<usize> = None;
    let mut gate = false;
    let mut unmatched = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--fixture" => fixture = it.next().ok_or("--fixture needs a path")?.into(),
            "--corpus" => corpus = it.next().ok_or("--corpus needs a path")?.into(),
            "--limit" => limit = Some(it.next().ok_or("--limit needs N")?.parse()?),
            "--gate" => gate = true,
            "--unmatched" => unmatched = true,
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let gt: GroundTruth = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    eprintln!(
        "fixture: {} ({} replays, source={})",
        fixture.display(),
        gt.replays.len(),
        gt.source
    );

    let mut ids: Vec<&String> = gt.replays.keys().collect();
    ids.sort();
    if let Some(n) = limit {
        ids.truncate(n);
    }

    let mut acc = Samples::default();
    let (mut matched, mut total) = (0usize, 0usize);
    let (mut present, mut missing) = (0usize, 0usize);

    for id in ids {
        let path = corpus.join(format!("{id}.replay"));
        if !path.exists() {
            missing += 1;
            continue;
        }
        present += 1;
        let entry = &gt.replays[id];
        let canonical = match analyze_one(&path, id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  skip {id}: {e}");
                continue;
            }
        };
        let dur = entry.duration_s.unwrap_or(canonical.duration_s);
        let mc = pair_replay(&canonical.features, entry, dur, &mut acc);
        matched += mc.matched;
        total += mc.total;
        if unmatched && mc.matched < mc.total {
            let ours: Vec<(String, Option<i32>)> = canonical
                .features
                .iter()
                .map(|f| (f.player.clone(), f.team))
                .collect();
            let gt_names: Vec<(&str, i32)> = entry
                .players
                .iter()
                .map(|p| (p.name.as_str(), p.team))
                .collect();
            eprintln!("  {id}: matched {}/{}", mc.matched, mc.total);
            eprintln!("    ballchasing: {gt_names:?}");
            eprintln!("    analyzer   : {ours:?}");
        }
    }

    let report = evaluate(&acc, matched, total);
    eprintln!(
        "\nreplays: {present} present, {missing} missing (no local .replay)\n\
         players: {}/{} matched by name (coverage {:.3})\n",
        report.matched, report.total, report.coverage
    );

    println!(
        "{:<22} {:>5} {:>9} {:>9} {:>11}",
        "channel", "n", "spearman", "pearson", "med_rel_err"
    );
    print_row("time_supersonic_s", &report.supersonic);
    print_row("mean_dist_to_ball", &report.dist_to_ball);
    print_row("boost_used~bcpm", &report.boost_bcpm);
    print_row("boost_used~bpm", &report.boost_bpm);

    if !gate {
        return Ok(true);
    }

    let fails = report.failures();
    if fails.is_empty() {
        eprintln!(
            "\nGATE PASS: reconstruction agrees with ballchasing within tolerance \
             (boost mapping: {}).",
            report.boost().0
        );
        Ok(true)
    } else {
        eprintln!("\nGATE FAIL:");
        for f in &fails {
            eprintln!("  - {f}");
        }
        Ok(false)
    }
}

fn analyze_one(path: &Path, id: &str) -> Result<CanonicalMatch, Box<dyn Error>> {
    let data = std::fs::read(path)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    Ok(build_canonical(&decoded, id.to_string()))
}

fn print_row(name: &str, a: &Agreement) {
    let f = |o: Option<f32>| {
        o.map(|v| format!("{v:.4}"))
            .unwrap_or_else(|| "  n/a".into())
    };
    println!(
        "{:<22} {:>5} {:>9} {:>9} {:>11.4}",
        name,
        a.n,
        f(a.spearman),
        f(a.pearson),
        a.median_rel_err
    );
}
