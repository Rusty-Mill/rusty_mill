//! ECDH on P-256 and P-384 (SEC 1 uncompressed points, as TLS 1.3 sends them).
//!
//! The private scalar is secret. [`PrivateKey`] multiplies by it with the
//! complete projective formulas of Renes, Costello and Batina (Eurocrypt 2016,
//! algorithms 4 and 6, `a = -3`), which are valid for every pair of curve points
//! including doubling and the point at infinity, so no input needs a special-case
//! branch. The scalar loop is fixed length (`8 * len` iterations), always doubles
//! and adds, and keeps or discards the sum with a mask ([`select`]), never a branch
//! or an index. The final inversion has a public exponent. Checked by valgrind
//! taint runs, exact disassembly counts and a timing test (`scripts/ct_check.sh`),
//! and **not proven**.
//!
//! The peer's point is public: it is validated with variable-time code (both
//! coordinates below `p`, on the curve; the cofactor is 1, so there is no subgroup
//! check) and the point at infinity cannot be encoded. Intermediate field elements
//! live in stack copies this crate cannot wipe individually; only the private
//! scalar's bytes are wiped.
//!
//! Randomness is the caller's: [`PrivateKey::generate`] takes a fill function.

use rusty_crypto_key::wipe;

use crate::field::{Field, E};
use crate::weierstrass::{Curve as Wc, Jac};

/// The supported curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    /// NIST P-256 (secp256r1).
    P256,
    /// NIST P-384 (secp384r1).
    P384,
}

/// Largest scalar / coordinate length (P-384).
const MAX_LEN: usize = 48;
/// Largest uncompressed point: `0x04 || X || Y` on P-384.
const MAX_POINT: usize = 1 + 2 * MAX_LEN;

impl Curve {
    /// Byte length of a scalar, a coordinate and the shared secret.
    pub const fn len(self) -> usize {
        match self {
            Curve::P256 => 32,
            Curve::P384 => 48,
        }
    }

    /// Always false; present so `len` is not flagged on its own.
    pub const fn is_empty(self) -> bool {
        false
    }

    fn build(self) -> Option<Wc> {
        match self {
            Curve::P256 => Wc::p256(),
            Curve::P384 => Wc::p384(),
        }
    }
}

/// The scalar is not a valid private key (wrong length, zero, or not below the
/// group order), or the peer's point is not a valid public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidKey;

impl core::fmt::Display for InvalidKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("invalid ECDH key")
    }
}

/// Why [`PrivateKey::generate`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateError<E> {
    /// The caller's fill function failed.
    Source(E),
    /// Every candidate was zero or not below the group order (for a working
    /// source this is far below 2^-100 likely): treated as a broken source.
    Exhausted,
}

/// Candidates tried by [`PrivateKey::generate`] before giving up.
const GENERATE_ATTEMPTS: usize = 8;

/// An uncompressed SEC 1 point.
#[derive(Clone, Copy)]
pub struct Point {
    bytes: [u8; MAX_POINT],
    len: usize,
}

impl Point {
    /// `0x04 || X || Y`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// An ECDH shared secret (the affine x coordinate). Wiped on drop.
pub struct SharedSecret {
    bytes: [u8; MAX_LEN],
    len: usize,
}

impl SharedSecret {
    /// The big-endian x coordinate.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Drop for SharedSecret {
    fn drop(&mut self) {
        wipe(&mut self.bytes);
    }
}

/// A private scalar `1 <= d < n`. Wiped on drop; never printed.
pub struct PrivateKey {
    curve: Curve,
    bytes: [u8; MAX_LEN],
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        wipe(&mut self.bytes);
    }
}

impl core::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PrivateKey(..)")
    }
}

impl PrivateKey {
    /// A key from exactly `curve.len()` big-endian bytes, which must encode
    /// `1 <= d < n`. The range check is variable time: a rejected candidate is
    /// discarded, and an accepted one's validity is not secret.
    pub fn from_bytes(curve: Curve, bytes: &[u8]) -> Result<Self, InvalidKey> {
        let c = curve.build().ok_or(InvalidKey)?;
        let d = c.fnn.parse_be(bytes).ok_or(InvalidKey)?;
        if bytes.len() != curve.len() || c.fnn.is_zero_vartime(&d) || !c.fnn.is_reduced_vartime(&d)
        {
            return Err(InvalidKey);
        }
        let mut key = Self {
            curve,
            bytes: [0; MAX_LEN],
        };
        key.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(key)
    }

    /// A fresh key: `fill` supplies `curve.len()` random bytes per attempt, and a
    /// candidate outside `[1, n-1]` is discarded and redrawn (no modular bias).
    pub fn generate<E>(
        curve: Curve,
        mut fill: impl FnMut(&mut [u8]) -> Result<(), E>,
    ) -> Result<Self, GenerateError<E>> {
        let mut buf = [0u8; MAX_LEN];
        for _ in 0..GENERATE_ATTEMPTS {
            if let Err(e) = fill(&mut buf[..curve.len()]) {
                wipe(&mut buf);
                return Err(GenerateError::Source(e));
            }
            let key = Self::from_bytes(curve, &buf[..curve.len()]);
            wipe(&mut buf);
            if let Ok(key) = key {
                return Ok(key);
            }
        }
        Err(GenerateError::Exhausted)
    }

    /// The scalar's bytes, so the valgrind taint example can mark them secret. Not
    /// part of the supported API: nothing else should read a private key's bytes.
    #[doc(hidden)]
    pub fn scalar_bytes_for_taint_run(&self) -> &[u8] {
        &self.bytes[..self.curve.len()]
    }

    /// The public key `d * G`, uncompressed.
    pub fn public_key(&self) -> Result<Point, InvalidKey> {
        let c = self.curve.build().ok_or(InvalidKey)?;
        let g = to_proj(&c, &c.generator());
        let q = mul(&c, &self.bytes[..self.curve.len()], &g);
        encode(&c, &q).ok_or(InvalidKey)
    }

    /// The shared secret with `peer` (`0x04 || X || Y`): the x coordinate of
    /// `d * peer`. Rejects an encoding that is not an uncompressed point on the
    /// curve with coordinates below `p`.
    pub fn agree(&self, peer: &[u8]) -> Result<SharedSecret, InvalidKey> {
        let c = self.curve.build().ok_or(InvalidKey)?;
        let len = self.curve.len();
        let q = decode(&c, peer).ok_or(InvalidKey)?;
        let r = mul(&c, &self.bytes[..len], &q);
        // Infinity is impossible for a valid point and a scalar in [1, n-1]
        // (prime order); the test depends on public data only.
        if c.fp.is_zero_vartime(&r.z) {
            return Err(InvalidKey);
        }
        let x = c.fp.leave(&c.fp.mul(&r.x, &c.fp.invert(&r.z)));
        let mut out = SharedSecret {
            bytes: [0; MAX_LEN],
            len,
        };
        c.fp.to_be_bytes(&x, &mut out.bytes[..len])
            .ok_or(InvalidKey)?;
        Ok(out)
    }
}

/// A projective point `(X : Y : Z)`; infinity is `(0 : 1 : 0)`.
#[derive(Clone, Copy)]
struct Proj {
    x: E,
    y: E,
    z: E,
}

fn to_proj(c: &Wc, p: &Jac) -> Proj {
    // The generator and decoded points have z = 1 (affine).
    debug_assert!(c.fp.eq_vartime(&p.z, &c.fp.one()));
    Proj {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

fn decode(c: &Wc, bytes: &[u8]) -> Option<Proj> {
    let len = c.len;
    if bytes.len() != 1 + 2 * len || bytes[0] != 4 {
        return None;
    }
    let jac = c.point_from_be(&bytes[1..=len], &bytes[1 + len..])?;
    Some(to_proj(c, &jac))
}

fn encode(c: &Wc, p: &Proj) -> Option<Point> {
    let f = &c.fp;
    if f.is_zero_vartime(&p.z) {
        return None;
    }
    let zi = f.invert(&p.z);
    let (x, y) = (f.leave(&f.mul(&p.x, &zi)), f.leave(&f.mul(&p.y, &zi)));
    let mut out = Point {
        bytes: [0; MAX_POINT],
        len: 1 + 2 * c.len,
    };
    out.bytes[0] = 4;
    f.to_be_bytes(&x, &mut out.bytes[1..=c.len])?;
    f.to_be_bytes(&y, &mut out.bytes[1 + c.len..out.len])?;
    Some(out)
}

fn infinity(f: &Field) -> Proj {
    Proj {
        x: E::ZERO,
        y: f.one(),
        z: E::ZERO,
    }
}

/// `inline(never)` on the secret-path functions keeps each one a separate symbol, so
/// `scripts/disasm_limits.txt` can pin its jump count.
///
/// `a` if `mask` is all ones, `b` if it is zero. No branch, no index.
#[inline(never)]
fn select(mask: u64, a: &Proj, b: &Proj) -> Proj {
    let pick = |a: &E, b: &E| {
        let mut out = E::ZERO;
        for i in 0..out.0.len() {
            out.0[i] = b.0[i] ^ (mask & (a.0[i] ^ b.0[i]));
        }
        out
    };
    Proj {
        x: pick(&a.x, &b.x),
        y: pick(&a.y, &b.y),
        z: pick(&a.z, &b.z),
    }
}

/// Complete addition (RCB 2016, algorithm 4, `a = -3`). Valid for all inputs.
#[inline(never)]
fn add(c: &Wc, p: &Proj, q: &Proj) -> Proj {
    let f = &c.fp;
    let b = &c.b;
    let (x1, y1, z1, x2, y2, z2) = (&p.x, &p.y, &p.z, &q.x, &q.y, &q.z);
    let mut t0 = f.mul(x1, x2);
    let mut t1 = f.mul(y1, y2);
    let mut t2 = f.mul(z1, z2);
    let mut t3 = f.add(x1, y1);
    let mut t4 = f.add(x2, y2);
    t3 = f.mul(&t3, &t4);
    t4 = f.add(&t0, &t1);
    t3 = f.sub(&t3, &t4);
    t4 = f.add(y1, z1);
    let mut x3 = f.add(y2, z2);
    t4 = f.mul(&t4, &x3);
    x3 = f.add(&t1, &t2);
    t4 = f.sub(&t4, &x3);
    x3 = f.add(x1, z1);
    let mut y3 = f.add(x2, z2);
    x3 = f.mul(&x3, &y3);
    y3 = f.add(&t0, &t2);
    y3 = f.sub(&x3, &y3);
    let mut z3 = f.mul(b, &t2);
    x3 = f.sub(&y3, &z3);
    z3 = f.add(&x3, &x3);
    x3 = f.add(&x3, &z3);
    z3 = f.sub(&t1, &x3);
    x3 = f.add(&t1, &x3);
    y3 = f.mul(b, &y3);
    t1 = f.add(&t2, &t2);
    t2 = f.add(&t1, &t2);
    y3 = f.sub(&y3, &t2);
    y3 = f.sub(&y3, &t0);
    t1 = f.add(&y3, &y3);
    y3 = f.add(&t1, &y3);
    t1 = f.add(&t0, &t0);
    t0 = f.add(&t1, &t0);
    t0 = f.sub(&t0, &t2);
    t1 = f.mul(&t4, &y3);
    t2 = f.mul(&t0, &y3);
    y3 = f.mul(&x3, &z3);
    y3 = f.add(&y3, &t2);
    x3 = f.mul(&t3, &x3);
    x3 = f.sub(&x3, &t1);
    z3 = f.mul(&t4, &z3);
    t1 = f.mul(&t3, &t0);
    z3 = f.add(&z3, &t1);
    Proj {
        x: x3,
        y: y3,
        z: z3,
    }
}

/// Complete doubling (RCB 2016, algorithm 6, `a = -3`). Valid for all inputs.
#[inline(never)]
fn double(c: &Wc, p: &Proj) -> Proj {
    let f = &c.fp;
    let b = &c.b;
    let (x, y, z) = (&p.x, &p.y, &p.z);
    let mut t0 = f.mul(x, x);
    let t1 = f.mul(y, y);
    let mut t2 = f.mul(z, z);
    let mut t3 = f.mul(x, y);
    t3 = f.add(&t3, &t3);
    let mut z3 = f.mul(x, z);
    z3 = f.add(&z3, &z3);
    let mut y3 = f.mul(b, &t2);
    y3 = f.sub(&y3, &z3);
    let mut x3 = f.add(&y3, &y3);
    y3 = f.add(&x3, &y3);
    x3 = f.sub(&t1, &y3);
    y3 = f.add(&t1, &y3);
    y3 = f.mul(&x3, &y3);
    x3 = f.mul(&x3, &t3);
    t3 = f.add(&t2, &t2);
    t2 = f.add(&t2, &t3);
    z3 = f.mul(b, &z3);
    z3 = f.sub(&z3, &t2);
    z3 = f.sub(&z3, &t0);
    t3 = f.add(&z3, &z3);
    z3 = f.add(&z3, &t3);
    t3 = f.add(&t0, &t0);
    t0 = f.add(&t3, &t0);
    t0 = f.sub(&t0, &t2);
    t0 = f.mul(&t0, &z3);
    y3 = f.add(&y3, &t0);
    t0 = f.mul(y, z);
    t0 = f.add(&t0, &t0);
    z3 = f.mul(&t0, &z3);
    x3 = f.sub(&x3, &z3);
    z3 = f.mul(&t0, &t1);
    z3 = f.add(&z3, &z3);
    z3 = f.add(&z3, &z3);
    Proj {
        x: x3,
        y: y3,
        z: z3,
    }
}

/// `scalar * p` for a secret big-endian `scalar` of exactly `c.len` bytes.
#[inline(never)]
fn mul(c: &Wc, scalar: &[u8], p: &Proj) -> Proj {
    let mut acc = infinity(&c.fp);
    for byte in scalar {
        for shift in (0..8).rev() {
            acc = double(c, &acc);
            let sum = add(c, &acc, p);
            let bit = u64::from((byte >> shift) & 1);
            let mask = core::hint::black_box(0u64.wrapping_sub(bit));
            acc = select(mask, &sum, &acc);
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curves() -> [(Curve, Wc); 2] {
        [
            (Curve::P256, Wc::p256().unwrap()),
            (Curve::P384, Wc::p384().unwrap()),
        ]
    }

    /// The affine x of a projective point, plain limbs, or `None` at infinity.
    fn x_of(c: &Wc, p: &Proj) -> Option<E> {
        if c.fp.is_zero_vartime(&p.z) {
            return None;
        }
        Some(c.fp.leave(&c.fp.mul(&p.x, &c.fp.invert(&p.z))))
    }

    fn scalar(len: usize, seed: u8) -> alloc::vec::Vec<u8> {
        let mut state = u64::from(seed) | 1;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 56) as u8
            })
            .collect()
    }

    fn jac_of(c: &Wc, k: &E) -> crate::weierstrass::Jac {
        c.mul2_vartime(k, &E::ZERO, &c.generator())
    }

    #[test]
    fn complete_addition_and_doubling_agree_with_the_reference_arithmetic() {
        for (_, c) in curves() {
            let g = to_proj(&c, &c.generator());
            let inf = infinity(&c.fp);
            // 2G three ways: doubling, G + G (the case incomplete formulas get wrong).
            let two = c.mul2_vartime(&E::from_limbs(&[2]), &E::ZERO, &c.generator());
            let want = c.affine_x(&two).unwrap();
            assert!(c.fp.eq_vartime(&x_of(&c, &double(&c, &g)).unwrap(), &want));
            assert!(c.fp.eq_vartime(&x_of(&c, &add(&c, &g, &g)).unwrap(), &want));
            // identity is neutral on both sides, and doubling it stays at infinity.
            assert!(c.fp.eq_vartime(
                &x_of(&c, &add(&c, &g, &inf)).unwrap(),
                &c.affine_x(&c.generator()).unwrap()
            ));
            assert!(c.fp.eq_vartime(
                &x_of(&c, &add(&c, &inf, &g)).unwrap(),
                &c.affine_x(&c.generator()).unwrap()
            ));
            assert!(x_of(&c, &double(&c, &inf)).is_none());
            assert!(x_of(&c, &add(&c, &inf, &inf)).is_none());
            // P + (-P) = infinity.
            let neg = Proj {
                x: g.x,
                y: c.fp.neg(&g.y),
                z: g.z,
            };
            assert!(x_of(&c, &add(&c, &g, &neg)).is_none());
        }
    }

    #[test]
    fn scalar_multiplication_agrees_with_the_reference_for_edge_and_random_scalars() {
        for (curve, c) in curves() {
            let len = curve.len();
            let g = to_proj(&c, &c.generator());
            let mut scalars = alloc::vec::Vec::new();
            for tail in [1u8, 2, 3] {
                let mut one = alloc::vec![0u8; len];
                one[len - 1] = tail;
                scalars.push(one);
            }
            let mut n_minus_1 = alloc::vec![0u8; len];
            c.fnn
                .to_be_bytes(
                    &{
                        let mut n = c.fnn.modulus();
                        n.0[0] -= 1;
                        n
                    },
                    &mut n_minus_1,
                )
                .unwrap();
            scalars.push(n_minus_1);
            for seed in 0..6 {
                scalars.push(scalar(len, seed));
            }
            for bytes in scalars {
                let k = c.fnn.parse_be(&bytes).unwrap();
                let want = c.affine_x(&jac_of(&c, &k)).map(|x| x.0);
                let got = x_of(&c, &mul(&c, &bytes, &g)).map(|x| x.0);
                assert_eq!(got, want, "{bytes:02x?}");
            }
        }
    }

    #[test]
    fn private_key_range_is_enforced() {
        for (curve, c) in curves() {
            let len = curve.len();
            assert!(PrivateKey::from_bytes(curve, &alloc::vec![0u8; len]).is_err());
            let mut one = alloc::vec![0u8; len];
            one[len - 1] = 1;
            assert!(PrivateKey::from_bytes(curve, &one).is_ok());
            let mut n = alloc::vec![0u8; len];
            c.fnn.to_be_bytes(&c.fnn.modulus(), &mut n).unwrap();
            assert!(PrivateKey::from_bytes(curve, &n).is_err(), "d = n");
            n[len - 1] -= 1;
            assert!(PrivateKey::from_bytes(curve, &n).is_ok(), "d = n - 1");
            assert!(PrivateKey::from_bytes(curve, &alloc::vec![1u8; len - 1]).is_err());
            assert!(PrivateKey::from_bytes(curve, &alloc::vec![1u8; len + 1]).is_err());
        }
    }

    #[test]
    fn generate_redraws_out_of_range_candidates_and_reports_a_broken_source() {
        let mut calls = 0;
        let key = PrivateKey::generate(Curve::P256, |b: &mut [u8]| -> Result<(), ()> {
            calls += 1;
            b.fill(if calls < 3 { 0xff } else { 0x07 }); // 0xff.. >= n twice, then valid
            Ok(())
        });
        assert!(key.is_ok() && calls == 3);
        let zeros = PrivateKey::generate(Curve::P384, |b: &mut [u8]| -> Result<(), ()> {
            b.fill(0);
            Ok(())
        });
        assert!(matches!(zeros, Err(GenerateError::Exhausted)));
        let broken = PrivateKey::generate(Curve::P256, |_: &mut [u8]| Err("no entropy"));
        assert!(matches!(broken, Err(GenerateError::Source("no entropy"))));
    }

    #[test]
    fn peer_encodings_are_validated() {
        for (curve, _) in curves() {
            let len = curve.len();
            let key = PrivateKey::from_bytes(curve, &scalar(len, 9)).unwrap_or_else(|_| {
                let mut one = alloc::vec![0u8; len];
                one[len - 1] = 5;
                PrivateKey::from_bytes(curve, &one).unwrap()
            });
            let good = key.public_key().unwrap();
            assert!(key.agree(good.as_bytes()).is_ok());
            let mut bad = good;
            bad.bytes[0] = 2; // compressed form
            assert!(key.agree(bad.as_bytes()).is_err());
            let mut bad = good;
            bad.bytes[len + 1] ^= 1; // off the curve
            assert!(key.agree(bad.as_bytes()).is_err());
            assert!(key.agree(&good.as_bytes()[..good.len - 1]).is_err());
            assert!(key.agree(&[0u8; 1]).is_err()); // infinity is not encodable
            assert!(key.agree(&[]).is_err());
        }
    }
}
