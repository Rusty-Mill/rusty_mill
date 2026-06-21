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
    // Closest/farthest-to-ball are computed (no longer stubbed at 0).
    assert!(
        p["positioning"]["percent_closest_to_ball"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(
        p["positioning"]["percent_farthest_from_ball"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    // Car body is decoded from the loadout (ballchasing's car_id/car_name).
    assert!(blue[0]["car_id"].as_u64().unwrap() > 0);
    assert!(blue[0]["car_name"].as_str().is_some_and(|s| !s.is_empty()));
    // Camera profile + steering sensitivity decoded from the network stream.
    assert!(blue[0]["camera"]["fov"].as_f64().unwrap() > 0.0);
    assert!(blue[0]["camera"]["distance"].as_f64().unwrap() > 0.0);
    assert!(blue[0]["steering_sensitivity"].as_f64().unwrap() > 0.0);
    // Field counts match the schema (28 boost, 27 positioning channels).
    assert_eq!(p["boost"].as_object().unwrap().len(), 28);
    assert_eq!(p["positioning"].as_object().unwrap().len(), 27);
}

#[test]
fn html_dashboard_renders_self_contained() {
    use replay_analyzer::analyze::build_canonical;
    use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
    use replay_analyzer::decode::ReplayParser;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/replays/419a.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, "419a");
    let doc = ballchasing_document(&canonical);
    let page = bc_clone::html::render_html(&doc, &canonical);

    // Self-contained: one HTML doc with inline CSS, no external assets / scripts.
    assert!(page.starts_with("<!doctype html>"));
    assert!(page.contains("<style>") && !page.contains("<script"));
    assert!(!page.contains("http://") && !page.contains("https://"));
    // The ballchasing-style tab set and a couple of section headings are present.
    for s in [
        "Overview",
        "Core",
        "Ball",
        "Boost",
        "Movement",
        "Positioning",
        "Heatmaps",
        "Demos",
    ] {
        assert!(page.contains(&format!(">{s}</label>")), "missing tab {s}");
    }
    assert!(page.contains("Scoreboard") && page.contains("Team stats overview"));
    assert!(page.contains("Positioning heatmaps") && page.contains("<svg"));
    // The Overview tab carries the Game Timeline (lanes + score band + legend).
    assert!(
        page.contains("Game timeline") && page.contains("class=\"tl-score\""),
        "game timeline missing"
    );
    // The Boost tab carries the per-player pad pickup maps.
    assert!(page.contains("Pickup maps"), "boost pickup maps missing");
    // The scoreboard shows the decoded car body.
    assert!(
        page.contains("<th>CAR</th>") && page.contains("Octane"),
        "car column / name missing from scoreboard"
    );
    // The Overview carries the per-player camera & settings table.
    assert!(
        page.contains("Camera &amp; settings"),
        "camera table missing"
    );
    // The Boost tab carries the (approximate) supersonic-boost column + caveat.
    assert!(
        page.contains("SS used*") && page.contains("boost burned while supersonic"),
        "supersonic-boost column / caveat missing"
    );
    // A real player name from the sample shows up in the scoreboard.
    assert!(page.contains("Nadir"), "player name missing from dashboard");
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
            car_id: None,
            car_name: None,
            camera: None,
            steering_sensitivity: None,
            stats: s,
        }
    };
    let team = aggregate_side(&[mk(2, 100.0, 3, 10.0), mk(1, 300.0, 5, 20.0)]);
    assert_eq!(team.core.goals, 3, "goals sum");
    assert_eq!(team.boost.count_collected_big, 8, "counts sum");
    // BPM is a per-minute rate that adds across teammates (ballchasing sums it).
    assert!((team.boost.bpm - 400.0).abs() < 1e-3, "bpm sums");
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
