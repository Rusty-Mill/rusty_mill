//! `recon-check` — cross-check our reconstruction against subtr-actor's
//! independent one for a single `.replay`, and print a tiered diff (spec §13,
//! Phase 2). Exits non-zero on any Tier-R1 breach. Fully offline.
//!
//! Usage:
//!   recon-check <file.replay>

use std::process::ExitCode;

use recon_check::cross_check_replay;

const USAGE: &str = "usage: recon-check <file.replay>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let id = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay");

    let rep = match cross_check_replay(&bytes, id) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "recon-check {} @ {:.0} Hz: our {} frames / subtr {} frames / {} aligned ({:.1}% coverage)",
        rep.replay_id,
        rep.hz,
        rep.our_frames,
        rep.subtr_frames,
        rep.matched_frames,
        rep.coverage * 100.0
    );
    println!(
        "  ball Δ: median {:.1} uu, p95 {:.1} uu, max {:.1} uu | agree {:.1}% (≤{:.0} uu)",
        rep.ball_median_uu,
        rep.ball_p95_uu,
        rep.ball_max_uu,
        rep.ball_agree_rate * 100.0,
        recon_check::BALL_AGREE_TOL_UU,
    );
    println!("  per-player car Δ:");
    for s in &rep.per_player {
        println!(
            "    {:<24} median {:>6.1} uu  p95 {:>7.1} uu  ({} frames)",
            s.player, s.car_median_uu, s.car_p95_uu, s.frames
        );
    }

    if rep.tier2.is_empty() {
        println!("  Tier R2 (advisory): none");
    } else {
        println!("  Tier R2 (advisory):");
        for f in &rep.tier2 {
            println!("    ~ [{}] {}", f.field, f.detail);
        }
    }

    if rep.tier1_ok() {
        println!("  Tier R1 (gated): PASS — reconstructions agree within tolerance");
        ExitCode::SUCCESS
    } else {
        println!("  Tier R1 (gated): FAIL — {} breach(es):", rep.tier1.len());
        for f in &rep.tier1 {
            println!("    ✗ [{}] {}", f.field, f.detail);
        }
        ExitCode::FAILURE
    }
}
