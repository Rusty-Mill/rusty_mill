//! CI contract test for the canonical-vs-ballchasing cross-check (spec §13,
//! Phase 1). Runs entirely **offline** against checked-in fixtures — no network,
//! no ballchasing key — so it's safe in CI. Capturing/refreshing the fixtures is
//! the manual job of `scripts/ballchasing_fetch.py` (key + ToS gated).
//!
//! The pair is a real ranked 2v2 (ballchasing id `990e4485-…`, on Utopia): our
//! boxcars decode (`990e4485.canonical.json`, a header-only slice) vs
//! ballchasing's independent decode (`990e4485.ballchasing.json`, sanitized —
//! uploader + player ids stripped). The original `42f2`/`419a` samples can't be
//! used here because uploading them to ballchasing was blocked by the
//! environment's egress request-body cap; a corpus replay already on ballchasing
//! gives a real pair via GET. Regenerate with:
//!   python assets/corpus/refresh_corpus_replays.py --ids 990e4485-71c0-41a4-bc9a-ef3923871906
//!   replay-scoring <id>.replay --dump-canonical -   # then slice to header fields
//!   python scripts/ballchasing_fetch.py <id>.replay --id <id> --out <…>.ballchasing.json

use std::path::{Path, PathBuf};

use replay_analyzer::model::CanonicalMatch;
use replay_scoring::contract::{cross_check, BallchasingReplay};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_canonical(name: &str) -> CanonicalMatch {
    let p = fixtures().join(name);
    serde_json::from_slice(
        &std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())),
    )
    .unwrap_or_else(|e| panic!("parse {}: {e}", p.display()))
}

fn load_ballchasing(name: &str) -> BallchasingReplay {
    let p = fixtures().join(name);
    serde_json::from_slice(
        &std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())),
    )
    .unwrap_or_else(|e| panic!("parse {}: {e}", p.display()))
}

/// On the real pair, every Tier-1 (exact) fact must agree — the redundancy
/// guarantee. (This replay also happens to agree on Tier-2 stats, but those are
/// advisory and not asserted.)
#[test]
fn real_pair_passes_tier1_exactly() {
    let our = load_canonical("990e4485.canonical.json");
    let bc = load_ballchasing("990e4485.ballchasing.json");
    let rep = cross_check(&our, &bc);

    assert!(
        rep.tier1_ok(),
        "real canonical-vs-ballchasing pair must pass Tier 1, got mismatches: {:#?}",
        rep.tier1
    );
    // Sanity: we actually compared a full 2v2 roster + all goals, not nothing.
    assert_eq!(rep.matched_players, 4, "expected a full 2v2 roster matched");
    assert_eq!(rep.goals_checked, 10, "expected all 10 goals cross-checked");
}

/// A deliberately drifted ballchasing fixture (one goal mis-attributed across
/// teams — total count unchanged, but per-team score + per-scorer attribution
/// diverge) must be caught as a Tier-1 failure. This is the §11 "parser drift"
/// tripwire: a silent header regression a naive goal-count check would miss.
#[test]
fn drifted_pair_is_caught_as_tier1_failure() {
    let our = load_canonical("990e4485.canonical.json");
    let drift = load_ballchasing("990e4485.ballchasing.drift.json");
    let rep = cross_check(&our, &drift);

    assert!(
        !rep.tier1_ok(),
        "the drifted fixture must trip a Tier-1 mismatch, but the cross-check passed"
    );
    // The drift moved a goal between teams, so per-team score must be among the
    // findings (and the total goal count must NOT be, proving the breakdown
    // check is doing the work).
    assert!(
        rep.tier1.iter().any(|f| f.field == "team_score"),
        "expected a team_score mismatch among Tier-1 findings: {:#?}",
        rep.tier1
    );
    assert!(
        !rep.tier1.iter().any(|f| f.field == "goal_count"),
        "total goal count was unchanged by this drift; the breakdown should catch it"
    );
}
