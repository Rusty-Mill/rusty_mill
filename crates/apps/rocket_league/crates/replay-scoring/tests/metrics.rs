//! Behavior-isolating metric tests: hand-built frame sequences that exercise a
//! single metric and assert its raw value.

use replay_analyzer::model::{Event, Vec3};
use replay_scoring::config::{Metric, ScoreConfig};
use replay_scoring::features::{CarView, FrameView, Third};
use replay_scoring::{metrics, roles};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

#[allow(clippy::too_many_arguments)]
fn cv(pri: i32, ttb: f32, p: Vec3, pax: f32, goalside: bool, third: Third) -> CarView {
    CarView {
        pri,
        team: 0,
        p,
        v: v(0.0, 0.0, 0.0),
        pa: v(pax, 0.0, 17.0),
        boost: Some(50),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: ttb,
        goalside,
        ball_third: third,
    }
}

fn frame(t: f32, cars: Vec<CarView>) -> FrameView {
    FrameView {
        t,
        ball: Some(replay_analyzer::model::Kin {
            p: v(0.0, 0.0, 100.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars,
    }
}

fn raws_for(frames: &[FrameView], target: i32) -> std::collections::BTreeMap<Metric, Option<f32>> {
    let cfg = ScoreConfig::default();
    let roles = roles::assign(frames, &[0], &cfg);
    metrics::compute(frames, &roles, &[] as &[Event], target, 0, &cfg)
}

#[test]
fn double_commit_fires_when_both_press() {
    // Both cars pressuring (ttb < press_ttb=1.2) every frame -> rate 1.0.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 0.5, v(0.0, -100.0, 17.0), 0.0, true, Third::Mid),
                    cv(2, 0.6, v(0.0, 100.0, 17.0), 0.0, true, Third::Mid),
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::DoubleCommitRate], Some(1.0));
}

#[test]
fn central_support_high_when_target_supports_centrally() {
    // pri2 leads (1st man), pri1 is 2nd man sitting central + goal-side.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 1.5, v(0.0, -2000.0, 17.0), 300.0, true, Third::Mid), // 2nd man, central, goalside
                    cv(2, 0.5, v(0.0, 0.0, 17.0), 0.0, false, Third::Mid),      // 1st man
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::CentralSupportFraction], Some(1.0));
    // And it is NOT counted as a double-commit (only one car pressuring).
    assert_eq!(raws[&Metric::DoubleCommitRate], Some(0.0));
}

#[test]
fn goalside_first_tracks_defensive_goalside_share() {
    // Target is 1st man in the defensive third; goal-side in 7/10 frames.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            let goalside = i < 7;
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 0.5, v(0.0, -3000.0, 17.0), 0.0, goalside, Third::Def), // 1st man on defense
                    cv(2, 1.5, v(0.0, -1000.0, 17.0), 0.0, true, Third::Def),
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::GoalsideDiscipline1st], Some(0.7));
}

#[test]
fn possession_retention_from_touch_chains() {
    let cfg = ScoreConfig::default();
    let roles = roles::assign(&[], &[0], &cfg);
    // pri 1 (team 0) touches: kept then lost; one trailing touch by pri 1.
    let events = vec![
        Event::Touch {
            t: 0.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
        Event::Touch {
            t: 1.0,
            pri: 9,
            player: None,
            team: Some(0),
        }, // same team -> kept
        Event::Touch {
            t: 2.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
        Event::Touch {
            t: 3.0,
            pri: 7,
            player: None,
            team: Some(1),
        }, // other team -> lost
    ];
    let raws = metrics::compute(&[], &roles, &events, 1, 0, &cfg);
    // 2 target touches with a successor; 1 kept -> 0.5.
    assert_eq!(raws[&Metric::PossessionRetention], Some(0.5));
}
