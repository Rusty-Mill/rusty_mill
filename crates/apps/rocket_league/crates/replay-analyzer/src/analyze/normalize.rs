//! Attack-direction normalization (T2).
//!
//! Teams defend opposite goals, so raw `+Y`/`-Y` and left/right are not directly
//! comparable across teams. Normalization rotates each team's frame so it always
//! **attacks `+Y`**: a team already attacking `+Y` is left as-is; a team attacking
//! `-Y` is rotated 180° about the vertical axis (`x,y -> -x,-y`, `z` unchanged).
//! Velocities transform identically.
//!
//! The per-team sign is derived from kickoff geometry: at kickoff each car sits
//! on its own (defensive) half, so the sign of a team's mean `y` reveals which
//! goal it defends — and therefore which way it attacks.

use crate::model::{FrameOut, GridCar, GridFrame, Kin, PlayerTrack, Vec3};
use crate::field;
use std::collections::BTreeMap;

/// Multiplier that rotates a team's world frame so it attacks `+Y`.
///
/// `+1` leaves coordinates unchanged; `-1` negates `x` and `y` (180° z-rotation).
pub fn flip_xy(v: Vec3, sign: i32) -> Vec3 {
    if sign >= 0 {
        v
    } else {
        Vec3 {
            x: -v.x,
            y: -v.y,
            z: v.z,
        }
    }
}

/// Determine, for each team that has a track, the sign that makes it attack `+Y`.
///
/// Resolved from the first detectable kickoff (each team's mean `y` is on its
/// defensive half). Falls back to the standard convention (team 0 attacks `+Y`,
/// others `-Y`) for any team absent from the kickoff frame.
pub fn team_attack_sign(frames: &[FrameOut], tracks: &[PlayerTrack]) -> BTreeMap<i32, i32> {
    let pri_team: BTreeMap<i32, i32> = tracks
        .iter()
        .filter_map(|t| t.team.map(|tm| (t.pri, tm)))
        .collect();

    let mut signs = BTreeMap::new();

    if let Some(frame) = find_kickoff(frames) {
        // Sum y per team across cars present at kickoff.
        let mut sum: BTreeMap<i32, (f64, u32)> = BTreeMap::new();
        for c in &frame.cars {
            if let Some(team) = pri_team.get(&c.pri) {
                let e = sum.entry(*team).or_insert((0.0, 0));
                e.0 += c.p.y as f64;
                e.1 += 1;
            }
        }
        for (team, (ysum, n)) in sum {
            if n > 0 {
                let mean_y = ysum / n as f64;
                // Defends -Y (mean_y < 0) => already attacks +Y => +1.
                signs.insert(team, if mean_y <= 0.0 { 1 } else { -1 });
            }
        }
    }

    // Ensure every team with a track has a sign (convention fallback).
    for team in tracks.iter().filter_map(|t| t.team) {
        signs.entry(team).or_insert(if team == 0 { 1 } else { -1 });
    }

    signs
}

/// Find the first kickoff frame: ball resting at field center with at least two
/// live cars, all seated on canonical kickoff spawns at ground level.
///
/// Mid-play frames where the ball merely passes near center fail the
/// all-cars-on-spawns requirement, so this isolates a genuine kickoff.
pub fn find_kickoff(frames: &[FrameOut]) -> Option<&FrameOut> {
    frames.iter().find(|f| {
        let Some(b) = &f.ball else { return false };
        b.x.abs() < 6.0
            && b.y.abs() < 6.0
            && (85.0..100.0).contains(&b.z)
            && f.cars.len() >= 2
            && f
                .cars
                .iter()
                .all(|c| c.p.z < 60.0 && field::is_kickoff_spawn(c.p.to_arr(), 60.0))
    })
}

/// Produce a grid frame expressed in `team`'s attacking-direction frame: the
/// whole world (ball + every car) rotated so `team` attacks `+Y`, making
/// own/opponent half and left/right comparable across teams.
pub fn attacking_frame(
    frame: &GridFrame,
    team: i32,
    signs: &BTreeMap<i32, i32>,
) -> GridFrame {
    let sign = signs.get(&team).copied().unwrap_or(1);
    GridFrame {
        t: frame.t,
        ball: frame.ball.map(|b| Kin {
            p: flip_xy(b.p, sign),
            v: flip_xy(b.v, sign),
        }),
        cars: frame
            .cars
            .iter()
            .map(|c| GridCar {
                pri: c.pri,
                team: c.team,
                p: flip_xy(c.p, sign),
                v: flip_xy(c.v, sign),
            })
            .collect(),
    }
}
