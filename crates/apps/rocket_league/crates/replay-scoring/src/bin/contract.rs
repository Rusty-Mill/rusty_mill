//! `contract` — run the canonical-vs-ballchasing cross-check on two JSON files
//! and print a tiered diff (spec §13, Phase 1). This is the thin CLI over
//! [`replay_scoring::contract::cross_check`]; the comparator logic and its CI
//! test live in the library.
//!
//! Usage:
//!   contract <our.canonical.json> <ballchasing.json>
//!
//! `<our.canonical.json>` is a `CanonicalMatch` (`replay-scoring
//! --dump-canonical`); `<ballchasing.json>` is a sanitized ballchasing replay
//! (`scripts/ballchasing_fetch.py`). Exits non-zero on **any Tier-1 mismatch**
//! (the exact-fact contract); Tier-2 advisories print but never fail. No network.

use std::error::Error;
use std::process::ExitCode;

use replay_analyzer::model::CanonicalMatch;
use replay_scoring::contract::{cross_check, BallchasingReplay, Tier};

const USAGE: &str = "usage: contract <our.canonical.json> <ballchasing.json>";

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

/// Returns `Ok(false)` on any Tier-1 mismatch (the non-zero-exit gate).
fn run() -> Result<bool, Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let our_path = args.next().ok_or(USAGE)?;
    let bc_path = args.next().ok_or(USAGE)?;
    if args.next().is_some() {
        return Err(USAGE.into());
    }

    let our: CanonicalMatch = serde_json::from_slice(&std::fs::read(&our_path)?)?;
    let bc: BallchasingReplay = serde_json::from_slice(&std::fs::read(&bc_path)?)?;

    if bc.status.as_deref().is_some_and(|s| s != "ok") {
        eprintln!(
            "warning: ballchasing status is {:?}, not \"ok\" — its decode may be incomplete",
            bc.status
        );
    }

    let rep = cross_check(&our, &bc);
    println!(
        "cross-check {} : {} player(s) matched, {} goal(s) checked",
        rep.replay_id, rep.matched_players, rep.goals_checked
    );

    if rep.tier2.is_empty() {
        println!("  Tier 2 (advisory): none");
    } else {
        println!("  Tier 2 (advisory — recomputed stats may legitimately differ):");
        for f in &rep.tier2 {
            debug_assert_eq!(f.tier, Tier::Two);
            println!("    ~ [{}] {}", f.field, f.detail);
        }
    }

    if rep.tier1_ok() {
        println!("  Tier 1 (exact): PASS — every authoritative fact agrees");
        Ok(true)
    } else {
        println!("  Tier 1 (exact): FAIL — {} mismatch(es):", rep.tier1.len());
        for f in &rep.tier1 {
            println!("    ✗ [{}] {}", f.field, f.detail);
        }
        Ok(false)
    }
}
