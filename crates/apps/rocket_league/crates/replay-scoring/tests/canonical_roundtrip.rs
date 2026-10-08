//! The re-score path (`--from-canonical`) must score identically to a freshly
//! parsed replay: `CanonicalMatch` serde-round-trips losslessly and the scoring
//! core is a pure function of it, so caching the canonical skips the parse with
//! no change in output (service §9).

use std::path::PathBuf;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::model::CanonicalMatch;
use replay_scoring::{score_all, ScoreConfig};

#[test]
fn scoring_a_roundtripped_canonical_is_identical() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/replays/42f2.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, "42f2");

    // Round-trip through JSON exactly as `--dump-canonical` / `--from-canonical` do.
    let json = serde_json::to_vec(&canonical).expect("serialize");
    let restored: CanonicalMatch = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(
        canonical, restored,
        "canonical must serde round-trip losslessly"
    );

    let cfg = ScoreConfig::default();
    assert_eq!(
        score_all(&canonical, &cfg),
        score_all(&restored, &cfg),
        "scoring a round-tripped canonical must match scoring the original",
    );
}
