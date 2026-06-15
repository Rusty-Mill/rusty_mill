//! Tests for the verification query surface on [`SkillReport`] — "was this skill
//! performed", sliced by player and time window. Built from a hand-assembled
//! report so the queries are exercised independently of detection.

use std::collections::BTreeMap;

use replay_skills::report::{PlayerSkills, SkillInstance, SkillReport};
use replay_skills::Skill;

fn inst(skill: Skill, t: f32, pri: i32) -> SkillInstance {
    SkillInstance {
        skill,
        t,
        pri,
        player: Some(format!("p{pri}")),
        team: Some(if pri == 1 { 0 } else { 1 }),
        confidence: 1.0,
        detail: String::new(),
    }
}

fn player(pri: i32, team: i32, counts: &[(Skill, usize)]) -> PlayerSkills {
    PlayerSkills {
        pri,
        player: format!("p{pri}"),
        team: Some(team),
        counts: counts.iter().copied().collect::<BTreeMap<_, _>>(),
    }
}

fn report() -> SkillReport {
    // p1 (team 0): two aerials + a demo. p2 (team 1): one aerial, no demo.
    SkillReport {
        replay_id: "fixture".into(),
        config_version: "skcfg-test".into(),
        analyzer_version: "test".into(),
        parser_version: "test".into(),
        instances: vec![
            inst(Skill::Aerial, 5.0, 1),
            inst(Skill::Aerial, 40.0, 1),
            inst(Skill::Demo, 42.0, 1),
            inst(Skill::Aerial, 60.0, 2),
        ],
        players: vec![
            player(1, 0, &[(Skill::Aerial, 2), (Skill::Demo, 1)]),
            player(2, 1, &[(Skill::Aerial, 1)]),
        ],
    }
}

#[test]
fn performed_anywhere_in_replay() {
    let r = report();
    assert!(r.performed(Skill::Aerial));
    assert!(r.performed(Skill::Demo));
    assert!(!r.performed(Skill::CeilingPlay));
}

#[test]
fn performed_by_pri_and_by_name() {
    let r = report();
    assert!(r.performed_by(Skill::Demo, 1));
    assert!(!r.performed_by(Skill::Demo, 2));
    assert!(r.performed_by_name(Skill::Aerial, "p2"));
    assert!(!r.performed_by_name(Skill::Demo, "p2"));
    // Unknown player verifies as not-performed rather than panicking.
    assert!(!r.performed_by_name(Skill::Aerial, "ghost"));
}

#[test]
fn counts_and_totals() {
    let r = report();
    assert_eq!(r.count_for(Skill::Aerial, 1), 2);
    assert_eq!(r.count_for(Skill::Aerial, 2), 1);
    assert_eq!(r.count_for(Skill::Demo, 2), 0);
    assert_eq!(r.total(Skill::Aerial), 3);
    assert_eq!(r.total(Skill::Demo), 1);
}

#[test]
fn performed_within_time_window() {
    let r = report();
    // p1's aerials are at t=5 and t=40; demo at t=42.
    assert!(r.performed_in_window(Skill::Aerial, 0.0, 10.0));
    assert!(!r.performed_in_window(Skill::Demo, 0.0, 10.0));
    assert!(r.performed_in_window(Skill::Demo, 41.0, 43.0));

    // Window + player: p2's only aerial is at t=60.
    assert!(r.performed_by_in_window(Skill::Aerial, 2, 55.0, 65.0));
    assert!(!r.performed_by_in_window(Skill::Aerial, 2, 0.0, 10.0));
    assert!(!r.performed_by_in_window(Skill::Aerial, 1, 55.0, 65.0));
}

#[test]
fn instances_of_are_filtered_and_time_ordered() {
    let r = report();
    let aerials: Vec<f32> = r.instances_of(Skill::Aerial).map(|i| i.t).collect();
    assert_eq!(aerials, vec![5.0, 40.0, 60.0]);
    assert_eq!(r.instances_of(Skill::Demo).count(), 1);
}

#[test]
fn player_rollup_lookup() {
    let r = report();
    let p1 = r.player(1).expect("p1 present");
    assert!(p1.performed(Skill::Aerial));
    assert_eq!(p1.count(Skill::Aerial), 2);
    assert_eq!(p1.skills(), vec![Skill::Aerial, Skill::Demo]);
    assert!(r.player(99).is_none());
}
