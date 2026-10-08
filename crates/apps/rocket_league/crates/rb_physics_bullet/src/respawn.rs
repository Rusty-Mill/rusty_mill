//! Demolition respawn (`RB-PHYSICS-001-FR-151`, ADR-0075).
//!
//! Measured on 12 demolitions of the game (`gen_demolitions.py`,
//! `respawn_report.py`, RB-RESEARCH-O018):
//!
//! - the victim is gone for exactly 3.000 s (360 ticks);
//! - it comes back at one of the ten kickoff spawn points, at rest, facing the
//!   point's heading; the first frame back shows it at z = 83 with no boost, the next
//!   at z = 36 with 33.33 boost, falling from rest (-5.4 uu/s that tick);
//! - **which** point is random: the same tape gave three different points in three
//!   runs, and the point does not follow the victim's position or team. So the port
//!   takes the pick as an input (`PhysicsWorld::set_respawn_point`) and, when it is
//!   not given, uses [`default_pick`], a deterministic stand-in.

use rb_domain::{Quat, Vec3};

/// Seconds a demolished car is out of the match.
pub const RESPAWN_DELAY_SECS: f32 = 3.0;
/// Height of the first frame back (uu).
pub const SPAWN_RAW_HEIGHT: f32 = 83.0;
/// Height of the car from the next frame on (uu); it falls from here.
pub const RESPAWN_HEIGHT: f32 = 36.0;
/// Boost of a respawned car from the next frame on (a third of a tank).
pub const RESPAWN_BOOST: f32 = 100.0 / 3.0;

/// A kickoff spawn point: a place on the field and the heading a car has there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnPoint {
    pub x: f32,
    pub y: f32,
    /// Heading in degrees about +z, from the +x axis.
    pub yaw_degrees: f32,
}

impl SpawnPoint {
    /// The car's origin `height` uu up from the floor at this point.
    pub fn position(&self, height: f32) -> Vec3 {
        Vec3::new(self.x, self.y, height)
    }

    /// The car's orientation: level, turned to the point's heading.
    pub fn rotation(&self) -> Quat {
        let half = self.yaw_degrees.to_radians() * 0.5;
        Quat::new(0.0, 0.0, half.sin(), half.cos())
    }
}

/// Soccar's ten kickoff spawn points, blue then orange.
pub const SPAWN_POINTS: [SpawnPoint; 10] = [
    SpawnPoint {
        x: -2048.0,
        y: -2560.0,
        yaw_degrees: 45.0,
    },
    SpawnPoint {
        x: 2048.0,
        y: -2560.0,
        yaw_degrees: 135.0,
    },
    SpawnPoint {
        x: -256.0,
        y: -3840.0,
        yaw_degrees: 90.0,
    },
    SpawnPoint {
        x: 256.0,
        y: -3840.0,
        yaw_degrees: 90.0,
    },
    SpawnPoint {
        x: 0.0,
        y: -4608.0,
        yaw_degrees: 90.0,
    },
    SpawnPoint {
        x: 2048.0,
        y: 2560.0,
        yaw_degrees: -135.0,
    },
    SpawnPoint {
        x: -2048.0,
        y: 2560.0,
        yaw_degrees: -45.0,
    },
    SpawnPoint {
        x: 256.0,
        y: 3840.0,
        yaw_degrees: -90.0,
    },
    SpawnPoint {
        x: -256.0,
        y: 3840.0,
        yaw_degrees: -90.0,
    },
    SpawnPoint {
        x: 0.0,
        y: 4608.0,
        yaw_degrees: -90.0,
    },
];

/// The spawn point nearest `(x, y)`: the pick a recorded respawn corresponds to.
pub fn nearest_spawn_point(x: f32, y: f32) -> usize {
    let mut best = 0;
    let mut best_distance = f32::INFINITY;
    for (index, point) in SPAWN_POINTS.iter().enumerate() {
        let distance = (point.x - x).hypot(point.y - y);
        if distance < best_distance {
            best = index;
            best_distance = distance;
        }
    }
    best
}

/// The spawn point a respawn uses when the caller gave none: a deterministic stand-in
/// for the game's random pick, from the tick and the car (splitmix-style mixing, so
/// consecutive ticks and cars spread over all ten points).
pub fn default_pick(tick: u64, car: usize) -> usize {
    let mut z = tick
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((car as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z % SPAWN_POINTS.len() as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_ten_points_mirrored_between_the_teams() {
        assert_eq!(SPAWN_POINTS.len(), 10);
        let (blue, orange) = SPAWN_POINTS.split_at(5);
        for point in blue {
            let mirrored = orange
                .iter()
                .any(|other| other.x == -point.x && other.y == -point.y);
            assert!(mirrored, "{point:?} has no mirror image");
        }
    }

    #[test]
    fn a_point_faces_its_heading_level() {
        let rotation = SPAWN_POINTS[2].rotation();
        let nose = rotation.rotate(&Vec3::new(1.0, 0.0, 0.0));
        assert!(nose.x.abs() < 1e-5 && (nose.y - 1.0).abs() < 1e-5 && nose.z.abs() < 1e-5);
        assert_eq!(
            SPAWN_POINTS[0].position(36.0),
            Vec3::new(-2048.0, -2560.0, 36.0)
        );
    }

    #[test]
    fn the_nearest_point_of_a_recorded_respawn_is_found() {
        for (index, point) in SPAWN_POINTS.iter().enumerate() {
            assert_eq!(nearest_spawn_point(point.x, point.y), index);
            assert_eq!(nearest_spawn_point(point.x + 30.0, point.y - 20.0), index);
        }
    }

    #[test]
    fn the_default_pick_is_deterministic_in_range_and_spreads_over_the_points() {
        assert_eq!(default_pick(1234, 1), default_pick(1234, 1));
        let mut seen = [false; 10];
        for tick in 0..400 {
            let pick = default_pick(tick, 1);
            assert!(pick < 10);
            seen[pick] = true;
        }
        assert!(
            seen.iter().all(|s| *s),
            "all ten points get picked: {seen:?}"
        );
    }
}
