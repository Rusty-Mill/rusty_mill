//! Tests for skill→goal outcome linking.

use std::collections::BTreeMap;

use replay_analyzer::model::Event;
use replay_skills::report::{PlayerSkills, SkillInstance, SkillReport};
use replay_skills::{outcomes, Skill};

fn inst(skill: Skill, pri: i32, team: i32, t: f32) -> SkillInstance {
    SkillInstance {
        skill,
        t,
        pri,
        player: Some(format!("p{pri}")),
        team: Some(team),
        confidence: 1.0,
        detail: String::new(),
    }
}

#[test]
fn counts_skills_within_window_before_a_same_team_goal() {
    let report = SkillReport {
        replay_id: "t".into(),
        config_version: "t".into(),
        analyzer_version: "t".into(),
        parser_version: "t".into(),
        instances: vec![
            inst(Skill::Aerial, 1, 0, 45.0), // team 0, 5s before the goal -> counts
            inst(Skill::Aerial, 1, 0, 30.0), // team 0, 20s before -> doesn't
            inst(Skill::Demo, 2, 1, 48.0),   // team 1, but the goal is team 0 -> doesn't
        ],
        players: vec![
            PlayerSkills {
                pri: 1,
                player: "p1".into(),
                team: Some(0),
                counts: [(Skill::Aerial, 2)].into_iter().collect::<BTreeMap<_, _>>(),
            },
            PlayerSkills {
                pri: 2,
                player: "p2".into(),
                team: Some(1),
                counts: [(Skill::Demo, 1)].into_iter().collect::<BTreeMap<_, _>>(),
            },
        ],
    };
    let events = vec![Event::Goal {
        t: 50.0,
        scorer: None,
        team: Some(0),
    }];

    let outs = outcomes(&report, &events, 6.0);
    let p1 = outs.iter().find(|o| o.pri == 1).expect("p1");
    assert_eq!(
        p1.by_skill[&Skill::Aerial],
        (2, 1),
        "1 of 2 aerials within 6s before the goal"
    );
    assert_eq!(p1.total_before_goal, 1);
    let p2 = outs.iter().find(|o| o.pri == 2).expect("p2");
    assert_eq!(
        p2.total_before_goal, 0,
        "a team-1 demo doesn't count toward a team-0 goal"
    );
}
