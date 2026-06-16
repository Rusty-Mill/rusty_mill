//! Tests the impact bridge wiring: `attach_impact` sets per-player impact and
//! tags `touch` events with ΔV. The ΔV math itself is covered by `replay-value`;
//! here we verify the scene gets populated and non-touch events stay untagged.

use std::collections::BTreeMap;

use replay_analyzer::model::{
    CanonicalMatch, Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, Vec3,
};
use replay_value::ValueConfig;
use replay_viewer::{attach_impact, build_scene};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// A frame with the ball live (so a touch has a computable before/after state)
/// and one car per team.
fn frame(t: f32, ball_y: f32) -> GridFrame {
    GridFrame {
        t,
        ball: Some(Kin {
            p: v(0.0, ball_y, 100.0),
            v: v(0.0, 800.0, 0.0),
        }),
        cars: vec![
            GridCar {
                pri: 1,
                team: Some(0),
                p: v(0.0, ball_y - 100.0, 17.0),
                v: v(0.0, 800.0, 0.0),
                boost: Some(150),
                rot: None,
            },
            GridCar {
                pri: 2,
                team: Some(1),
                p: v(0.0, ball_y + 300.0, 17.0),
                v: v(0.0, 0.0, 0.0),
                boost: Some(100),
                rot: None,
            },
        ],
    }
}

fn match_with_touch() -> CanonicalMatch {
    // Ball present in every frame, advancing toward +Y, with a team-0 goal so the
    // value model has an objective to learn (and the touch a before/after state).
    let frames: Vec<GridFrame> = (0..=30)
        .map(|k| {
            let t = k as f32 * 0.1;
            frame(t, -2000.0 + t * 800.0)
        })
        .collect();
    let mut team_scores = BTreeMap::new();
    team_scores.insert(0, 1);
    team_scores.insert(1, 0);
    let mut signs = BTreeMap::new();
    signs.insert(0, 1);
    signs.insert(1, -1);
    CanonicalMatch {
        replay_id: "t".into(),
        parser_version: "t".into(),
        analyzer_version: "t".into(),
        map: Some("map".into()),
        team_size: Some(1),
        record_fps: Some(30.0),
        num_frames: 31,
        duration_s: 3.0,
        team_scores,
        players: vec![],
        tracks: vec![
            PlayerTrack {
                player: "Alice".into(),
                pri: 1,
                team: Some(0),
                num_segments: 1,
                samples: vec![],
                gaps: vec![],
            },
            PlayerTrack {
                player: "Bob".into(),
                pri: 2,
                team: Some(1),
                num_segments: 1,
                samples: vec![],
                gaps: vec![],
            },
        ],
        frames: vec![],
        resampled: Resampled {
            hz: 10.0,
            team_attack_sign: signs,
            frames,
        },
        events: vec![
            Event::Touch {
                t: 0.5,
                pri: 1,
                player: Some("Alice".into()),
                team: Some(0),
            },
            Event::Goal {
                t: 2.0,
                scorer: Some("Alice".into()),
                team: Some(0),
            },
        ],
        features: vec![],
    }
}

#[test]
fn attach_impact_sets_player_impact_and_touch_dv() {
    let m = match_with_touch();
    let mut scene = build_scene(&m, &[]);

    // Nothing attached yet.
    assert!(scene.players.iter().all(|p| p.impact.is_none()));
    assert!(scene.events.iter().all(|e| e.dv.is_none()));

    attach_impact(&mut scene, &m, &ValueConfig::default());

    // The toucher (Alice, pri 1) gets an impact total.
    let alice = scene.players.iter().find(|p| p.pri == 1).expect("alice");
    assert!(alice.impact.is_some(), "toucher gets an impact readout");
    assert!(alice.impact.unwrap().is_finite());

    // The touch event is tagged with a (finite) ΔV; the goal event is not.
    let touch = scene
        .events
        .iter()
        .find(|e| e.kind == "touch")
        .expect("touch event");
    assert!(touch.dv.is_some(), "touch event tagged with ΔV");
    assert!(touch.dv.unwrap().is_finite());
    let goal = scene
        .events
        .iter()
        .find(|e| e.kind == "goal")
        .expect("goal event");
    assert!(goal.dv.is_none(), "non-touch events carry no ΔV");
}

#[test]
fn empty_match_is_a_noop() {
    // No frames → attach_impact must not panic and must leave the scene untouched.
    let mut m = match_with_touch();
    m.resampled.frames.clear();
    let mut scene = build_scene(&m, &[]);
    attach_impact(&mut scene, &m, &ValueConfig::default());
    assert!(scene.players.iter().all(|p| p.impact.is_none()));
}
