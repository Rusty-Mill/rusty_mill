//! Drift guard: a metric that is a reducer over episodes must equal the `Report`'s
//! raw value bit for bit, so the list a player sees can never disagree with the score.

use replay_analyzer::{analyze::build_canonical, decode::boxcars_adapter::BoxcarsParser};
use replay_analyzer::{decode::ReplayParser, model::Event, CanonicalMatch};
use replay_scoring::episodes::Outcome;
use replay_scoring::{extract, score, Episode, ScoreConfig};

fn canonical(name: &str) -> CanonicalMatch {
    let path = format!(
        "{}/../assets/replays/{name}.replay",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    build_canonical(&BoxcarsParser::new().parse(&data).expect("decode"), name)
}

fn is_recovery(e: &&Episode) -> bool {
    matches!(e, Episode::Recovery { .. })
}
fn is_loss(e: &&Episode) -> bool {
    matches!(e, Episode::Loss { .. })
}
fn is_demo(e: &&Episode) -> bool {
    matches!(e, Episode::Demo { .. })
}
fn is_shot(e: &&Episode) -> bool {
    matches!(e, Episode::Shot { .. })
}
fn is_challenge(e: &&Episode) -> bool {
    matches!(e, Episode::Challenge { .. })
}

/// For each player on both sample replays: `reduce(that player's episodes of one
/// kind)` must equal the report's raw `key`, bit for bit.
fn assert_reducer(key: &str, kind: fn(&&Episode) -> bool, reduce: fn(&[&Episode]) -> Option<f32>) {
    let cfg = ScoreConfig::default();
    for name in ["42f2", "419a"] {
        let m = canonical(name);
        let eps = extract(&m, &cfg);
        assert!(eps.iter().any(|e| kind(&e)), "{name}: no {key} episodes");
        for t in &m.tracks {
            let mine: Vec<&Episode> = eps
                .iter()
                .filter(kind)
                .filter(|e| e.pri() == t.pri)
                .collect();
            let got = score(&m, t.pri, &cfg)
                .metrics
                .iter()
                .find(|b| b.key == key)
                .and_then(|b| b.raw);
            assert_eq!(
                reduce(&mine).map(f32::to_bits),
                got.map(f32::to_bits),
                "{name} {key} pri {}",
                t.pri
            );
        }
    }
}

#[test]
fn recovery_speed_is_the_mean_recovery_duration() {
    assert_reducer("recovery_speed", is_recovery, |es| {
        (!es.is_empty()).then(|| es.iter().map(|e| e.dur()).sum::<f32>() / es.len() as f32)
    });
}

#[test]
fn challenge_timing_is_the_share_of_clean_challenges() {
    assert_reducer("challenge_timing", is_challenge, |es| {
        (!es.is_empty()).then(|| es.iter().filter(|e| e.is_ok()).count() as f32 / es.len() as f32)
    });
}

#[test]
fn dangerous_turnover_is_total_loss_danger_per_followed_touch() {
    let cfg = ScoreConfig::default();
    for name in ["42f2", "419a"] {
        let m = canonical(name);
        let eps = extract(&m, &cfg);
        let touches: Vec<i32> = m
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Touch { pri, .. } => Some(*pri),
                _ => None,
            })
            .collect();
        for t in &m.tracks {
            let followed = touches.windows(2).filter(|w| w[0] == t.pri).count();
            let danger = eps
                .iter()
                .filter(|e| is_loss(e) && e.pri() == t.pri)
                .fold(0.0f32, |s, e| s + e.danger());
            let want = (followed > 0).then(|| danger / followed as f32);
            let got = score(&m, t.pri, &cfg)
                .metrics
                .iter()
                .find(|b| b.key == "dangerous_turnover")
                .and_then(|b| b.raw);
            assert_eq!(
                want.map(f32::to_bits),
                got.map(f32::to_bits),
                "{name} pri {}",
                t.pri
            );
        }
    }
}

#[test]
fn extract_is_time_ordered_and_deterministic() {
    let (m, cfg) = (canonical("419a"), ScoreConfig::default());
    let eps = extract(&m, &cfg);
    assert!(eps.windows(2).all(|w| w[0].t() <= w[1].t()));
    assert_eq!(eps, extract(&m, &cfg));
    let n = |k: fn(&&Episode) -> bool| eps.iter().filter(k).count();
    assert_eq!(
        (
            n(is_recovery),
            n(is_challenge),
            n(is_loss),
            n(is_shot),
            n(is_demo)
        ),
        (100, 186, 100, 19, 3), // recorded from the first run
        "419a episode counts — a change means a definition moved"
    );
}

#[test]
fn shots_per_player_match_the_header_counters_and_carry_outcomes() {
    for (name, goals_credited) in [("42f2", 6), ("419a", 4)] {
        let m = canonical(name);
        let eps = extract(&m, &ScoreConfig::default());
        for t in &m.tracks {
            let shots = eps
                .iter()
                .filter(is_shot)
                .filter(|e| e.pri() == t.pri)
                .count();
            let want = m
                .players
                .iter()
                .find(|p| p.name == t.player)
                .map(|p| p.shots);
            assert_eq!(Some(shots as i32), want, "{name} {}", t.player);
        }
        let outcome = |o: Outcome| {
            eps.iter()
                .filter(|e| matches!(e, Episode::Shot { outcome, .. } if *outcome == o))
                .count()
        };
        // A rebound goal can come without its own shot tick, so goals can trail the header.
        assert_eq!(outcome(Outcome::Goal), goals_credited, "{name} goals");
    }
}

#[test]
fn demo_episodes_match_the_authoritative_demo_events() {
    let m = canonical("419a");
    let eps = extract(&m, &ScoreConfig::default());
    let demos: Vec<(i32, i32, f32, bool)> = eps
        .iter()
        .filter_map(|e| match e {
            Episode::Demo {
                pri,
                victim,
                down,
                goal,
                ..
            } => Some((*pri, *victim, *down, *goal)),
            _ => None,
        })
        .collect();
    // Same three demolitions the header/replicated events record; each victim is out ~3 s.
    assert_eq!(demos.len(), 3);
    assert!(demos.iter().all(|d| (2.8..3.3).contains(&d.2)), "{demos:?}");
    assert!(demos.iter().all(|d| d.0 != d.1));
}

#[test]
fn shots_carry_a_probability_and_the_model_artifact_round_trips() {
    let m = canonical("419a");
    let eps = extract(&m, &ScoreConfig::default());
    let xgs: Vec<f32> = eps
        .iter()
        .filter_map(|e| match e {
            Episode::Shot { xg, .. } => Some(*xg),
            _ => None,
        })
        .collect();
    assert_eq!(xgs.len(), 19);
    assert!(xgs.iter().all(|p| (0.0..1.0).contains(p)), "{xgs:?}");
    let total: f32 = xgs.iter().sum();
    assert!(
        (2.0..12.0).contains(&total),
        "a sanity band for 19 shots and 5 goals, not a calibration: expected goals {total}"
    );

    let model = replay_scoring::XgModel::default();
    let back: replay_scoring::XgModel =
        serde_json::from_str(&serde_json::to_string(&model).unwrap()).unwrap();
    assert_eq!(back, model);
}
