//! Featurizer behavior: correct values from a hand-built frame and exact
//! team symmetry (the two perspectives mirror each other).

use replay_analyzer::model::{GridCar, GridFrame, Kin, Vec3};
use replay_value::features::{state_features, N_FEATURES};
use std::collections::BTreeMap;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn car(pri: i32, team: i32, p: Vec3, boost: u8) -> GridCar {
    GridCar {
        pri,
        team: Some(team),
        p,
        v: v(0.0, 0.0, 0.0),
        boost: Some(boost),
        rot: None,
    }
}

#[test]
fn featurizer_is_team_symmetric_and_correct() {
    // Team 0 attacks +Y, team 1 attacks -Y. Ball at +1000 Y; team-0 car behind
    // it (full boost), team-1 car ahead of it (no boost).
    let mut signs = BTreeMap::new();
    signs.insert(0, 1);
    signs.insert(1, -1);
    let frame = GridFrame {
        t: 1.0,
        ball: Some(Kin {
            p: v(0.0, 1000.0, 100.0),
            v: v(0.0, 500.0, 0.0),
        }),
        cars: vec![
            car(1, 0, v(0.0, 500.0, 17.0), 255),
            car(2, 1, v(0.0, 2000.0, 17.0), 0),
        ],
    };

    let s0 = state_features(&frame, 0, &signs).expect("team0 features");
    let s1 = state_features(&frame, 1, &signs).expect("team1 features");
    assert_eq!(s0.x.len(), N_FEATURES);

    // ball_y: +0.195 toward team-0's goal, mirrored negative for team 1.
    assert!((s0.x[0] - 1000.0 / 5120.0).abs() < 1e-4, "{}", s0.x[0]);
    assert!((s1.x[0] + 1000.0 / 5120.0).abs() < 1e-4, "{}", s1.x[0]);

    // No attacker is ahead of the ball in either perspective.
    assert_eq!(s0.x[7], 0.0);
    assert_eq!(s1.x[7], 0.0);

    // Exactly one defender is goal-side of the ball in both perspectives.
    assert_eq!(s0.x[8], 1.0);
    assert_eq!(s1.x[8], 1.0);

    // Boost differential: team 0 holds all the boost (+1), team 1 none (−1).
    assert!((s0.x[9] - 1.0).abs() < 1e-6);
    assert!((s1.x[9] + 1.0).abs() < 1e-6);

    // ball_z and ball_speed are perspective-invariant (z and magnitude).
    assert!((s0.x[2] - s1.x[2]).abs() < 1e-6);
    assert!((s0.x[4] - s1.x[4]).abs() < 1e-6);
}

#[test]
fn featurizer_needs_ball_and_own_car() {
    let signs = BTreeMap::from([(0, 1)]);
    // No ball -> no features.
    let no_ball = GridFrame {
        t: 0.0,
        ball: None,
        cars: vec![car(1, 0, v(0.0, 0.0, 17.0), 100)],
    };
    assert!(state_features(&no_ball, 0, &signs).is_none());
    // Ball present but no live car for the team -> no features.
    let no_car = GridFrame {
        t: 0.0,
        ball: Some(Kin {
            p: v(0.0, 0.0, 100.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars: vec![car(2, 1, v(0.0, 0.0, 17.0), 100)],
    };
    assert!(state_features(&no_car, 0, &signs).is_none());
}
