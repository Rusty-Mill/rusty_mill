//! Featurizer behavior: correct values from a hand-built frame and exact
//! team symmetry (the two perspectives mirror each other).

use replay_analyzer::model::{Event, GridCar, GridFrame, Kin, Vec3};
use replay_value::features::{features_at, state_features, N_FEATURES};
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
fn v2_features_swap_between_perspectives() {
    // A 2v1 frame with motion + orientation so the v2 columns are non-trivial.
    // The new per-car features are frame-invariant scalars, so one team's
    // attacker view must equal the other team's defender view (and man-advantage
    // negates). Indices: 10 man_adv, 11 goal_dist, 12/13 att/def closing,
    // 14/15 att/def facing, 16/17 att/def max-boost.
    let signs = BTreeMap::from([(0, 1), (1, -1)]);
    let cv = |pri, team, p, vel, boost, yaw: f32| GridCar {
        pri,
        team: Some(team),
        p,
        v: vel,
        boost: Some(boost),
        rot: Some(replay_analyzer::model::Rot3 {
            pitch: 0.0,
            yaw,
            roll: 0.0,
        }),
    };
    let frame = GridFrame {
        t: 1.0,
        ball: Some(Kin {
            p: v(0.0, 1000.0, 100.0),
            v: v(0.0, 500.0, 0.0),
        }),
        cars: vec![
            cv(
                1,
                0,
                v(0.0, 500.0, 17.0),
                v(0.0, 800.0, 0.0),
                200,
                std::f32::consts::FRAC_PI_2,
            ),
            cv(2, 0, v(-1000.0, 0.0, 17.0), v(0.0, 0.0, 0.0), 100, 0.0),
            cv(
                3,
                1,
                v(0.0, 2000.0, 17.0),
                v(0.0, -300.0, 0.0),
                50,
                -std::f32::consts::FRAC_PI_2,
            ),
        ],
    };
    let s0 = state_features(&frame, 0, &signs).unwrap();
    let s1 = state_features(&frame, 1, &signs).unwrap();
    let eq = |a: f32, b: f32| (a - b).abs() < 1e-4;

    // Man-advantage: team 0 is +1 up (2 vs 1), team 1 mirrors it.
    assert!(eq(s0.x[10], 1.0 / 3.0) && eq(s1.x[10], -1.0 / 3.0));
    // Shot proximity is a real distance in [0,1] for both.
    assert!((0.0..=1.0).contains(&s0.x[11]) && (0.0..=1.0).contains(&s1.x[11]));
    // Closing / facing / max-boost swap attacker↔defender across perspectives.
    assert!(eq(s0.x[12], s1.x[13]) && eq(s0.x[13], s1.x[12]));
    assert!(eq(s0.x[14], s1.x[15]) && eq(s0.x[15], s1.x[14]));
    assert!(eq(s0.x[16], s1.x[17]) && eq(s0.x[17], s1.x[16]));
    // The nearest attacker is driving onto the ball and facing it.
    assert!(s0.x[12] > 0.0, "closing toward ball positive: {}", s0.x[12]);
    assert!(s0.x[14] > 0.9, "facing the ball: {}", s0.x[14]);
    // All v2 columns finite.
    assert!(s0.x.iter().all(|f| f.is_finite()) && s1.x.iter().all(|f| f.is_finite()));
}

#[test]
fn features_at_temporal_and_carry_are_correct_and_symmetric() {
    let signs = BTreeMap::from([(0, 1), (1, -1)]);
    // A team-0 car carrying the ball on its roof at +1000 Y; a team-1 car far away.
    // Two frames 0.4 s apart so the trend look-back is exactly one frame; the ball's
    // +Y velocity jumps 0 → 2000 between them.
    let mk = |t: f32, vy: f32| GridFrame {
        t,
        ball: Some(Kin {
            p: v(0.0, 1000.0, 180.0),
            v: v(0.0, vy, 0.0),
        }),
        cars: vec![
            car(1, 0, v(0.0, 1000.0, 17.0), 50), // under the ball → carrying
            car(2, 1, v(0.0, -1000.0, 17.0), 50),
        ],
    };
    let frames = vec![mk(0.0, 0.0), mk(0.4, 2000.0)];
    // Team 0 touched at t=0.3; team 1 never.
    let events = vec![Event::Touch {
        t: 0.3,
        player: None,
        team: Some(0),
        pri: 1,
    }];

    let s0 = features_at(&frames, 1, 0, &signs, &events).unwrap();
    let s1 = features_at(&frames, 1, 1, &signs, &events).unwrap();
    let eq = |a: f32, b: f32| (a - b).abs() < 1e-4;

    // [18] momentum: (2000 − 0) / SPEED_SCALE(6000) ≈ +0.333; mirrors (negates) for
    // team 1, since the ball heads toward team 0's goal (away from team 1's).
    assert!(eq(s0.x[18], 2000.0 / 6000.0) && eq(s1.x[18], -2000.0 / 6000.0));
    // [19] recency: team 0 touched 0.1 s ago → 0.01; team 1 never → 1.0.
    assert!(eq(s0.x[19], 0.01) && eq(s1.x[19], 1.0));
    // [20] att-carry / [21] def-carry: team 0 is carrying, so it swaps across views.
    assert!(eq(s0.x[20], 1.0) && eq(s0.x[21], 0.0));
    assert!(eq(s1.x[20], 0.0) && eq(s1.x[21], 1.0));
    assert!(eq(s0.x[20], s1.x[21]) && eq(s0.x[21], s1.x[20]));
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
