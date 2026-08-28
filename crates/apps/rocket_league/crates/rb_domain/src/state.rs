//! Physics state types shared by every ingestion adapter and the divergence
//! scorer. Field set matches what the GDC 2018 talk and replay/BakkesMod
//! data actually expose: position, rotation, velocity, angular velocity,
//! and (for cars) boost amount. See `RB-VERIFY-001`/`RB-VERIFY-002`.

/// A 3D vector. Not `nalgebra`/`glam` on purpose: the domain crate has zero
/// dependencies until a second real numeric need justifies pulling one in.
///
/// The arithmetic below (`dot`/`cross`/`normalize`/operator overloads) is
/// the vector algebra `rb_physics_bullet` needs to port Bullet3's rigid-body
/// integration and contact-solving math. It lives here rather than
/// duplicated in the physics crate because a second consumer (divergence
/// scoring already uses `distance`) is exactly the "two real call sites"
/// bar for adding shared logic to the domain crate.
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

    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn distance(&self, other: &Vec3) -> f32 {
        (*self - *other).length()
    }

    pub fn dot(&self, other: &Vec3) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(&self, other: &Vec3) -> Vec3 {
        Vec3 {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn length_squared(&self) -> f32 {
        self.dot(self)
    }

    pub fn length(&self) -> f32 {
        self.length_squared().sqrt()
    }

    /// `None` for a vector too short to have a meaningful direction,
    /// matching Bullet's `SIMD_EPSILON`-guarded normalize calls (e.g.
    /// `btVector3::safeNormalize`) rather than dividing by ~zero.
    pub fn normalize(&self) -> Option<Vec3> {
        let len = self.length();
        if len < 1e-6 {
            None
        } else {
            Some(*self * (1.0 / len))
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl std::ops::Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3 {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, rhs: f32) -> Vec3 {
        Vec3 {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

impl std::ops::AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Vec3) {
        *self = *self + rhs;
    }
}

impl std::ops::SubAssign for Vec3 {
    fn sub_assign(&mut self, rhs: Vec3) {
        *self = *self - rhs;
    }
}

impl std::ops::MulAssign<f32> for Vec3 {
    fn mul_assign(&mut self, rhs: f32) {
        *self = *self * rhs;
    }
}

/// A rotation expressed as a quaternion (x, y, z, w). Replays and BakkesMod
/// both expose rotation this way; converting to Euler angles is a
/// presentation concern, not a domain one.
///
/// `mul` and `rotate` port `btQuaternion::operator*` and
/// `quatRotate`/`btQuaternion::operator*(vector)` from
/// `bullet3/src/LinearMath/btQuaternion.h`, needed by
/// `rb_physics_bullet::integrate_transform` (the exponential-map orientation
/// update Bullet uses in `btTransformUtil::integrateTransform`).
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

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Quat {
        Quat { x, y, z, w }
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w
    }

    /// Falls back to `IDENTITY` for a near-zero quaternion, matching
    /// `btQuaternion::safeNormalize`'s guard rather than dividing by ~zero —
    /// this only happens from degenerate input, never in a healthy sim step.
    pub fn normalize(&self) -> Quat {
        let len = self.length_squared().sqrt();
        if len < 1e-6 {
            Quat::IDENTITY
        } else {
            let inv = 1.0 / len;
            Quat {
                x: self.x * inv,
                y: self.y * inv,
                z: self.z * inv,
                w: self.w * inv,
            }
        }
    }

    /// Hamilton product, `self * rhs` — matches `btQuaternion::operator*`.
    pub fn mul(&self, rhs: &Quat) -> Quat {
        Quat {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y + self.y * rhs.w + self.z * rhs.x - self.x * rhs.z,
            z: self.w * rhs.z + self.z * rhs.w + self.x * rhs.y - self.y * rhs.x,
        }
    }

    /// Rotates `v` by this quaternion — matches Bullet's `quatRotate`
    /// (`btQuaternion * btVector3 * btQuaternion::inverse()`, expanded).
    pub fn rotate(&self, v: &Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let uv = q.cross(v);
        let uuv = q.cross(&uv);
        *v + (uv * self.w + uuv) * 2.0
    }
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn distance_between_identical_points_is_zero() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(a.distance(&a), 0.0);
    }

    #[test]
    fn distance_matches_pythagorean_expectation() {
        let a = Vec3::ZERO;
        let b = Vec3::new(3.0, 4.0, 0.0);
        assert_eq!(a.distance(&b), 5.0);
    }

    #[test]
    fn cross_product_of_orthonormal_axes_is_third_axis() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(x.cross(&y), Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn dot_product_of_orthogonal_vectors_is_zero() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(x.dot(&y), 0.0);
    }

    #[test]
    fn normalize_of_zero_vector_is_none() {
        assert_eq!(Vec3::ZERO.normalize(), None);
    }

    #[test]
    fn normalize_scales_to_unit_length() {
        let v = Vec3::new(3.0, 4.0, 0.0).normalize().unwrap();
        assert!((v.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn identity_quaternion_rotate_is_a_no_op() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(Quat::IDENTITY.rotate(&v), v);
    }

    #[test]
    fn quarter_turn_about_z_maps_x_axis_to_y_axis() {
        // 90deg about +Z: w = cos(45deg), z = sin(45deg).
        let half = std::f32::consts::FRAC_PI_4;
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let rotated = q.rotate(&Vec3::new(1.0, 0.0, 0.0));
        assert!((rotated.x).abs() < 1e-5);
        assert!((rotated.y - 1.0).abs() < 1e-5);
        assert!((rotated.z).abs() < 1e-5);
    }

    #[test]
    fn quaternion_product_composes_rotations() {
        let half = std::f32::consts::FRAC_PI_4;
        let quarter_turn_z = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let half_turn_z = quarter_turn_z.mul(&quarter_turn_z).normalize();
        let rotated = half_turn_z.rotate(&Vec3::new(1.0, 0.0, 0.0));
        assert!((rotated.x + 1.0).abs() < 1e-5);
        assert!((rotated.y).abs() < 1e-5);
    }
}
