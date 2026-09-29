//! Drift guard: a metric that is a reducer over episodes must equal the `Report`'s
//! raw value bit for bit, so the list a player sees can never disagree with the score.

use replay_analyzer::{analyze::build_canonical, decode::boxcars_adapter::BoxcarsParser};
use replay_analyzer::{decode::ReplayParser, CanonicalMatch};
use replay_scoring::{extract, score, Episode, ScoreConfig};

fn canonical(name: &str) -> CanonicalMatch {
    let path = format!(
        "{}/../assets/replays/{name}.replay",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    build_canonical(&BoxcarsParser::new().parse(&data).expect("decode"), name)
}

#[test]
fn recovery_speed_is_the_mean_recovery_duration() {
    let cfg = ScoreConfig::default();
    for name in ["42f2", "419a"] {
        let m = canonical(name);
        let eps = extract(&m, &cfg);
        assert!(!eps.is_empty(), "{name}: no recoveries");
        for t in &m.tracks {
            let durs: Vec<f32> = eps
                .iter()
                .filter(|e| e.pri() == t.pri)
                .map(Episode::dur)
                .collect();
            let want = (!durs.is_empty()).then(|| durs.iter().sum::<f32>() / durs.len() as f32);
            let got = score(&m, t.pri, &cfg)
                .metrics
                .iter()
                .find(|b| b.key == "recovery_speed")
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
    assert_eq!(
        eps.len(),
        100,
        "419a recovery count — a change means the definition moved"
    );
}
