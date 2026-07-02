//! End-to-end: real replay → canonical model → bridge → Pacifist scores.
//!
//! Guards the bridge against drift in the canonical model and proves the ported
//! domain scores real, full-match data (the ported unit tests cover semantics
//! on synthetic timelines; this covers the seam).

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_pacifist::bridge::timeline_from_canonical;
use replay_pacifist::context::MatchContext;
use replay_pacifist::scoring::Analyzer;
use replay_pacifist::{roster, Timeline};
use std::path::PathBuf;

fn timeline(name: &str) -> Timeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/replays")
        .join(name);
    let data = std::fs::read(&path).expect("read sample replay");
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, name.trim_end_matches(".replay"));
    timeline_from_canonical(&canonical)
}

#[test]
fn bridged_timeline_is_sane_and_every_player_scores() {
    let tl = timeline("42f2.replay");
    assert!(!tl.is_empty(), "grid frames present");
    assert!(
        tl.windows(2).all(|w| w[0].t <= w[1].t),
        "frame times monotone"
    );
    assert!(
        tl.iter().flat_map(|f| &f.players).all(|p| p.boost <= 100),
        "boost rescaled to 0..=100"
    );

    let players = roster(&tl);
    assert!(!players.is_empty(), "roster resolves");
    assert!(
        players.iter().all(|p| p.name.is_some()),
        "names bridged from canonical tracks"
    );

    let analyzer = Analyzer::default();
    let ctx = MatchContext::derive(&tl, analyzer.context_config());
    for entry in &players {
        let score = analyzer.score_player_in(&ctx, entry.player);
        let value = score
            .value
            .unwrap_or_else(|| panic!("{:?} has scoreable evidence", entry.name));
        assert!((0.0..=100.0).contains(&value.get()));
        assert!(score.confidence.get() > 0.0, "real match backs the score");
        assert_eq!(score.breakdown.len(), 8, "the full rubric reported");
    }
}
