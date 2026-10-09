//! Offline smoke test: decode a committed sample `.replay` through the unified
//! pipeline and assert every view is populated. No network, mirrors how the
//! other crates exercise the bundled `42f2`/`419a` fixtures in CI.

use std::path::PathBuf;

fn sample(name: &str) -> Vec<u8> {
    // CARGO_MANIFEST_DIR is `app/`; the samples live at the workspace root.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("../rleval/assets/replays")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

use replay_scoring::XgModel;
use rleval_app::pipeline;

#[test]
fn analyzes_a_sample_into_every_view() {
    let bytes = sample("42f2.replay");
    // No norms ⇒ purely absolute scoring (the rank-relative layer is optional).
    let a = pipeline::analyze(&bytes, "42f2", None, None, &XgModel::default())
        .expect("pipeline should succeed");

    // Match summary is populated.
    assert_eq!(a.replay_id, "42f2");
    assert!(a.map.is_some());
    assert!(a.duration_s > 0.0);
    assert!(a.standard_map, "EuroStadium is a standard Soccar arena");
    assert!(!a.team_scores.is_empty());
    assert!(
        a.coordinate_warnings.is_empty(),
        "a healthy sample raises no coordinate warning: {:?}",
        a.coordinate_warnings
    );

    // Every engine produced rows.
    assert!(!a.scores.is_empty(), "scoring produced reports");
    assert!(!a.skill_profiles.is_empty(), "skills produced profiles");
    assert!(!a.impact.players.is_empty(), "value produced impact rows");
    assert!(!a.pacifist.players.is_empty(), "pacifist produced rows");
    assert!(!a.episodes.is_empty(), "scoring produced recovery episodes");
    assert!(
        a.pacifist.players.iter().all(|p| p.dimensions.len() == 8),
        "every player carries the full rubric"
    );
    assert!(
        a.pacifist
            .players
            .iter()
            .all(|p| p.verdict == "PASS" || p.verdict == "FAIL"),
        "every player gets an FM-1 verdict"
    );

    // The two embeddable HTML views are self-contained documents.
    assert!(a.viewer_html.contains("<html"), "viewer is a full document");
    assert!(
        a.viewer_html.len() > 100_000,
        "offline viewer embeds three.js + scene"
    );
    assert!(
        a.scoring_html.contains("<html"),
        "scoring is a full document"
    );

    // The pieces line up: a scored player has a value-impact row.
    let scored_pri = a.scores[0].target_pri;
    assert!(
        a.impact.players.iter().any(|p| p.pri == scored_pri),
        "scored players appear in the impact table"
    );
}

#[test]
fn rejects_garbage_bytes() {
    let err = pipeline::analyze(b"not a replay", "junk", None, None, &XgModel::default());
    assert!(err.is_err(), "a non-replay should not parse");
}
