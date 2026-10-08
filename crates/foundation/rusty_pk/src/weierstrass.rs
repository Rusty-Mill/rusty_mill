//! Short Weierstrass curves `y^2 = x^3 - 3x + b` (P-256, P-384) in Jacobian
//! coordinates. **Variable time throughout**: used only to verify signatures
//! over public data. Every secret-scalar operation lives elsewhere.

use crate::field::{Field, E};
use crate::params::*;

/// A curve with its field, scalar field and base point (Montgomery domain).
pub(crate) struct Curve {
    pub(crate) fp: Field,
    pub(crate) fnn: Field,
    b: E,
    gx: E,
    gy: E,
    /// Byte length of a field element / scalar (32 or 48).
    pub(crate) len: usize,
}

/// A Jacobian point; `z == 0` is the point at infinity.
#[derive(Clone, Copy)]
pub(crate) struct Jac {
    x: E,
    y: E,
    z: E,
}

impl Curve {
    pub(crate) fn p256() -> Option<Self> {
        Self::build(&P256_P, &P256_N, &P256_B, &P256_GX, &P256_GY, 32)
    }

    pub(crate) fn p384() -> Option<Self> {
        Self::build(&P384_P, &P384_N, &P384_B, &P384_GX, &P384_GY, 48)
    }

    fn build(p: &[u64], n: &[u64], b: &[u64], gx: &[u64], gy: &[u64], len: usize) -> Option<Self> {
        let fp = Field::new(p)?;
        let fnn = Field::new(n)?;
        let enter = |v: &[u64]| fp.enter(&E::from_limbs(v));
        let (b, gx, gy) = (enter(b), enter(gx), enter(gy));
        Some(Self {
            fp,
            fnn,
            b,
            gx,
            gy,
            len,
        })
    }

    pub(crate) fn generator(&self) -> Jac {
        Jac {
            x: self.gx,
            y: self.gy,
            z: self.fp.one(),
        }
    }

    /// An affine point from big-endian coordinates, validated: both below
    /// `p`, and on the curve.
    pub(crate) fn point_from_be(&self, x: &[u8], y: &[u8]) -> Option<Jac> {
        let (x, y) = (self.fp.parse_be(x)?, self.fp.parse_be(y)?);
        if !self.fp.is_reduced_vartime(&x) || !self.fp.is_reduced_vartime(&y) {
            return None;
        }
        let (x, y) = (self.fp.enter(&x), self.fp.enter(&y));
        let f = &self.fp;
        let three_x = f.add(&f.add(&x, &x), &x);
        let rhs = f.add(&f.sub(&f.mul(&f.sqr(&x), &x), &three_x), &self.b);
        if !f.eq_vartime(&f.sqr(&y), &rhs) {
            return None;
        }
        Some(Jac { x, y, z: f.one() })
    }

    fn is_infinity(&self, p: &Jac) -> bool {
        self.fp.is_zero_vartime(&p.z)
    }

    fn infinity(&self) -> Jac {
        Jac {
            x: self.fp.one(),
            y: self.fp.one(),
            z: E::ZERO,
        }
    }

    fn double(&self, p: &Jac) -> Jac {
        let f = &self.fp;
        if self.is_infinity(p) || f.is_zero_vartime(&p.y) {
            return self.infinity();
        }
        let delta = f.sqr(&p.z);
        let gamma = f.sqr(&p.y);
        let beta = f.mul(&p.x, &gamma);
        let t = f.mul(&f.sub(&p.x, &delta), &f.add(&p.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let beta2 = f.add(&beta, &beta);
        let beta4 = f.add(&beta2, &beta2);
        let beta8 = f.add(&beta4, &beta4);
        let x3 = f.sub(&f.sqr(&alpha), &beta8);
        let z3 = f.sub(&f.sub(&f.sqr(&f.add(&p.y, &p.z)), &gamma), &delta);
        let gamma_sq = f.sqr(&gamma);
        let gamma_sq2 = f.add(&gamma_sq, &gamma_sq);
        let gamma_sq4 = f.add(&gamma_sq2, &gamma_sq2);
        let gamma_sq8 = f.add(&gamma_sq4, &gamma_sq4);
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &gamma_sq8);
        Jac {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    fn add(&self, p: &Jac, q: &Jac) -> Jac {
        let f = &self.fp;
        if self.is_infinity(p) {
            return *q;
        }
        if self.is_infinity(q) {
            return *p;
        }
        let z1z1 = f.sqr(&p.z);
        let z2z2 = f.sqr(&q.z);
        let u1 = f.mul(&p.x, &z2z2);
        let u2 = f.mul(&q.x, &z1z1);
        let s1 = f.mul(&f.mul(&p.y, &q.z), &z2z2);
        let s2 = f.mul(&f.mul(&q.y, &p.z), &z1z1);
        let h = f.sub(&u2, &u1);
        let r = f.sub(&s2, &s1);
        if f.is_zero_vartime(&h) {
            return if f.is_zero_vartime(&r) {
                self.double(p)
            } else {
                self.infinity()
            };
        }
        let r = f.add(&r, &r);
        let h2 = f.add(&h, &h);
        let i = f.sqr(&h2);
        let j = f.mul(&h, &i);
        let v = f.mul(&u1, &i);
        let x3 = f.sub(&f.sub(&f.sqr(&r), &j), &f.add(&v, &v));
        let s1j = f.mul(&s1, &j);
        let y3 = f.sub(&f.mul(&r, &f.sub(&v, &x3)), &f.add(&s1j, &s1j));
        let z3 = f.mul(&f.sub(&f.sub(&f.sqr(&f.add(&p.z, &q.z)), &z1z1), &z2z2), &h);
        Jac {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// `u1*G + u2*Q` (Shamir's trick). Scalars are plain (non-Montgomery)
    /// limbs. Variable time: public inputs only.
    pub(crate) fn mul2_vartime(&self, u1: &E, u2: &E, q: &Jac) -> Jac {
        let g = self.generator();
        let gq = self.add(&g, q);
        let mut acc = self.infinity();
        for bit in (0..self.len * 8).rev() {
            acc = self.double(&acc);
            let b1 = (u1.0[bit / 64] >> (bit % 64)) & 1;
            let b2 = (u2.0[bit / 64] >> (bit % 64)) & 1;
            acc = match (b1, b2) {
                (0, 0) => acc,
                (1, 0) => self.add(&acc, &g),
                (0, _) => self.add(&acc, q),
                _ => self.add(&acc, &gq),
            };
        }
        acc
    }

    /// The affine x coordinate (plain limbs), or `None` at infinity.
    pub(crate) fn affine_x(&self, p: &Jac) -> Option<E> {
        if self.is_infinity(p) {
            return None;
        }
        let f = &self.fp;
        let zi = f.invert(&p.z);
        Some(f.leave(&f.mul(&p.x, &f.sqr(&zi))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_has_the_stated_order() {
        for curve in [Curve::p256().unwrap(), Curve::p384().unwrap()] {
            // (n-1)G + G = nG = infinity.
            let mut n = curve.fnn.modulus();
            n.0[0] -= 1; // n - 1 (n is odd)
            let g = curve.generator();
            let r = curve.mul2_vartime(&n, &E::ZERO, &g); // (n-1)G
            let sum = curve.add(&r, &g);
            assert!(curve.is_infinity(&sum));
            assert!(!curve.is_infinity(&r));
        }
    }

    #[test]
    fn doubling_agrees_with_adding() {
        let c = Curve::p256().unwrap();
        let g = c.generator();
        let d = c.double(&g);
        let two = c.mul2_vartime(&E::from_limbs(&[2]), &E::ZERO, &g);
        assert!(c
            .fp
            .eq_vartime(&c.affine_x(&d).unwrap(), &c.affine_x(&two).unwrap()));
    }

    #[test]
    fn rejects_points_off_the_curve_or_out_of_range() {
        let c = Curve::p256().unwrap();
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        y[31] = 1;
        assert!(c.point_from_be(&x, &y).is_none());
        x = [0xff; 32];
        assert!(c.point_from_be(&x, &y).is_none());
    }
}
