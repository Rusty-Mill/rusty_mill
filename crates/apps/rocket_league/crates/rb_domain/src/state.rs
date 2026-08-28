//! Physics state types shared by every ingestion adapter and the divergence
//! scorer. Field set matches what the GDC 2018 talk and replay/BakkesMod
//! data actually expose: position, rotation, velocity, angular velocity,
//! and (for cars) boost amount. See `RB-VERIFY-001`/`RB-VERIFY-002`.

/// A 3D vector. Not `nalgebra`/`glam` on purpose: the domain crate has zero
/// dependencies until a second real numeric need justifies pulling one in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn distance(&self, other: &Vec3) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// A rotation expressed as a quaternion (x, y, z, w). Replays and BakkesMod
/// both expose rotation this way; converting to Euler angles is a
/// presentation concern, not a domain one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallState {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarState {
    pub player_id: u32,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub boost_amount: f32,
}

/// One simulation tick's worth of authoritative-or-recorded state.
///
/// `timestamp_secs` is seconds since the start of the capture/replay, not a
/// wall-clock time — divergence comparisons only ever need relative offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsFrame {
    pub timestamp_secs: f32,
    pub ball: BallState,
    pub cars: Vec<CarState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_between_identical_points_is_zero() {
        let a = Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        assert_eq!(a.distance(&a), 0.0);
    }

    #[test]
    fn distance_matches_pythagorean_expectation() {
        let a = Vec3::ZERO;
        let b = Vec3 {
            x: 3.0,
            y: 4.0,
            z: 0.0,
        };
        assert_eq!(a.distance(&b), 5.0);
    }
}
