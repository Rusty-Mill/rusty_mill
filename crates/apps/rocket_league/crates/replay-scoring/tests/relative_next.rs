//! Gap-to-next-bracket: each metric carries the median of the bracket above the
//! one the player is graded against, and the top bracket has none.

use std::collections::BTreeMap;

use replay_analyzer::decode::ReplayParser;
use replay_analyzer::{analyze::build_canonical, decode::boxcars_adapter::BoxcarsParser};
use replay_scoring::config::Metric;
use replay_scoring::relative::{NormSample, RankNorms};
use replay_scoring::{attach_relative, score_all, ScoreConfig};

fn bucket(name: &str, tier: f32, recovery: f32) -> Vec<NormSample> {
    (0..5)
        .map(|_| NormSample {
            bucket: name.into(),
            tier,
            composite: 50.0,
            raws: BTreeMap::from([(Metric::RecoverySpeed, Some(recovery))]),
        })
        .collect()
}

#[test]
fn next_median_is_the_bracket_above_and_none_at_the_top() {
    let path = format!(
        "{}/../assets/replays/419a.replay",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(path).expect("read 419a");
    let m = build_canonical(&BoxcarsParser::new().parse(&data).expect("decode"), "419a");
    let cfg = ScoreConfig::default();
    let norms = RankNorms::build(
        "t",
        &[bucket("gold", 8.0, 1.0), bucket("diamond", 14.0, 3.0)].concat(),
        1,
    );
    let med = |name: &str, bracket: &str| {
        let mut reports = score_all(&m, &cfg);
        attach_relative(&mut reports, &norms, &cfg, Some(bracket));
        let rel = reports[0].relative.clone().expect("relative");
        assert_eq!(rel.bracket, name);
        rel.metrics
            .into_iter()
            .find(|x| x.key == "recovery_speed")
            .expect("metric")
    };
    let gold = med("gold", "gold");
    assert_eq!((gold.bracket_median, gold.next_median), (1.0, Some(3.0)));
    assert_eq!(med("diamond", "diamond").next_median, None);
}
