//! Ed25519 signature verification (RFC 8032), matching `ring`'s acceptance:
//!
//! - public key 32 bytes, signature exactly 64;
//! - `S` canonical (`S < L`);
//! - the key's `y` is read modulo `p` (non-canonical encodings are accepted),
//!   a key that is not on the curve is rejected, and an `x = 0` key with the
//!   sign bit set is accepted (RFC 8032 would reject it);
//! - cofactorless check: `encode([S]B - [h]A)` must equal the 32 bytes of `R`,
//!   so a non-canonical `R` can never match.
//!
//! All inputs are public, so the arithmetic is variable time.

use rusty_sha2::{Hash, Sha512};

use crate::field::{Field, E};
use crate::mont;
use crate::params::{ED_BX, ED_BY, ED_D, ED_L, F25519_P, F25519_SQRT_M1};
use crate::VerifyError;

/// Verifies `signature` (R || S) over `message` under the 32-byte
/// `public_key`.
pub fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
    let key: &[u8; 32] = public_key.try_into().map_err(|_| VerifyError)?;
    if signature.len() != 64 {
        return Err(VerifyError);
    }
    let (r_bytes, s_bytes) = signature.split_at(32);

    let order = Field::new(&ED_L).ok_or(VerifyError)?;
    let s = E::from_limbs(&mont::from_be_bytes(&reversed::<32>(s_bytes), 4).ok_or(VerifyError)?);
    if !order.is_reduced_vartime(&s) {
        return Err(VerifyError);
    }

    let curve = Edwards::new().ok_or(VerifyError)?;
    let a = curve.decode(key).ok_or(VerifyError)?;
    let neg_a = curve.negate(&a);

    let mut hasher = Sha512::new();
    hasher.update(r_bytes);
    hasher.update(key);
    hasher.update(message);
    let h = reduce_digest(&order, &hasher.finalize());

    let check = curve.mul2_vartime(&s, &h, &neg_a);
    if curve.encode(&check).as_slice() == r_bytes {
        Ok(())
    } else {
        Err(VerifyError)
    }
}

/// `bytes` (little-endian) as a big-endian array.
fn reversed<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    for (o, b) in out.iter_mut().zip(bytes.iter().rev()) {
        *o = *b;
    }
    out
}

/// A 64-byte little-endian digest reduced modulo `L`, as plain limbs.
fn reduce_digest(order: &Field, digest: &[u8; 64]) -> E {
    let limb = |i: usize| {
        let mut word = [0u8; 8];
        word.copy_from_slice(&digest[8 * i..8 * i + 8]);
        u64::from_le_bytes(word)
    };
    let lo = E::from_limbs(&[limb(0), limb(1), limb(2), limb(3)]);
    let hi = E::from_limbs(&[limb(4), limb(5), limb(6), limb(7)]);
    // lo + hi*2^256 = lo + hi*R. In Montgomery form (times R) that is
    // enter(lo) + enter(enter(hi)); enter accepts any 256-bit input.
    let x = order.add(&order.enter(&lo), &order.enter(&order.enter(&hi)));
    order.leave(&x)
}

/// edwards25519 in extended coordinates `(X, Y, Z, T)`, `T = XY/Z`.
#[derive(Clone, Copy)]
struct Point {
    x: E,
    y: E,
    z: E,
    t: E,
}

struct Edwards {
    f: Field,
    d2: E,
    d: E,
    sqrt_m1: E,
    base: Point,
}

impl Edwards {
    fn new() -> Option<Self> {
        let f = Field::new(&F25519_P)?;
        let enter = |v: &[u64]| f.enter(&E::from_limbs(v));
        let d = enter(&ED_D);
        let d2 = f.add(&d, &d);
        let (bx, by) = (enter(&ED_BX), enter(&ED_BY));
        let base = Point {
            x: bx,
            y: by,
            z: f.one(),
            t: f.mul(&bx, &by),
        };
        let sqrt_m1 = enter(&F25519_SQRT_M1);
        Some(Self {
            f,
            d2,
            d,
            sqrt_m1,
            base,
        })
    }

    fn identity(&self) -> Point {
        Point {
            x: E::ZERO,
            y: self.f.one(),
            z: self.f.one(),
            t: E::ZERO,
        }
    }

    fn negate(&self, p: &Point) -> Point {
        Point {
            x: self.f.neg(&p.x),
            y: p.y,
            z: p.z,
            t: self.f.neg(&p.t),
        }
    }

    /// add-2008-hwcd-3 (unified, so it also doubles).
    fn add(&self, p: &Point, q: &Point) -> Point {
        let f = &self.f;
        let a = f.mul(&f.sub(&p.y, &p.x), &f.sub(&q.y, &q.x));
        let b = f.mul(&f.add(&p.y, &p.x), &f.add(&q.y, &q.x));
        let c = f.mul(&f.mul(&p.t, &self.d2), &q.t);
        let zz = f.mul(&p.z, &q.z);
        let d = f.add(&zz, &zz);
        let (e, ff, g, h) = (f.sub(&b, &a), f.sub(&d, &c), f.add(&d, &c), f.add(&b, &a));
        Point {
            x: f.mul(&e, &ff),
            y: f.mul(&g, &h),
            t: f.mul(&e, &h),
            z: f.mul(&ff, &g),
        }
    }

    /// dbl-2008-hwcd for `a = -1`.
    fn double(&self, p: &Point) -> Point {
        let f = &self.f;
        let a = f.sqr(&p.x);
        let b = f.sqr(&p.y);
        let zz = f.sqr(&p.z);
        let c = f.add(&zz, &zz);
        let d = f.neg(&a);
        let e = f.sub(&f.sub(&f.sqr(&f.add(&p.x, &p.y)), &a), &b);
        let g = f.add(&d, &b);
        let ff = f.sub(&g, &c);
        let h = f.sub(&d, &b);
        Point {
            x: f.mul(&e, &ff),
            y: f.mul(&g, &h),
            t: f.mul(&e, &h),
            z: f.mul(&ff, &g),
        }
    }

    /// `s*B + h*Q` (Shamir). Scalars are plain limbs. Variable time.
    fn mul2_vartime(&self, s: &E, h: &E, q: &Point) -> Point {
        let bq = self.add(&self.base, q);
        let mut acc = self.identity();
        for bit in (0..256).rev() {
            acc = self.double(&acc);
            let bs = (s.0[bit / 64] >> (bit % 64)) & 1;
            let bh = (h.0[bit / 64] >> (bit % 64)) & 1;
            acc = match (bs, bh) {
                (0, 0) => acc,
                (1, 0) => self.add(&acc, &self.base),
                (0, _) => self.add(&acc, q),
                _ => self.add(&acc, &bq),
            };
        }
        acc
    }

    fn encode(&self, p: &Point) -> [u8; 32] {
        let f = &self.f;
        let zi = f.invert(&p.z);
        let x = f.leave(&f.mul(&p.x, &zi));
        let y = f.leave(&f.mul(&p.y, &zi));
        let mut be = [0u8; 32];
        // A reduced field element always fits 32 bytes.
        let _ = f.to_be_bytes(&y, &mut be);
        let mut out = reversed::<32>(&be);
        out[31] |= ((x.0[0] & 1) as u8) << 7;
        out
    }

    /// Decodes a compressed point as `ring` does: `y` read mod `p`, no
    /// rejection of `x = 0` with the sign bit set.
    fn decode(&self, bytes: &[u8; 32]) -> Option<Point> {
        let f = &self.f;
        let sign = bytes[31] >> 7;
        let mut le = *bytes;
        le[31] &= 0x7f;
        let y_plain = f.parse_be(&reversed::<32>(&le))?;
        // y < 2^255 < 2p: adding zero reduces it once.
        let y = f.enter(&f.add(&y_plain, &E::ZERO));
        let one = f.one();
        let yy = f.sqr(&y);
        let u = f.sub(&yy, &one);
        let v = f.add(&f.mul(&yy, &self.d), &one);
        let w = f.mul(&u, &v);
        // (p - 5) / 8 = 2^252 - 3
        let exp = [
            0xffff_ffff_ffff_fffd,
            u64::MAX,
            u64::MAX,
            0x0fff_ffff_ffff_ffff,
        ];
        let mut x = f.mul(&f.pow_vartime(&w, &exp), &u);
        let vxx = f.mul(&f.sqr(&x), &v);
        if !f.eq_vartime(&vxx, &u) {
            if !f.eq_vartime(&vxx, &f.neg(&u)) {
                return None;
            }
            x = f.mul(&x, &self.sqrt_m1);
        }
        if (f.leave(&x).0[0] & 1) as u8 != sign {
            x = f.neg(&x);
        }
        Some(Point {
            x,
            y,
            z: one,
            t: f.mul(&x, &y),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_point_encodes_to_the_rfc_value() {
        let c = Edwards::new().unwrap();
        let enc = c.encode(&c.base);
        let mut expected = [0x66u8; 32];
        expected[0] = 0x58;
        assert_eq!(enc, expected);
        let decoded = c.decode(&enc).unwrap();
        assert_eq!(c.encode(&decoded), enc);
    }

    #[test]
    fn base_point_has_order_l() {
        let c = Edwards::new().unwrap();
        // (L-1)*B + B = identity: encode == encode(identity)
        let mut l_minus_1 = E::from_limbs(&ED_L);
        l_minus_1.0[0] -= 1;
        let p = c.mul2_vartime(&l_minus_1, &E::ZERO, &c.base);
        let sum = c.add(&p, &c.base);
        assert_eq!(c.encode(&sum), c.encode(&c.identity()));
        assert_ne!(c.encode(&p), c.encode(&c.identity()));
    }

    #[test]
    fn rejects_wrong_lengths_and_noncanonical_s() {
        assert!(verify(&[0; 31], b"", &[0; 64]).is_err());
        assert!(verify(&[0; 32], b"", &[0; 63]).is_err());
        let mut sig = [0u8; 64];
        sig[32..].copy_from_slice(&[0xff; 32]); // S >= L
        assert!(verify(&[0; 32], b"", &sig).is_err());
    }

    #[test]
    fn digest_reduction_matches_python() {
        let order = Field::new(&ED_L).unwrap();
        // hi = 1, lo = 0 is 2^256.
        let mut digest = [0u8; 64];
        digest[32] = 1;
        assert_eq!(
            reduce_digest(&order, &digest).0[..4],
            [
                0xd6ec31748d98951d,
                0xc6ef5bf4737dcf70,
                0xfffffffffffffffe,
                0x0fffffffffffffff
            ]
        );
        let digest: [u8; 64] = rusty_hex_decode("82b70eee7f1a5039bef07ec2347f066ed08f5dc7512447e3404300026b6e545594a065685d64c4980bb8d4544a8721a99a01ad219eb59cf6a15ef6f15a1d830b");
        assert_eq!(
            reduce_digest(&order, &digest).0[..4],
            [
                0xb9326cc59a3baba5,
                0x346dbafd70c256ae,
                0xb47b5d786377cdbb,
                0x0babc3e64ac3bdff
            ]
        );
    }

    fn rusty_hex_decode(s: &str) -> [u8; 64] {
        let mut out = [0u8; 64];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }
}
