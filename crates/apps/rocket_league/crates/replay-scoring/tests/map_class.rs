//! Map classification → confidence: a non-standard arena must flag every report
//! low-confidence (positional metrics assume standard Soccar geometry).

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::{score_all, Confidence, ScoreConfig};
use std::path::PathBuf;

#[test]
fn non_standard_map_forces_low_confidence() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rleval/assets/replays/42f2.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read sample: {e}"));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let mut canonical = build_canonical(&decoded, "42f2");
    let cfg = ScoreConfig::default();

    // The real map (Mannfield) is standard, so confidence is data-driven there.
    assert!(canonical.map.is_some(), "sample carries a map name");

    // Pretend it was played on a non-standard arena: every report goes low-conf.
    canonical.map = Some("HoopsStadium_P".into());
    let reports = score_all(&canonical, &cfg);
    assert!(!reports.is_empty());
    for r in reports {
        assert!(
            matches!(r.confidence, Confidence::LowConfidence),
            "{} should be low-confidence on a non-standard map",
            r.target_player
        );
    }
}
