//! **Physics runs** as a playback scene (ADR-0085): a recorded capture and,
//! optionally, the port's candidate run of it, on one timeline.
//!
//! Both arrive as [`rb_domain::PhysicsFrame`] lists (ADR-0005 capture files,
//! read by `rb_capture_ingest`), so the viewer never links the physics. Each
//! car becomes a [`SceneCar`]; candidate cars get their own `pri` range
//! ([`SIM_PRI_OFFSET`]) and a roster entry with [`ScenePlayer::ghost`] set,
//! which the page draws translucent over the recorded car. Frames stay at the
//! capture rate, and time is rebased so the first recorded frame is `t = 0`.

use std::collections::{BTreeMap, BTreeSet};

use rb_domain::{PhysicsFrame, Quat};

use crate::scene::{Field, Scene, SceneCar, SceneFrame, ScenePlayer};

/// Added to a candidate car's player id to form its `pri`, keeping it apart
/// from the recorded car with the same id.
pub const SIM_PRI_OFFSET: i32 = 1_000_000;

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
        let mut cars = cars_of(frame, 0)?;
        if let Some(matched) = nearest(sim, &mut cursor, frame.timestamp_secs, tolerance) {
            cars.extend(cars_of(matched, SIM_PRI_OFFSET)?);
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

/// The first candidate frame at or after `*cursor` within `tolerance` of `t`;
/// advances `*cursor` so a forward scan over sorted frames stays linear.
fn nearest<'a>(
    sim: &'a [PhysicsFrame],
    cursor: &mut usize,
    t: f32,
    tolerance: f32,
) -> Option<&'a PhysicsFrame> {
    while *cursor < sim.len() && sim[*cursor].timestamp_secs < t - tolerance {
        *cursor += 1;
    }
    sim.get(*cursor)
        .filter(|f| (f.timestamp_secs - t).abs() <= tolerance)
}

fn cars_of(frame: &PhysicsFrame, offset: i32) -> Result<Vec<SceneCar>, String> {
    frame
        .cars
        .iter()
        .map(|car| {
            Ok(SceneCar {
                pri: pri(car.player_id, offset)?,
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
    for (frames, offset, ghost) in [(recorded, 0, false), (sim, SIM_PRI_OFFSET, true)] {
        for player_id in ids(frames) {
            let name = if ghost {
                format!("car {player_id} (sim)")
            } else {
                format!("car {player_id}")
            };
            players.push(ScenePlayer {
                pri: pri(player_id, offset)?,
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

fn pri(player_id: u32, offset: i32) -> Result<i32, String> {
    i32::try_from(player_id)
        .ok()
        .and_then(|id| id.checked_add(offset))
        .ok_or_else(|| format!("player id {player_id} is too large for the viewer"))
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
