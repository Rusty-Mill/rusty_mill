//! A general 3x3 matrix, ported from
//! `bullet3/src/LinearMath/btMatrix3x3.h` (zlib license — see
//! `THIRD_PARTY_NOTICES.md`). Needed once a body's inertia is anisotropic
//! (a box's three principal moments differ) — a sphere's isotropic inertia
//! doesn't need this at all, but `RigidBody` uses `Mat3` uniformly for
//! both shapes (see `body.rs`) so `integrate`/`solver` have one code path
//! rather than a scalar-inertia branch duplicated per shape.

use rb_domain::{Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    rows: [[f32; 3]; 3],
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    pub fn from_diagonal(d: Vec3) -> Mat3 {
        Mat3 {
            rows: [[d.x, 0.0, 0.0], [0.0, d.y, 0.0], [0.0, 0.0, d.z]],
        }
    }

    /// Port of `btMatrix3x3::setRotation`: the rotation matrix equivalent
    /// of a unit quaternion.
    pub fn from_quat(q: &Quat) -> Mat3 {
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Mat3 {
            rows: [
                [1.0 - (yy + zz), xy - wz, xz + wy],
                [xy + wz, 1.0 - (xx + zz), yz - wx],
                [xz - wy, yz + wx, 1.0 - (xx + yy)],
            ],
        }
    }

    fn row(&self, i: usize) -> Vec3 {
        Vec3::new(self.rows[i][0], self.rows[i][1], self.rows[i][2])
    }

    pub fn mul_vec3(&self, v: &Vec3) -> Vec3 {
        Vec3::new(self.row(0).dot(v), self.row(1).dot(v), self.row(2).dot(v))
    }

    /// `self * other` (matrix product).
    pub fn mul_mat3(&self, other: &Mat3) -> Mat3 {
        let mut rows = [[0.0f32; 3]; 3];
        for (i, row) in rows.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.rows[i][k] * other.rows[k][j]).sum();
            }
        }
        Mat3 { rows }
    }

    pub fn transpose(&self) -> Mat3 {
        let mut rows = [[0.0f32; 3]; 3];
        for (i, row) in self.rows.iter().enumerate() {
            for (j, &val) in row.iter().enumerate() {
                rows[j][i] = val;
            }
        }
        Mat3 { rows }
    }

    /// `self * diag(d)` — scales column `j` by `d[j]`. Port of
    /// `btMatrix3x3::scaled`, used by `RigidBody::update_inertia_tensor` to
    /// compute `R * diag(invInertiaLocal) * R^T`.
    pub fn scaled_columns(&self, d: &Vec3) -> Mat3 {
        let mut rows = self.rows;
        for row in &mut rows {
            row[0] *= d.x;
            row[1] *= d.y;
            row[2] *= d.z;
        }
        Mat3 { rows }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn identity_times_vector_is_unchanged() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(Mat3::IDENTITY.mul_vec3(&v), v);
    }

    #[test]
    fn from_diagonal_scales_each_axis_independently() {
        let m = Mat3::from_diagonal(Vec3::new(2.0, 3.0, 4.0));
        let v = Vec3::new(1.0, 1.0, 1.0);
        assert_eq!(m.mul_vec3(&v), Vec3::new(2.0, 3.0, 4.0));
    }

    #[test]
    fn from_quat_identity_is_identity_matrix() {
        let m = Mat3::from_quat(&Quat::IDENTITY);
        let v = Vec3::new(5.0, -2.0, 7.0);
        assert!((m.mul_vec3(&v) - v).length() < 1e-6);
    }

    #[test]
    fn from_quat_quarter_turn_about_z_matches_quat_rotate() {
        let half = std::f32::consts::FRAC_PI_4;
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let m = Mat3::from_quat(&q);
        let v = Vec3::new(1.0, 0.0, 0.0);
        assert!((m.mul_vec3(&v) - q.rotate(&v)).length() < 1e-5);
    }

    #[test]
    fn transpose_of_rotation_matrix_is_its_inverse() {
        let half = std::f32::consts::FRAC_PI_4;
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let m = Mat3::from_quat(&q);
        let identity_ish = m.mul_mat3(&m.transpose());
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert!((identity_ish.mul_vec3(&v) - v).length() < 1e-5);
    }

    #[test]
    fn scaled_columns_matches_multiplying_by_diagonal_matrix() {
        let half = std::f32::consts::FRAC_PI_4;
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let m = Mat3::from_quat(&q);
        let d = Vec3::new(2.0, 3.0, 4.0);
        let via_scaled = m.scaled_columns(&d);
        let via_mul = m.mul_mat3(&Mat3::from_diagonal(d));
        let v = Vec3::new(1.0, 1.0, 1.0);
        assert!((via_scaled.mul_vec3(&v) - via_mul.mul_vec3(&v)).length() < 1e-5);
    }

    #[test]
    fn isotropic_inertia_is_orientation_independent() {
        // R * kI * R^T == kI for any rotation R — a sphere's inertia
        // tensor shouldn't change with orientation. Confirms Mat3-based
        // inertia is a strict generalization, not a behavior change, for
        // the existing sphere path.
        let half = std::f32::consts::FRAC_PI_4;
        let q = Quat::new(0.0, 0.0, half.sin(), half.cos());
        let basis = Mat3::from_quat(&q);
        let k = 2.5;
        let world = basis
            .scaled_columns(&Vec3::new(k, k, k))
            .mul_mat3(&basis.transpose());
        let v = Vec3::new(1.0, 1.0, 1.0);
        assert!((world.mul_vec3(&v) - Vec3::new(k, k, k)).length() < 1e-5);
    }
}
