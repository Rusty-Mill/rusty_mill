//! Tests for linking skills to the value model's per-touch ΔV (`value_link`).
//! Pure: ΔV is fed in as synthetic `TouchDv` records, no value model needed.

use std::collections::BTreeMap;

use replay_skills::report::{PlayerSkills, SkillInstance, SkillReport};
use replay_skills::{skill_values, Skill, TouchDv};

fn inst(skill: Skill, pri: i32, t: f32) -> SkillInstance {
    SkillInstance {
        skill,
        t,
        pri,
        player: Some(format!("p{pri}")),
        team: Some(0),
        confidence: 1.0,
        metric: 0.0,
        detail: String::new(),
    }
}

fn report(instances: Vec<SkillInstance>, counts: &[(Skill, usize)]) -> SkillReport {
    SkillReport {
        replay_id: "t".into(),
        config_version: "t".into(),
        analyzer_version: "t".into(),
        parser_version: "t".into(),
        instances,
        players: vec![PlayerSkills {
            pri: 1,
            player: "p1".into(),
            team: Some(0),
            counts: counts.iter().copied().collect::<BTreeMap<_, _>>(),
        }],
    }
}

#[test]
fn links_reps_to_same_player_touches_and_leaves_others_unlinked() {
    let r = report(
        vec![
            inst(Skill::Aerial, 1, 10.0),   // -> p1 touch @10.0 (dv +0.2)
            inst(Skill::Aerial, 1, 20.0),   // -> p1 touch @20.0 (dv -0.1)
            inst(Skill::Redirect, 1, 30.0), // no touch within tol -> unlinked
            inst(Skill::Demo, 1, 10.0),     // coincides with a touch, but a demo
                                            // is not a ball contact -> never linked
        ],
        &[(Skill::Aerial, 2), (Skill::Redirect, 1), (Skill::Demo, 1)],
    );
    let touch_dv = vec![
        TouchDv {
            pri: 1,
            t: 10.0,
            dv: 0.2,
        },
        TouchDv {
            pri: 1,
            t: 20.0,
            dv: -0.1,
        },
        // A different player's touch must never credit p1's nearby redirect.
        TouchDv {
            pri: 2,
            t: 30.0,
            dv: 0.9,
        },
    ];

    let out = skill_values(&r, &touch_dv, 0.1);
    let p1 = out.iter().find(|v| v.pri == 1).expect("p1");

    let (n, sum) = p1.by_skill[&Skill::Aerial];
    assert_eq!(n, 2, "both aerials linked");
    assert!((sum - 0.1).abs() < 1e-6, "0.2 + (-0.1) = 0.1");
    assert!(
        !p1.by_skill.contains_key(&Skill::Redirect),
        "redirect has no touch within tol"
    );
    assert!(
        !p1.by_skill.contains_key(&Skill::Demo),
        "a demo is not a ball contact, even atop a touch"
    );
    assert_eq!(p1.linked, 2);
    assert!((p1.sum_dv - 0.1).abs() < 1e-6);
    assert!((p1.mean_dv() - 0.05).abs() < 1e-6);
    assert!((p1.mean_dv_for(Skill::Aerial) - 0.05).abs() < 1e-6);
    assert_eq!(p1.mean_dv_for(Skill::Redirect), 0.0);
}

#[test]
fn picks_the_nearest_touch_within_tolerance() {
    let r = report(vec![inst(Skill::Flick, 1, 10.0)], &[(Skill::Flick, 1)]);
    // Two candidates inside the 0.1s window; the closer one (10.03) wins.
    let touch_dv = vec![
        TouchDv {
            pri: 1,
            t: 10.03,
            dv: 0.5,
        },
        TouchDv {
            pri: 1,
            t: 10.08,
            dv: 0.9,
        },
    ];
    let out = skill_values(&r, &touch_dv, 0.1);
    let p1 = &out[0];
    assert!(
        (p1.mean_dv_for(Skill::Flick) - 0.5).abs() < 1e-6,
        "nearest touch credited"
    );
    assert_eq!(p1.linked, 1);
}
