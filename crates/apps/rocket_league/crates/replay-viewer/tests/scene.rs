//! Unit tests for scene projection and HTML substitution, over a hand-built
//! minimal canonical match (so no `.replay` decode is needed).

use std::collections::BTreeMap;

use replay_analyzer::model::{
    CanonicalMatch, Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, Rot3, Vec3,
};
use replay_skills::report::SkillInstance;
use replay_skills::Skill;
use replay_viewer::{build_scene, html};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn car(pri: i32, p: Vec3, rot: Option<Rot3>, boost: Option<u8>) -> GridCar {
    GridCar {
        pri,
        team: Some(0),
        p,
        v: v(0.0, 0.0, 0.0),
        boost,
        rot,
    }
}

fn minimal_match() -> CanonicalMatch {
    let frames = vec![
        GridFrame {
            t: 0.0,
            ball: Some(Kin {
                p: v(1.4, 2.6, 93.2),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![car(
                1,
                v(10.4, 20.6, 17.2),
                Some(Rot3 {
                    pitch: 0.1234,
                    yaw: 1.5,
                    roll: -0.4,
                }),
                Some(255),
            )],
        },
        GridFrame {
            t: 0.5,
            ball: None, // ball not live this frame
            cars: vec![car(1, v(30.0, 40.0, 18.0), None, None)],
        },
    ];
    let mut team_scores = BTreeMap::new();
    team_scores.insert(0, 1);
    team_scores.insert(1, 0);
    CanonicalMatch {
        replay_id: "test".into(),
        parser_version: "test".into(),
        analyzer_version: "test".into(),
        map: Some("map".into()),
        team_size: Some(2),
        record_fps: Some(30.0),
        num_frames: 2,
        duration_s: 0.5,
        team_scores,
        players: vec![],
        tracks: vec![PlayerTrack {
            player: "Alice".into(),
            pri: 1,
            team: Some(0),
            num_segments: 1,
            samples: vec![],
            gaps: vec![],
        }],
        frames: vec![],
        resampled: Resampled {
            hz: 30.0,
            team_attack_sign: BTreeMap::new(),
            frames,
        },
        events: vec![
            Event::Possession {
                team: 0,
                start: 0.1,
                end: 0.4,
                touches: 2,
            }, // dropped
            Event::Touch {
                t: 0.2,
                pri: 1,
                player: Some("Alice".into()),
                team: Some(0),
            },
            Event::Goal {
                t: 0.45,
                scorer: Some("Alice".into()),
                team: Some(0),
            },
        ],
        features: vec![],
    }
}

#[test]
fn scene_projects_roster_frames_and_rounds() {
    let m = minimal_match();
    let skills = vec![SkillInstance {
        skill: Skill::Aerial,
        t: 0.30,
        pri: 1,
        player: Some("Alice".into()),
        team: Some(0),
        confidence: 1.0,
        detail: String::new(),
    }];
    let s = build_scene(&m, &skills);

    assert_eq!(s.players.len(), 1);
    assert_eq!(s.players[0].name, "Alice");
    assert_eq!(s.frames.len(), 2);

    // Positions round to whole uu; the ball is present frame 0, absent frame 1.
    assert_eq!(s.frames[0].cars[0].p, [10.0, 21.0, 17.0]);
    assert_eq!(s.frames[0].ball, Some([1.0, 3.0, 93.0]));
    assert_eq!(s.frames[1].ball, None);
    // Raw boost byte 255 -> 100%, rotation rounded to 1e-3, missing -> 0.
    assert_eq!(s.frames[0].cars[0].boost, 100);
    assert_eq!(s.frames[0].cars[0].rot, [0.123, 1.5, -0.4]);
    assert_eq!(s.frames[1].cars[0].boost, 0);
    assert_eq!(s.frames[1].cars[0].rot, [0.0, 0.0, 0.0]);
}

#[test]
fn scene_merges_events_and_skills_drops_possessions() {
    let m = minimal_match();
    let skills = vec![SkillInstance {
        skill: Skill::Aerial,
        t: 0.30,
        pri: 1,
        player: Some("Alice".into()),
        team: Some(0),
        confidence: 1.0,
        detail: String::new(),
    }];
    let s = build_scene(&m, &skills);

    // touch (0.2) + skill (0.3) + goal (0.45); possession is dropped; time-sorted.
    let kinds: Vec<&str> = s.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["touch", "skill", "goal"]);
    assert!(s.events.iter().all(|e| e.kind != "possession"));
    assert!(s.events[1].label.contains("Aerial"));
}

#[test]
fn html_embeds_scene_and_substitutes_placeholders() {
    let m = minimal_match();
    let out = html(&build_scene(&m, &[]));
    // Template anchors present, placeholders fully substituted.
    assert!(out.contains("const S = {"), "scene literal embedded");
    assert!(
        !out.contains("/*SCENE_DATA*/"),
        "scene placeholder replaced"
    );
    assert!(!out.contains("{{TITLE}}"), "title placeholder replaced");
    assert!(out.contains("three@0.160.0"), "three.js import present");
    assert!(out.contains("\"Alice\""), "player data embedded");
}

#[test]
fn html_escapes_angle_brackets_in_player_names() {
    let mut m = minimal_match();
    m.tracks[0].player = "</script><b>x".into();
    let out = html(&build_scene(&m, &[]));
    // The raw closing tag must not appear literally inside the embedded data.
    assert!(!out.contains("</script><b>x"), "angle brackets escaped");
    assert!(out.contains("\\u003c/script"), "escaped as unicode");
}

#[test]
fn html_offline_embeds_three_as_data_urls_no_cdn() {
    let out = replay_viewer::html_offline(&build_scene(&minimal_match(), &[]));
    assert!(
        out.contains("data:text/javascript;base64,"),
        "three embedded as a data url"
    );
    assert!(
        !out.contains("cdn.jsdelivr.net"),
        "no CDN reference in offline output"
    );
    // The viewer imports this exact specifier, so the offline map must key it exactly.
    assert!(out.contains("three/addons/controls/OrbitControls.js"));
    assert!(out.contains("const S = {") && !out.contains("/*IMPORTMAP*/"));
}

#[test]
fn attach_roles_tags_cars_with_a_man_role() {
    let m = minimal_match();
    let mut s = build_scene(&m, &[]);
    assert_eq!(
        s.frames[0].cars[0].role, None,
        "build_scene leaves role unset"
    );
    replay_viewer::attach_roles(&mut s, &m, &replay_scoring::ScoreConfig::default());
    assert!(
        matches!(s.frames[0].cars[0].role, Some(1) | Some(2)),
        "a live car gets a 1st/2nd-man role"
    );
}

#[test]
fn attach_winprob_fills_a_valid_momentum_curve() {
    let m = minimal_match();
    let mut s = build_scene(&m, &[]);
    assert!(s.win_prob.is_empty(), "build_scene leaves win_prob empty");
    replay_viewer::attach_winprob(&mut s, &m, &replay_value::ValueConfig::default());
    assert!(!s.win_prob.is_empty(), "win_prob curve attached");
    assert!(
        s.win_prob.iter().all(|&p| (0.0..=1.0).contains(&p)),
        "probabilities stay in [0,1]"
    );
}

#[test]
fn downsample_thins_frames_and_updates_hz() {
    let m = minimal_match();
    let mut s = build_scene(&m, &[]);
    assert_eq!((s.frames.len(), s.hz), (2, 30.0));
    // 30 -> 15 Hz keeps every 2nd frame.
    replay_viewer::scene::downsample(&mut s, 15.0);
    assert_eq!((s.frames.len(), s.hz), (1, 15.0));
    // Target at/above the current rate is a no-op.
    replay_viewer::scene::downsample(&mut s, 30.0);
    assert_eq!((s.frames.len(), s.hz), (1, 15.0));
}
