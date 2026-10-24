//! **Physics runs** as a playback scene (ADR-0085): a recorded capture and,
//! optionally, the port's candidate run of it, on one timeline.
//!
//! Both arrive as [`rb_domain::PhysicsFrame`] lists (ADR-0005 capture files,
//! read by `rb_capture_ingest`), so the viewer never links the physics. Each
//! car becomes a [`SceneCar`]; candidate cars get negative `pri`s ([`pri`])
//! and a roster entry with [`ScenePlayer::ghost`] set,
//! which the page draws translucent over the recorded car. Frames stay at the
//! capture rate, and time is rebased so the first recorded frame is `t = 0`.

use std::collections::{BTreeMap, BTreeSet};

use rb_domain::{PhysicsFrame, Quat};

use crate::scene::{Field, Scene, SceneCar, SceneFrame, ScenePlayer};

/// A car's `pri`: its player id for a recorded car, `-1 - id` for a candidate
/// car. The two ranges cannot meet, whatever the ids. Fails for an id above
/// `i32::MAX`.
pub fn pri(player_id: u32, ghost: bool) -> Result<i32, String> {
    let id = i32::try_from(player_id)
        .map_err(|_| format!("player id {player_id} is too large for the viewer"))?;
    Ok(if ghost { -1 - id } else { id })
}

/// The team colour used for every car: a capture has no teams.
const CAPTURE_TEAM: i32 = 0;

/// Build the scene for a recorded run and an optional candidate run.
///
/// Candidate frames are matched to recorded frames by nearest timestamp
/// within half a recorded tick; a recorded frame with no match shows only the
/// recorded cars. Fails on an empty recording or a player id too large for a
/// `pri`.
pub fn build_physics_scene(
    id: &str,
    recorded: &[PhysicsFrame],
    sim: Option<&[PhysicsFrame]>,
) -> Result<Scene, String> {
    let (first, last) = match (recorded.first(), recorded.last()) {
        (Some(first), Some(last)) => (first.timestamp_secs, last.timestamp_secs),
        _ => return Err("the recording has no frames".into()),
    };
    let duration = last - first;
    let hz = if duration > 0.0 {
        (recorded.len() - 1) as f32 / duration
    } else {
        0.0
    };
    let tolerance = if hz > 0.0 { 0.5 / hz } else { f32::INFINITY };
    let sim = sim.unwrap_or(&[]);

    let mut frames = Vec::with_capacity(recorded.len());
    let mut cursor = 0;
    for frame in recorded {
        let mut cars = cars_of(frame, false)?;
        if let Some(matched) = nearest(sim, &mut cursor, frame.timestamp_secs, tolerance) {
            cars.extend(cars_of(matched, true)?);
        }
        frames.push(SceneFrame {
            t: round(frame.timestamp_secs - first, 3),
            ball: Some(pos(frame.ball.position)),
            cars,
        });
    }

    Ok(Scene {
        replay_id: format!("{id} (t = 0 at capture {first:.3} s)"),
        map: None,
        non_standard_map: false,
        hz: round(hz, 2),
        duration_s: round(duration, 3),
        team_scores: BTreeMap::new(),
        attack_sign: BTreeMap::new(),
        field: Field::default(),
        players: roster(recorded, sim)?,
        frames,
        events: Vec::new(),
        pad_pickups: Vec::new(),
        win_prob: Vec::new(),
        player_stats: Vec::new(),
        support_spacing_band: None,
    })
}

/// Rotation as the viewer's `[pitch, yaw, roll]` (rad): ZYX extraction, pitch
/// clamped at the poles, the same convention as the replay decoder.
pub fn quat_to_euler(q: Quat) -> [f32; 3] {
    let Quat { x, y, z, w } = q;
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let sinp = 2.0 * (w * y - z * x);
    let pitch = if sinp.abs() >= 1.0 {
        std::f32::consts::FRAC_PI_2.copysign(sinp)
    } else {
        sinp.asin()
    };
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    [pitch, yaw, roll]
}

/// The candidate frame closest to `t`, if one lies within `tolerance`.
/// Advances `*cursor` past frames too early for `t`, so a forward scan over
/// time-sorted frames stays linear.
fn nearest<'a>(
    sim: &'a [PhysicsFrame],
    cursor: &mut usize,
    t: f32,
    tolerance: f32,
) -> Option<&'a PhysicsFrame> {
    while *cursor < sim.len() && sim[*cursor].timestamp_secs < t - tolerance {
        *cursor += 1;
    }
    sim[*cursor..]
        .iter()
        .take_while(|f| f.timestamp_secs <= t + tolerance)
        .min_by(|a, b| {
            (a.timestamp_secs - t)
                .abs()
                .total_cmp(&(b.timestamp_secs - t).abs())
        })
}

fn cars_of(frame: &PhysicsFrame, ghost: bool) -> Result<Vec<SceneCar>, String> {
    frame
        .cars
        .iter()
        .map(|car| {
            Ok(SceneCar {
                pri: pri(car.player_id, ghost)?,
                p: pos(car.position),
                rot: quat_to_euler(car.rotation).map(|a| round(a, 3)),
                boost: car.boost_amount.clamp(0.0, 100.0).round() as u8,
                role: None,
            })
        })
        .collect()
}

/// One roster entry per player id seen in each run, recorded first.
fn roster(recorded: &[PhysicsFrame], sim: &[PhysicsFrame]) -> Result<Vec<ScenePlayer>, String> {
    let ids = |frames: &[PhysicsFrame]| -> BTreeSet<u32> {
        frames
            .iter()
            .flat_map(|f| f.cars.iter().map(|c| c.player_id))
            .collect()
    };
    let mut players = Vec::new();
    for (frames, ghost) in [(recorded, false), (sim, true)] {
        for player_id in ids(frames) {
            let name = if ghost {
                format!("car {player_id} (sim)")
            } else {
                format!("car {player_id}")
            };
            players.push(ScenePlayer {
                pri: pri(player_id, ghost)?,
                name,
                team: Some(CAPTURE_TEAM),
                goals: 0,
                assists: 0,
                saves: 0,
                impact: None,
                ghost,
            });
        }
    }
    Ok(players)
}

fn round(x: f32, places: i32) -> f32 {
    let f = 10f32.powi(places);
    (x * f).round() / f
}

fn pos(p: rb_domain::Vec3) -> [f32; 3] {
    [p.x.round(), p.y.round(), p.z.round()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    fn frame_at(t: f32) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: t,
            ball: rb_domain::BallState {
                position: rb_domain::Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: rb_domain::Vec3::ZERO,
                angular_velocity: rb_domain::Vec3::ZERO,
            },
            cars: Vec::new(),
        }
    }

    #[test]
    fn nearest_picks_the_closest_frame_in_the_window() {
        let sim = [frame_at(0.96), frame_at(1.01)];
        let mut cursor = 0;
        let hit = nearest(&sim, &mut cursor, 1.0, 0.05).map(|f| f.timestamp_secs);
        assert_eq!(hit, Some(1.01));
        assert_eq!(nearest(&sim, &mut cursor, 2.0, 0.05), None);
    }

    #[test]
    fn recorded_and_candidate_pris_never_meet() {
        assert_eq!(pri(0, false), Ok(0));
        assert_eq!(pri(0, true), Ok(-1));
        assert_eq!(pri(1_000_000, false), Ok(1_000_000));
        assert_ne!(pri(1_000_000, false), pri(0, true));
        assert_eq!(pri(i32::MAX as u32, true), Ok(i32::MIN));
        assert!(pri(u32::MAX, false).is_err());
    }

    #[test]
    fn identity_is_level() {
        assert!(close(quat_to_euler(Quat::IDENTITY), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn quarter_turn_about_z_is_yaw() {
        let h = FRAC_PI_2 / 2.0;
        let q = Quat::new(0.0, 0.0, h.sin(), h.cos());
        assert!(close(quat_to_euler(q), [0.0, FRAC_PI_2, 0.0]));
    }

    #[test]
    fn pitch_clamps_at_the_pole() {
        let h = FRAC_PI_2 / 2.0;
        let q = Quat::new(0.0, h.sin(), 0.0, h.cos());
        assert!((quat_to_euler(q)[0] - FRAC_PI_2).abs() < 1e-3);
    }
}
