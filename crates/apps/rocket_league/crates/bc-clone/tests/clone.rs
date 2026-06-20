//! Tests for the ballchasing clone + the exact-match validator.
//!
//! * An integration test decodes the committed sample replay and asserts the
//!   emitted document has ballchasing's shape (sides, nested stat groups, every
//!   field present and populated).
//! * Offline unit tests exercise the validator's diff/gate logic on synthetic
//!   documents — no replay needed.

use bc_clone::compare::{aggregate_side, compare_documents};
use bc_clone::{ballchasing_document, Player, Stats};
use serde_json::{json, Value};
use std::path::PathBuf;

fn sample_doc() -> Value {
    use replay_analyzer::analyze::build_canonical;
    use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
    use replay_analyzer::decode::ReplayParser;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/replays/419a.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let doc = ballchasing_document(&build_canonical(&decoded, "419a"));
    serde_json::to_value(doc).expect("serialize doc")
}

#[test]
fn document_has_ballchasing_shape() {
    let d = sample_doc();
    // Top-level + sides.
    for k in [
        "id",
        "status",
        "map_name",
        "team_size",
        "duration",
        "blue",
        "orange",
    ] {
        assert!(d.get(k).is_some(), "missing top-level field {k}");
    }
    assert_eq!(d["status"], "ok");

    let blue = &d["blue"]["players"];
    assert_eq!(blue.as_array().unwrap().len(), 3, "419a is a 3v3");

    // Every player carries the five nested stat groups, each non-empty.
    let p = &blue[0]["stats"];
    for g in ["core", "boost", "movement", "positioning", "demo"] {
        assert!(
            p[g].as_object().map(|o| !o.is_empty()).unwrap_or(false),
            "group {g} missing/empty"
        );
    }
    // The authoritative additions are populated and sane.
    assert!(p["movement"]["count_powerslide"].as_u64().unwrap() > 0);
    assert!(p["boost"]["bpm"].as_f64().unwrap() > 0.0);
    assert!(p["boost"]["count_collected_big"].as_u64().is_some());
    // Field counts match the schema (28 boost, 27 positioning channels).
    assert_eq!(p["boost"].as_object().unwrap().len(), 28);
    assert_eq!(p["positioning"].as_object().unwrap().len(), 27);
}

#[test]
fn team_aggregate_sums_and_means_correctly() {
    // Two players: extensive fields sum, intensive (percent_/avg_/bpm) average.
    let mk = |goals: i32, bpm: f32, count_big: u32, pct_super: f32| -> Player {
        let mut s = Stats::default();
        s.core.goals = goals;
        s.core.shots = goals; // 1 shot per goal here
        s.boost.bpm = bpm;
        s.boost.count_collected_big = count_big;
        s.movement.percent_supersonic_speed = pct_super;
        Player {
            name: "x".into(),
            stats: s,
        }
    };
    let team = aggregate_side(&[mk(2, 100.0, 3, 10.0), mk(1, 300.0, 5, 20.0)]);
    assert_eq!(team.core.goals, 3, "goals sum");
    assert_eq!(team.boost.count_collected_big, 8, "counts sum");
    assert!((team.boost.bpm - 200.0).abs() < 1e-3, "bpm averages");
    assert!(
        (team.movement.percent_supersonic_speed - 15.0).abs() < 1e-3,
        "percent averages"
    );
}

/// A minimal two-player-per-side document in ballchasing's nesting.
fn doc(blue_goals: [(i32, &str); 2], collected: [f32; 2]) -> Value {
    let player = |name: &str, goals: i32, coll: f32| {
        json!({
            "name": name,
            "stats": {
                "core": {"goals": goals, "shots": goals},
                "boost": {"amount_collected": coll, "count_collected_big": 3},
                "movement": {"percent_supersonic_speed": 10.0}
            }
        })
    };
    json!({
        "blue": {"players": [
            player(blue_goals[0].1, blue_goals[0].0, collected[0]),
            player(blue_goals[1].1, blue_goals[1].0, collected[1]),
        ]},
        "orange": {"players": []}
    })
}

#[test]
fn validator_passes_identical_and_flags_drift() {
    let ours = doc([(2, "Alice"), (1, "Bob")], [500.0, 400.0]);

    // Identical → fully paired, no failures.
    let report = compare_documents(&ours, &ours);
    assert_eq!(report.players_paired, 2);
    assert!(
        report.failures().is_empty(),
        "identical docs must pass: {:?}",
        report.failures()
    );

    // Perturb an EXACT field (goals) → a failure naming it.
    let drift_exact = doc([(2, "Alice"), (2, "Bob")], [500.0, 400.0]);
    let fails = compare_documents(&ours, &drift_exact).failures();
    assert!(
        fails.iter().any(|f| f.contains("goals")),
        "exact drift not caught: {fails:?}"
    );

    // Perturb a CORE continuous field beyond tolerance (amount_collected +40%).
    let drift_core = doc([(2, "Alice"), (1, "Bob")], [700.0, 560.0]);
    let fails = compare_documents(&ours, &drift_core).failures();
    assert!(
        fails.iter().any(|f| f.contains("amount_collected")),
        "core drift not caught: {fails:?}"
    );
}

#[test]
fn validator_flags_unpaired_roster() {
    let ours = doc([(2, "Alice"), (1, "Bob")], [500.0, 400.0]);
    // Their doc has a different second name on the same side, with no residual
    // 1-to-1 fallback available beyond the exact Alice match.
    let theirs = json!({
        "blue": {"players": [
            {"name": "Alice", "stats": {"core": {"goals": 2}}},
        ]},
        "orange": {"players": []}
    });
    let report = compare_documents(&ours, &theirs);
    assert!(
        report.failures().iter().any(|f| f.contains("roster")),
        "roster mismatch not flagged: {:?}",
        report.failures()
    );
}
