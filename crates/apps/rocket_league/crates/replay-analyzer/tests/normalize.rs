//! Unit tests for T2 attack-direction normalization: sign detection from kickoff
//! geometry, the flip transform, and producing a team's attacking-direction frame
//! where both teams become comparable (each attacks +Y).

use replay_analyzer::analyze::normalize::{attacking_frame, flip_xy, team_attack_sign};
use replay_analyzer::model::{
    CarState, FrameOut, GridCar, GridFrame, Kin, PlayerTrack, TrackGap, TrackSample, Vec3,
};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn track(pri: i32, team: i32) -> PlayerTrack {
    PlayerTrack {
        player: format!("p{pri}"),
        pri,
        team: Some(team),
        num_segments: 1,
        samples: vec![TrackSample {
            t: 0.0,
            actor_id: pri,
            p: v(0.0, 0.0, 17.0),
            v: v(0.0, 0.0, 0.0),
        }],
        gaps: Vec::<TrackGap>::new(),
    }
}

fn car(pri: i32, p: Vec3) -> CarState {
    CarState {
        actor_id: pri,
        pri,
        player: Some(format!("p{pri}")),
        p,
        v: v(0.0, 0.0, 0.0),
    }
}

/// A kickoff: ball centered, team 0 spawned on -Y, team 1 on +Y, both on
/// canonical spawn coordinates at ground level.
fn kickoff_frame() -> FrameOut {
    FrameOut {
        t: 10.0,
        ball: Some(v(0.0, 0.0, 93.0)),
        cars: vec![
            car(10, v(0.0, -4608.0, 17.0)), // team 0, far-back spawn
            car(20, v(0.0, 4608.0, 17.0)),  // team 1, mirrored
        ],
    }
}

#[test]
fn attack_sign_detected_from_kickoff_halves() {
    let frames = vec![kickoff_frame()];
    let tracks = vec![track(10, 0), track(20, 1)];
    let signs = team_attack_sign(&frames, &tracks);

    // Team 0 defends -Y (already attacks +Y) -> +1; team 1 mirrored -> -1.
    assert_eq!(signs.get(&0), Some(&1));
    assert_eq!(signs.get(&1), Some(&-1));
}

#[test]
fn attack_sign_falls_back_to_convention_without_kickoff() {
    // No kickoff frame in the stream.
    let frames: Vec<FrameOut> = Vec::new();
    let tracks = vec![track(10, 0), track(20, 1)];
    let signs = team_attack_sign(&frames, &tracks);
    assert_eq!(signs.get(&0), Some(&1));
    assert_eq!(signs.get(&1), Some(&-1));
}

#[test]
fn flip_xy_is_a_180_degree_z_rotation() {
    assert_eq!(flip_xy(v(1.0, 2.0, 3.0), 1), v(1.0, 2.0, 3.0));
    assert_eq!(flip_xy(v(1.0, 2.0, 3.0), -1), v(-1.0, -2.0, 3.0));
}

#[test]
fn attacking_frame_makes_both_teams_attack_plus_y() {
    let signs = team_attack_sign(&[kickoff_frame()], &[track(10, 0), track(20, 1)]);

    // Grid frame mirroring the kickoff, with a ball drifting toward +Y for team 0.
    let grid = GridFrame {
        t: 10.0,
        ball: Some(Kin {
            p: v(0.0, 1000.0, 93.0),
            v: v(0.0, 500.0, 0.0),
        }),
        cars: vec![
            GridCar { pri: 10, team: Some(0), p: v(0.0, -4608.0, 17.0), v: v(0.0, 0.0, 0.0) },
            GridCar { pri: 20, team: Some(1), p: v(0.0, 4608.0, 17.0), v: v(0.0, 0.0, 0.0) },
        ],
    };

    // In team 0's frame (sign +1): unchanged.
    let f0 = attacking_frame(&grid, 0, &signs);
    assert_eq!(f0.ball.unwrap().p, v(0.0, 1000.0, 93.0));
    assert_eq!(f0.cars[0].p, v(0.0, -4608.0, 17.0));

    // In team 1's frame (sign -1): whole world rotated 180°. Team 1's own car,
    // at world +Y, now sits on its own -Y half (it attacks +Y), and the ball
    // that was advancing for team 0 now recedes for team 1.
    let f1 = attacking_frame(&grid, 1, &signs);
    assert_eq!(f1.cars[1].p, v(0.0, -4608.0, 17.0));
    assert_eq!(f1.ball.unwrap().p, v(0.0, -1000.0, 93.0));
    assert_eq!(f1.ball.unwrap().v, v(0.0, -500.0, 0.0));

    // Comparability: each team's own kickoff car is on the -Y half in its own
    // attacking frame.
    let own0 = flip_xy(v(0.0, -4608.0, 17.0), *signs.get(&0).unwrap());
    let own1 = flip_xy(v(0.0, 4608.0, 17.0), *signs.get(&1).unwrap());
    assert!(own0.y < 0.0 && own1.y < 0.0, "both own spawns on -Y after normalization");
}
