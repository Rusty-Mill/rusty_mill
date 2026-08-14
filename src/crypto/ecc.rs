//! Elliptic curve arithmetic over NIST P-256 (aka `secp256r1`, aka
//! `prime256v1`), the curve `ES256` (RFC 7518 §3.4) uses. Backs
//! [`crate::jwt::es256`].
//!
//! # Security note: not constant-time
//!
//! Like [`crate::crypto::bigint`]'s `modpow`, this implementation is not
//! constant-time in the cache/branch-timing sense: [`Point::scalar_mul`]
//! uses a Montgomery ladder (a fixed sequence of one point-add and one
//! point-doubling per scalar bit, rather than skipping the add for zero
//! bits) specifically to avoid the *worst* class of leakage, and ECDSA
//! signing ([`crate::jwt::es256::sign_p256_sha256`]) uses RFC 6979 deterministic
//! nonce derivation specifically to avoid the catastrophic
//! nonce-reuse/bias failure class that has broken ECDSA in the wild
//! (weak/predictable RNGs). Neither of those defenses makes the
//! underlying [`crate::crypto::bigint::BigUint`] arithmetic itself
//! constant-time -- there is no hardened, side-channel-resistant bignum
//! backend here, by nature of being hand-rolled. Treat this as
//! appropriate for typical OAuth/OIDC token verification and DPoP proof
//! generation, not as hardened against a co-located attacker profiling
//! cache/branch timing.

use crate::crypto::bigint::BigUint;
use std::cmp::Ordering;

/// P-256 field prime: `2^256 - 2^224 + 2^192 + 2^96 - 1`.
fn p() -> BigUint {
    BigUint::from_bytes_be(&hex(
        "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    ))
}

/// P-256 curve coefficient `a` (the curve is `y^2 = x^3 - 3x + b`).
fn a() -> BigUint {
    BigUint::from_bytes_be(&hex(
        "ffffffff00000001000000000000000000000000fffffffffffffffffffffffc",
    ))
}

/// P-256 curve coefficient `b`.
fn b() -> BigUint {
    BigUint::from_bytes_be(&hex(
        "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
    ))
}

/// P-256 group order (the number of points on the curve, and the modulus
/// ECDSA's `r`/`s` and nonces live in).
pub fn order() -> BigUint {
    BigUint::from_bytes_be(&hex(
        "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
    ))
}

/// The P-256 base point `G`.
pub fn base_point() -> Point {
    Point::Affine {
        x: BigUint::from_bytes_be(&hex(
            "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
        )),
        y: BigUint::from_bytes_be(&hex(
            "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
        )),
    }
}

/// The byte length of a P-256 field element / coordinate (32 bytes).
pub const FIELD_BYTE_LEN: usize = 32;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

pub(crate) fn mod_add(x: &BigUint, y: &BigUint, m: &BigUint) -> BigUint {
    x.add(y).rem(m)
}

pub(crate) fn mod_sub(x: &BigUint, y: &BigUint, m: &BigUint) -> BigUint {
    let x = x.rem(m);
    let y = y.rem(m);
    if x.compare(&y) != Ordering::Less {
        x.sub(&y)
    } else {
        m.sub(&y.sub(&x))
    }
}

pub(crate) fn mod_mul(x: &BigUint, y: &BigUint, m: &BigUint) -> BigUint {
    x.mul(y).rem(m)
}

/// Modular inverse via Fermat's little theorem (`x^(m-2) mod m`), valid
/// for any prime `m` -- both the field prime `p` and the group order
/// `n` are prime for P-256. `m` must be a public constant: see
/// [`BigUint::modpow`]'s note on why a public exponent (here, `m - 2`)
/// keeps this safe to use with a secret `x`.
pub(crate) fn mod_inverse(x: &BigUint, m: &BigUint) -> BigUint {
    let two = BigUint::from_u32(2);
    x.rem(m).modpow(&m.sub(&two), m)
}

/// A point on the P-256 curve, in affine coordinates, or the point at
/// infinity (the group's identity element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Point {
    Infinity,
    Affine { x: BigUint, y: BigUint },
}

impl Point {
    /// Parses an uncompressed SEC1 point encoding (RFC 7518 §6.2.1's `x`/`y`
    /// are exactly this format's two halves): `04 || X || Y`, each of
    /// [`FIELD_BYTE_LEN`] bytes. Also accepts bare `X || Y` (no `0x04`
    /// prefix), which is what JWK `x`/`y` members decode to individually.
    pub fn from_affine_coordinates(x_bytes: &[u8], y_bytes: &[u8]) -> Option<Point> {
        if x_bytes.len() > FIELD_BYTE_LEN || y_bytes.len() > FIELD_BYTE_LEN {
            return None;
        }
        let point = Point::Affine {
            x: BigUint::from_bytes_be(x_bytes),
            y: BigUint::from_bytes_be(y_bytes),
        };
        if point.is_on_curve() {
            Some(point)
        } else {
            None
        }
    }

    /// Checks that this point actually satisfies the curve equation
    /// `y^2 = x^3 - 3x + b (mod p)` -- rejecting points that don't is a
    /// required defense against invalid-curve attacks (a key confusion
    /// attack where a malicious "public key" is chosen off the intended
    /// curve to leak information about a private scalar used with it).
    /// The point at infinity is trivially considered on-curve.
    pub fn is_on_curve(&self) -> bool {
        match self {
            Point::Infinity => true,
            Point::Affine { x, y } => {
                let p = p();
                if x.compare(&p) != Ordering::Less || y.compare(&p) != Ordering::Less {
                    return false;
                }
                // y^2 = x^3 + a*x + b (mod p). `a` is already stored as
                // its canonical positive representative (p - 3), so this
                // is a plain addition, not `x^3 - a*x + b`.
                let lhs = mod_mul(y, y, &p);
                let x3 = mod_mul(&mod_mul(x, x, &p), x, &p);
                let ax = mod_mul(&a(), x, &p);
                let rhs = mod_add(&mod_add(&x3, &ax, &p), &b(), &p);
                lhs == rhs
            }
        }
    }

    fn double(&self) -> Point {
        let p = p();
        match self {
            Point::Infinity => Point::Infinity,
            // A point with y=0 has order 2, which can't occur on P-256
            // (its group order is odd/prime, so it has no order-2
            // element besides infinity) -- defensive rather than
            // reachable for valid points, since 2y=0 isn't invertible.
            Point::Affine { y, .. } if y.is_zero() => Point::Infinity,
            Point::Affine { x, y } => {
                // lambda = (3x^2 + a) / (2y) mod p
                let three_x2 = mod_mul(&BigUint::from_u32(3), &mod_mul(x, x, &p), &p);
                let numerator = mod_add(&three_x2, &a(), &p);
                let denominator = mod_inverse(&mod_add(y, y, &p), &p);
                let lambda = mod_mul(&numerator, &denominator, &p);

                let x3 = mod_sub(&mod_mul(&lambda, &lambda, &p), &mod_add(x, x, &p), &p);
                let y3 = mod_sub(&mod_mul(&lambda, &mod_sub(x, &x3, &p), &p), y, &p);
                Point::Affine { x: x3, y: y3 }
            }
        }
    }

    fn add_points(&self, other: &Point) -> Point {
        let p = p();
        match (self, other) {
            (Point::Infinity, q) => q.clone(),
            (point, Point::Infinity) => point.clone(),
            (Point::Affine { x: x1, y: y1 }, Point::Affine { x: x2, y: y2 }) => {
                if x1 == x2 {
                    if *y1 == mod_sub(&p, y2, &p) || (y1.is_zero() && y2.is_zero()) {
                        // P + (-P) = O
                        return Point::Infinity;
                    }
                    // P == Q: addition formula's denominator (x2-x1) would
                    // be zero, so this must be a doubling instead.
                    return self.double();
                }
                // lambda = (y2 - y1) / (x2 - x1) mod p
                let numerator = mod_sub(y2, y1, &p);
                let denominator = mod_inverse(&mod_sub(x2, x1, &p), &p);
                let lambda = mod_mul(&numerator, &denominator, &p);

                let x3 = mod_sub(&mod_sub(&mod_mul(&lambda, &lambda, &p), x1, &p), x2, &p);
                let y3 = mod_sub(&mod_mul(&lambda, &mod_sub(x1, &x3, &p), &p), y1, &p);
                Point::Affine { x: x3, y: y3 }
            }
        }
    }

    /// Scalar multiplication `k * self`, via a Montgomery ladder (see the
    /// module-level security note) over internal `Jacobian` coordinates.
    /// Always runs a fixed 256 iterations (P-256's bit width), regardless
    /// of `k`'s actual magnitude, so the iteration count alone doesn't
    /// leak how large `k` is.
    ///
    /// Uses Jacobian coordinates rather than the affine `add_points`/
    /// `double` above specifically for speed: affine addition needs a
    /// field inversion (a full modular exponentiation) *per point
    /// operation*, and the ladder performs roughly two of those per bit --
    /// for a 256-bit scalar that's several hundred inversions, which this
    /// hand-rolled bignum backend is nowhere near fast enough to make
    /// practical (tens of seconds). Jacobian coordinates defer the
    /// inversion to a single call at the very end, in `Jacobian::to_affine`.
    pub fn scalar_mul(&self, k: &BigUint) -> Point {
        let mut r0 = Jacobian::infinity();
        let mut r1 = Jacobian::from_affine(self);
        for i in (0..256).rev() {
            if k.bit(i) {
                r0 = r0.add(&r1);
                r1 = r1.double();
            } else {
                r1 = r0.add(&r1);
                r0 = r0.double();
            }
        }
        r0.to_affine()
    }

    /// This point's affine `(x, y)` coordinates, each padded to
    /// [`FIELD_BYTE_LEN`] bytes, or `None` for the point at infinity.
    pub fn to_affine_bytes(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        match self {
            Point::Infinity => None,
            Point::Affine { x, y } => Some((
                x.to_bytes_be_padded(FIELD_BYTE_LEN)?,
                y.to_bytes_be_padded(FIELD_BYTE_LEN)?,
            )),
        }
    }
}

/// A point in Jacobian projective coordinates: `(X, Y, Z)` represents the
/// affine point `(X/Z^2, Y/Z^3)`; `Z = 0` represents the point at
/// infinity. Used only internally by [`Point::scalar_mul`]'s ladder, to
/// avoid a field inversion per point operation (see that method's docs).
/// Formulas are the standard `a = -3` Jacobian doubling
/// (`dbl-2001-b`-style) and general addition (`add-2007-bl`-style) laws.
#[derive(Debug, Clone)]
struct Jacobian {
    x: BigUint,
    y: BigUint,
    z: BigUint,
}

impl Jacobian {
    fn infinity() -> Self {
        Jacobian {
            x: BigUint::from_u32(1),
            y: BigUint::from_u32(1),
            z: BigUint::zero(),
        }
    }

    fn from_affine(point: &Point) -> Self {
        match point {
            Point::Infinity => Jacobian::infinity(),
            Point::Affine { x, y } => Jacobian {
                x: x.clone(),
                y: y.clone(),
                z: BigUint::from_u32(1),
            },
        }
    }

    fn is_infinity(&self) -> bool {
        self.z.is_zero()
    }

    fn to_affine(&self) -> Point {
        if self.is_infinity() {
            return Point::Infinity;
        }
        let p = p();
        let z_inv = mod_inverse(&self.z, &p);
        let z_inv2 = mod_mul(&z_inv, &z_inv, &p);
        let z_inv3 = mod_mul(&z_inv2, &z_inv, &p);
        Point::Affine {
            x: mod_mul(&self.x, &z_inv2, &p),
            y: mod_mul(&self.y, &z_inv3, &p),
        }
    }

    fn double(&self) -> Jacobian {
        if self.is_infinity() || self.y.is_zero() {
            return Jacobian::infinity();
        }
        let p = p();
        let (x1, y1, z1) = (&self.x, &self.y, &self.z);

        let delta = mod_mul(z1, z1, &p);
        let gamma = mod_mul(y1, y1, &p);
        let beta = mod_mul(x1, &gamma, &p);
        let x1_minus_delta = mod_sub(x1, &delta, &p);
        let x1_plus_delta = mod_add(x1, &delta, &p);
        let alpha = mod_mul(
            &BigUint::from_u32(3),
            &mod_mul(&x1_minus_delta, &x1_plus_delta, &p),
            &p,
        );

        let eight_beta = mod_mul(&BigUint::from_u32(8), &beta, &p);
        let x3 = mod_sub(&mod_mul(&alpha, &alpha, &p), &eight_beta, &p);

        let y1_plus_z1_sq = {
            let s = mod_add(y1, z1, &p);
            mod_mul(&s, &s, &p)
        };
        let z3 = mod_sub(&mod_sub(&y1_plus_z1_sq, &gamma, &p), &delta, &p);

        let four_beta_minus_x3 = mod_sub(&mod_mul(&BigUint::from_u32(4), &beta, &p), &x3, &p);
        let eight_gamma2 = mod_mul(&BigUint::from_u32(8), &mod_mul(&gamma, &gamma, &p), &p);
        let y3 = mod_sub(&mod_mul(&alpha, &four_beta_minus_x3, &p), &eight_gamma2, &p);

        Jacobian {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    fn add(&self, other: &Jacobian) -> Jacobian {
        if self.is_infinity() {
            return other.clone();
        }
        if other.is_infinity() {
            return self.clone();
        }
        let p = p();
        let (x1, y1, z1) = (&self.x, &self.y, &self.z);
        let (x2, y2, z2) = (&other.x, &other.y, &other.z);

        let z1z1 = mod_mul(z1, z1, &p);
        let z2z2 = mod_mul(z2, z2, &p);
        let u1 = mod_mul(x1, &z2z2, &p);
        let u2 = mod_mul(x2, &z1z1, &p);
        let s1 = mod_mul(y1, &mod_mul(z2, &z2z2, &p), &p);
        let s2 = mod_mul(y2, &mod_mul(z1, &z1z1, &p), &p);

        if u1 == u2 {
            return if s1 != s2 {
                Jacobian::infinity() // P + (-P)
            } else {
                self.double() // P == Q
            };
        }

        let h = mod_sub(&u2, &u1, &p);
        let i = {
            let two_h = mod_add(&h, &h, &p);
            mod_mul(&two_h, &two_h, &p)
        };
        let j = mod_mul(&h, &i, &p);
        let r = {
            let d = mod_sub(&s2, &s1, &p);
            mod_add(&d, &d, &p)
        };
        let v = mod_mul(&u1, &i, &p);

        let x3 = mod_sub(
            &mod_sub(&mod_mul(&r, &r, &p), &j, &p),
            &mod_add(&v, &v, &p),
            &p,
        );
        let two_s1_j = mod_mul(&BigUint::from_u32(2), &mod_mul(&s1, &j, &p), &p);
        let y3 = mod_sub(&mod_mul(&r, &mod_sub(&v, &x3, &p), &p), &two_s1_j, &p);
        let z3 = {
            let s = mod_add(z1, z2, &p);
            let s_sq = mod_mul(&s, &s, &p);
            mod_mul(&mod_sub(&mod_sub(&s_sq, &z1z1, &p), &z2z2, &p), &h, &p)
        };

        Jacobian {
            x: x3,
            y: y3,
            z: z3,
        }
    }
}

/// Computes `u1*G + u2*Q` (double scalar multiplication), the core
/// operation of ECDSA verification. Implemented as two independent
/// [`Point::scalar_mul`] calls plus a point addition rather than a
/// combined Shamir's-trick ladder -- simpler and still correct, just not
/// the fastest possible; verification isn't performance-critical here.
pub fn double_scalar_mul(u1: &BigUint, g: &Point, u2: &BigUint, q: &Point) -> Point {
    g.scalar_mul(u1).add_points(&q.scalar_mul(u2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_point_is_on_curve() {
        assert!(base_point().is_on_curve());
    }

    #[test]
    fn point_at_infinity_is_identity() {
        let g = base_point();
        assert_eq!(g.add_points(&Point::Infinity), g);
        assert_eq!(Point::Infinity.add_points(&g), g);
    }

    #[test]
    fn doubling_matches_addition_to_self() {
        let g = base_point();
        assert_eq!(g.double(), g.add_points(&g));
    }

    #[test]
    fn scalar_mul_by_one_is_identity() {
        let g = base_point();
        assert_eq!(g.scalar_mul(&BigUint::from_u32(1)), g);
    }

    #[test]
    fn scalar_mul_by_two_matches_doubling() {
        let g = base_point();
        assert_eq!(g.scalar_mul(&BigUint::from_u32(2)), g.double());
    }

    #[test]
    fn scalar_mul_by_three_matches_repeated_addition() {
        let g = base_point();
        let expected = g.double().add_points(&g);
        assert_eq!(g.scalar_mul(&BigUint::from_u32(3)), expected);
    }

    #[test]
    fn point_plus_its_negation_is_infinity() {
        let g = base_point();
        let Point::Affine { x, y } = &g else {
            panic!("G is affine")
        };
        let neg_g = Point::Affine {
            x: x.clone(),
            y: mod_sub(&p(), y, &p()),
        };
        assert!(neg_g.is_on_curve());
        assert_eq!(g.add_points(&neg_g), Point::Infinity);
    }

    #[test]
    fn rejects_point_not_on_curve() {
        let (gx, gy) = base_point().to_affine_bytes().unwrap();
        let mut tampered_y = gy.clone();
        tampered_y[31] ^= 0x01;
        assert!(Point::from_affine_coordinates(&gx, &tampered_y).is_none());
    }

    #[test]
    fn order_times_g_is_infinity() {
        // The defining property of the group order: n*G = O.
        assert_eq!(base_point().scalar_mul(&order()), Point::Infinity);
    }

    /// Cross-checks `scalar_mul` against a real key pair generated by
    /// `openssl ecparam -genkey`: `d * G` must equal the public key
    /// `openssl` reports for that private scalar `d`. This validates the
    /// implementation against an independent, trusted source of truth
    /// rather than only against itself.
    #[test]
    fn scalar_mul_matches_openssl_generated_keypair() {
        let d = BigUint::from_bytes_be(&hex(
            "67718fec6a6b21b412a5c5306286f1ee30e32498fd6c61b66f57d0ad1d7c0738",
        ));
        let expected = Point::from_affine_coordinates(
            &hex("9958e30d1b1ca2943fb08c191400beab172729085e843cf130422d686bf81a7b"),
            &hex("a7613a86bac66693dd6adead383e9e1f0407424dc7281049bce06c3fefa91e6f"),
        )
        .expect("openssl's public key must itself be on-curve");
        assert_eq!(base_point().scalar_mul(&d), expected);
    }

    /// Cross-checks the fast Jacobian-coordinate ladder ([`Point::scalar_mul`])
    /// against repeated calls to the slower, independently-validated affine
    /// `add_points`/`double` for a range of small scalars -- the two
    /// implementations share no code, so agreement is meaningful evidence
    /// both are correct.
    #[test]
    fn jacobian_scalar_mul_matches_affine_repeated_addition() {
        let g = base_point();
        let mut affine_accumulator = Point::Infinity;
        for k in 1u32..=25 {
            affine_accumulator = affine_accumulator.add_points(&g);
            let fast = g.scalar_mul(&BigUint::from_u32(k));
            assert_eq!(
                fast, affine_accumulator,
                "scalar_mul({k}) disagreed with affine repeated addition"
            );
        }
    }
}
