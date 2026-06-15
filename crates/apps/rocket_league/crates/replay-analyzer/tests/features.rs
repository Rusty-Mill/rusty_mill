//! T5 feature tests.
//!
//! `external_validation`: the milestone's intended cross-check is against
//! ballchasing.com or carball for the same replay. Neither is available in this
//! environment — the ballchasing API requires a token (HTTP 401 without one) and
//! `carball` pins `pandas==1.0.3`/`numpy==1.14.5`, which fail to build on modern
//! Python. So feature *logic* is validated definitively here against
//! hand-computed expected values, and feature *plausibility* against internal
//! invariants on a real replay; the external numeric cross-check remains the
//! open item, gated on those credentials/installs.

use replay_analyzer::analyze::features::player_features;
use replay_analyzer::model::{
    Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, TrackGap, TrackSample, Vec3,
};
use std::collections::BTreeMap;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn sample(t: f32, boost: u8) -> TrackSample {
    TrackSample {
        t,
        actor_id: 1,
        p: v(0.0, 0.0, 17.0),
        v: v(0.0, 0.0, 0.0),
        boost: Some(boost),
        rot: None,
    }
}

/// Track for player pri=1: boost 255->155 (uses 100 bytes), +45 pickup
/// (ignored), a respawn gap, then 200->85 across the gap (skipped), then
/// 85->51 (uses 34 bytes). Total used = 134 bytes.
fn track() -> PlayerTrack {
    PlayerTrack {
        player: "P".into(),
        pri: 1,
        team: Some(0),
        num_segments: 2,
        samples: vec![
            sample(0.0, 255),
            sample(0.5, 155),
            sample(1.0, 200),
            sample(2.0, 85),
            sample(3.0, 51),
        ],
        gaps: vec![TrackGap {
            start: 1.0,
            end: 2.0,
            reason: replay_analyzer::model::GapReason::Respawn,
        }],
    }
}

fn gcar(pri: i32, p: Vec3, speed_x: f32) -> GridCar {
    GridCar {
        pri,
        team: Some(0),
        p,
        v: v(speed_x, 0.0, 0.0),
        boost: Some(50),
        rot: None,
    }
}

/// 10 Hz grid: 3 supersonic frames (0.3 s); ball present in 2 frames at
/// distances 100 and 300 (mean 200).
fn resampled() -> Resampled {
    let frames = vec![
        GridFrame {
            t: 0.0,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![gcar(1, v(100.0, 0.0, 100.0), 2300.0)],
        },
        GridFrame {
            t: 0.1,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![gcar(1, v(300.0, 0.0, 100.0), 2300.0)],
        },
        GridFrame {
            t: 0.2,
            ball: None,
            cars: vec![gcar(1, v(0.0, 0.0, 100.0), 2300.0)],
        },
        GridFrame {
            t: 0.3,
            ball: None,
            cars: vec![gcar(1, v(0.0, 0.0, 100.0), 1000.0)],
        },
    ];
    Resampled {
        hz: 10.0,
        team_attack_sign: BTreeMap::new(),
        frames,
    }
}

fn events() -> Vec<Event> {
    vec![
        Event::Touch {
            t: 0.0,
            pri: 1,
            player: Some("P".into()),
            team: Some(0),
        },
        Event::Touch {
            t: 1.0,
            pri: 1,
            player: Some("P".into()),
            team: Some(0),
        },
        Event::Possession {
            team: 0,
            start: 0.0,
            end: 5.0,
            touches: 2,
        },
    ]
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps
}

#[test]
fn features_match_hand_computed_values() {
    let f = player_features(&resampled(), &[track()], &events());
    assert_eq!(f.len(), 1);
    let p = &f[0];

    assert_eq!(p.touches, 2);
    // 134 bytes used -> 134/255*100 = 52.55 boost units.
    assert!(
        near(p.boost_used, 52.55, 0.1),
        "boost_used = {}",
        p.boost_used
    );
    // 3 supersonic frames * 0.1 s.
    assert!(
        near(p.time_supersonic_s, 0.3, 1e-4),
        "supersonic = {}",
        p.time_supersonic_s
    );
    // distances 100 and 300 -> mean 200 (3rd frame has no ball).
    assert!(
        near(p.mean_dist_to_ball, 200.0, 0.5),
        "mean_dist = {}",
        p.mean_dist_to_ball
    );
    // team 0 possession 0..5.
    assert!(
        near(p.possession_time_s, 5.0, 1e-4),
        "possession = {}",
        p.possession_time_s
    );
}

#[test]
fn boost_used_skips_respawn_gap() {
    // If the across-gap decrease (200 -> 85) were wrongly counted, boost_used
    // would include 115 extra bytes (~45 units). Assert it does not.
    let f = player_features(&resampled(), &[track()], &events());
    assert!(
        f[0].boost_used < 60.0,
        "gap decrease must not count: {}",
        f[0].boost_used
    );
}
