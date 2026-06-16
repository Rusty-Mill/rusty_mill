//! Tests for per-player skill proficiency profiles (rates + quality proxy).

use std::collections::BTreeMap;

use replay_skills::report::{PlayerSkills, SkillInstance, SkillReport};
use replay_skills::{profiles, Skill};

fn inst(skill: Skill, pri: i32, conf: f32, metric: f32) -> SkillInstance {
    SkillInstance {
        skill,
        t: 0.0,
        pri,
        player: Some(format!("p{pri}")),
        team: Some(0),
        confidence: conf,
        metric,
        detail: String::new(),
    }
}

#[test]
fn profile_computes_rates_and_quality() {
    // p1: 3 aerials (conf .6/.8/1.0, peak height 1000/1500/2000 uu) + 1 demo,
    // over a 2-minute match.
    let counts: BTreeMap<Skill, usize> =
        [(Skill::Aerial, 3), (Skill::Demo, 1)].into_iter().collect();
    let report = SkillReport {
        replay_id: "t".into(),
        config_version: "t".into(),
        analyzer_version: "t".into(),
        parser_version: "t".into(),
        instances: vec![
            inst(Skill::Aerial, 1, 0.6, 1000.0),
            inst(Skill::Aerial, 1, 0.8, 1500.0),
            inst(Skill::Aerial, 1, 1.0, 2000.0),
            inst(Skill::Demo, 1, 1.0, 1.0),
        ],
        players: vec![PlayerSkills {
            pri: 1,
            player: "p1".into(),
            team: Some(0),
            counts,
        }],
    };

    let pr = profiles(&report, 120.0);
    assert_eq!(pr.len(), 1);
    let aer = pr[0].stat(Skill::Aerial).expect("aerial stat");
    assert_eq!(aer.count, 3);
    assert!(
        (aer.per_min - 1.5).abs() < 1e-3,
        "3 aerials / 2 min = 1.5/min"
    );
    assert!(
        (aer.mean_quality - 0.8).abs() < 1e-3,
        "mean of .6/.8/1.0 = .8"
    );
    assert!(
        (aer.mean_metric - 1500.0).abs() < 1e-3,
        "mean peak height of 1000/1500/2000 = 1500"
    );
    assert!(
        (pr[0].total_per_min - 2.0).abs() < 1e-3,
        "4 skills / 2 min = 2/min"
    );
    // A skill not performed has no stat.
    assert!(pr[0].stat(Skill::Flick).is_none());
}

#[test]
fn profile_handles_zero_duration_without_dividing_by_zero() {
    let report = SkillReport {
        replay_id: "t".into(),
        config_version: "t".into(),
        analyzer_version: "t".into(),
        parser_version: "t".into(),
        instances: vec![inst(Skill::Supersonic, 1, 1.0, 0.0)],
        players: vec![PlayerSkills {
            pri: 1,
            player: "p1".into(),
            team: Some(0),
            counts: [(Skill::Supersonic, 1)].into_iter().collect(),
        }],
    };
    let pr = profiles(&report, 0.0);
    assert!(
        pr[0].total_per_min.is_finite(),
        "rate stays finite at zero duration"
    );
}
