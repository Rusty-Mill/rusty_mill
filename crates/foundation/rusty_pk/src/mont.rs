//! Montgomery arithmetic on little-endian `u64` limb slices.
//!
//! `mul`, `add`, `sub`, `select` have no data-dependent branch or index: a
//! conditional subtraction is a masked select. The helpers marked `vartime`
//! branch on values and are for public data only.
//!
//! A value `a` is held as `a*R mod n` with `R = 2^(64*limbs)`. The modulus
//! must be odd.

use alloc::vec;
use alloc::vec::Vec;

/// Largest supported limb count (8192 bits).
pub(crate) const MAX_LIMBS: usize = 128;

/// An odd modulus with its Montgomery constants.
#[derive(Clone)]
pub(crate) struct Modulus {
    n: Vec<u64>,
    n0: u64,
    /// `R^2 mod n`, to enter the Montgomery domain.
    rr: Vec<u64>,
    /// `R mod n`: the number 1 in the Montgomery domain.
    one: Vec<u64>,
}

impl Modulus {
    /// Builds the constants for an odd `n` with a non-zero top limb.
    /// Returns `None` for an even, empty, oversized or top-limb-zero modulus.
    pub(crate) fn new(n: &[u64]) -> Option<Self> {
        let k = n.len();
        if k == 0 || k > MAX_LIMBS || n[0] & 1 == 0 || n[k - 1] == 0 {
            return None;
        }
        let n0 = neg_inv(n[0]);
        // R mod n: start from 2^(bits-1) < n, double up to 2^(64k).
        let bits = 64 * k - n[k - 1].leading_zeros() as usize;
        let mut one = vec![0u64; k];
        one[(bits - 1) / 64] = 1 << ((bits - 1) % 64);
        for _ in 0..(64 * k - (bits - 1)) {
            double_mod(&mut one, n);
        }
        // Montgomery form of 2 is 2R mod n; raising it to 64k gives
        // 2^(64k) * R = R^2 mod n.
        let mut two = one.clone();
        double_mod(&mut two, n);
        let mut m = Self {
            n: n.to_vec(),
            n0,
            rr: vec![0; k],
            one,
        };
        let mut acc = m.one.clone();
        let e = 64 * k;
        for bit in (0..usize::BITS - e.leading_zeros()).rev() {
            let sq = acc.clone();
            m.mul(&mut acc, &sq, &sq);
            if (e >> bit) & 1 == 1 {
                let prev = acc.clone();
                m.mul(&mut acc, &prev, &two);
            }
        }
        m.rr = acc;
        Some(m)
    }

    pub(crate) fn modulus(&self) -> &[u64] {
        &self.n
    }

    /// `R mod n`.
    pub(crate) fn one(&self) -> &[u64] {
        &self.one
    }

    /// `out = a * b / R mod n` (Montgomery product). Constant time.
    pub(crate) fn mul(&self, out: &mut [u64], a: &[u64], b: &[u64]) {
        let k = self.n.len();
        let mut t = [0u64; MAX_LIMBS + 2];
        for &bi in &b[..k] {
            let mut carry = 0u64;
            for j in 0..k {
                let v = t[j] as u128 + a[j] as u128 * bi as u128 + carry as u128;
                t[j] = v as u64;
                carry = (v >> 64) as u64;
            }
            let v = t[k] as u128 + carry as u128;
            t[k] = v as u64;
            t[k + 1] = (v >> 64) as u64;

            let m = t[0].wrapping_mul(self.n0);
            let v = t[0] as u128 + m as u128 * self.n[0] as u128;
            let mut carry = (v >> 64) as u64;
            for j in 1..k {
                let v = t[j] as u128 + m as u128 * self.n[j] as u128 + carry as u128;
                t[j - 1] = v as u64;
                carry = (v >> 64) as u64;
            }
            let v = t[k] as u128 + carry as u128;
            t[k - 1] = v as u64;
            t[k] = t[k + 1] + (v >> 64) as u64;
            t[k + 1] = 0;
        }
        // Result is t[0..=k] < 2n: subtract n once if it is >= n.
        reduce_once(&mut out[..k], &t[..k], t[k], &self.n);
    }

    /// `a` into the Montgomery domain: `a * R mod n`. `a` must be `< n`.
    pub(crate) fn enter(&self, out: &mut [u64], a: &[u64]) {
        self.mul(out, a, &self.rr);
    }

    /// `a` out of the Montgomery domain.
    pub(crate) fn leave(&self, out: &mut [u64], a: &[u64]) {
        let mut one = [0u64; MAX_LIMBS];
        one[0] = 1;
        self.mul(out, a, &one[..self.n.len()]);
    }

    /// `out = a + b mod n`. Constant time.
    pub(crate) fn add(&self, out: &mut [u64], a: &[u64], b: &[u64]) {
        let k = self.n.len();
        let mut t = [0u64; MAX_LIMBS];
        let mut carry = 0u64;
        for j in 0..k {
            let v = a[j] as u128 + b[j] as u128 + carry as u128;
            t[j] = v as u64;
            carry = (v >> 64) as u64;
        }
        reduce_once(&mut out[..k], &t[..k], carry, &self.n);
    }

    /// `out = a - b mod n`. Constant time.
    pub(crate) fn sub(&self, out: &mut [u64], a: &[u64], b: &[u64]) {
        let k = self.n.len();
        let mut borrow = 0u64;
        let mut t = [0u64; MAX_LIMBS];
        for j in 0..k {
            let (d1, b1) = a[j].overflowing_sub(b[j]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            t[j] = d2;
            borrow = (b1 | b2) as u64;
        }
        // If a < b, add n back (masked, not branched).
        let mask = 0u64.wrapping_sub(borrow);
        let mut carry = 0u64;
        for j in 0..k {
            let v = t[j] as u128 + (self.n[j] & mask) as u128 + carry as u128;
            out[j] = v as u64;
            carry = (v >> 64) as u64;
        }
    }

    /// `base^exp mod n` where `exp` is **public** (variable time in `exp`).
    /// `base` and the result are in the Montgomery domain.
    pub(crate) fn pow_vartime(&self, out: &mut [u64], base: &[u64], exp: &[u64]) {
        let k = self.n.len();
        let mut acc = [0u64; MAX_LIMBS];
        acc[..k].copy_from_slice(&self.one);
        let mut tmp = [0u64; MAX_LIMBS];
        let mut started = false;
        for limb in exp.iter().rev() {
            for bit in (0..64).rev() {
                let set = (limb >> bit) & 1 == 1;
                if !started && !set {
                    continue;
                }
                started = true;
                tmp[..k].copy_from_slice(&acc[..k]);
                self.mul(&mut acc[..k], &tmp[..k], &tmp[..k]);
                if set {
                    tmp[..k].copy_from_slice(&acc[..k]);
                    self.mul(&mut acc[..k], &tmp[..k], base);
                }
            }
        }
        out[..k].copy_from_slice(&acc[..k]);
    }
}

/// `-x^{-1} mod 2^64` for odd `x` (Newton iteration).
fn neg_inv(x: u64) -> u64 {
    let mut inv = x; // correct to 3 bits for odd x
    for _ in 0..5 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(x.wrapping_mul(inv)));
    }
    inv.wrapping_neg()
}

/// `out = t - n` if `carry:t >= n`, else `t`; selected with a mask.
fn reduce_once(out: &mut [u64], t: &[u64], carry: u64, n: &[u64]) {
    let k = n.len();
    let mut d = [0u64; MAX_LIMBS];
    let mut borrow = 0u64;
    for j in 0..k {
        let (d1, b1) = t[j].overflowing_sub(n[j]);
        let (d2, b2) = d1.overflowing_sub(borrow);
        d[j] = d2;
        borrow = (b1 | b2) as u64;
    }
    // t >= n iff there is a carry out of the top limb, or no borrow.
    let take_sub = carry | (borrow ^ 1);
    let mask = 0u64.wrapping_sub(take_sub & 1);
    for j in 0..k {
        out[j] = (d[j] & mask) | (t[j] & !mask);
    }
}

/// `x = 2x mod n` for `x < n`. Public data only (used during setup).
fn double_mod(x: &mut [u64], n: &[u64]) {
    let k = n.len();
    let mut carry = 0u64;
    for limb in x.iter_mut() {
        let next = *limb >> 63;
        *limb = (*limb << 1) | carry;
        carry = next;
    }
    let t = x.to_vec();
    reduce_once(x, &t, carry, n);
    let _ = k;
}

/// Parses big-endian bytes into `limbs` little-endian limbs. `None` if the
/// value does not fit.
pub(crate) fn from_be_bytes(bytes: &[u8], limbs: usize) -> Option<Vec<u64>> {
    let mut out = vec![0u64; limbs];
    for (i, &b) in bytes.iter().rev().enumerate() {
        let limb = i / 8;
        if limb >= limbs {
            if b != 0 {
                return None;
            }
            continue;
        }
        out[limb] |= (b as u64) << (8 * (i % 8));
    }
    Some(out)
}

/// Writes `limbs` as big-endian bytes into `out` (zero-padded on the left).
/// `None` if it does not fit.
pub(crate) fn to_be_bytes(limbs: &[u64], out: &mut [u8]) -> Option<()> {
    out.fill(0);
    for (i, limb) in limbs.iter().enumerate() {
        for b in 0..8 {
            let byte = (limb >> (8 * b)) as u8;
            let pos = i * 8 + b;
            if pos >= out.len() {
                if byte != 0 {
                    return None;
                }
            } else {
                out[out.len() - 1 - pos] = byte;
            }
        }
    }
    Some(())
}

/// `a < b` for equal-length limb slices. Public data only.
pub(crate) fn lt_vartime(a: &[u64], b: &[u64]) -> bool {
    for j in (0..a.len()).rev() {
        if a[j] != b[j] {
            return a[j] < b[j];
        }
    }
    false
}

/// Whether every limb is zero. Public data only.
pub(crate) fn is_zero_vartime(a: &[u64]) -> bool {
    a.iter().all(|&x| x == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_rsa::BigUint;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn limbs(&mut self, k: usize) -> Vec<u64> {
            (0..k).map(|_| self.next()).collect()
        }
    }

    fn big(limbs: &[u64]) -> BigUint {
        let mut bytes = Vec::new();
        for l in limbs.iter().rev() {
            bytes.extend_from_slice(&l.to_be_bytes());
        }
        BigUint::from_bytes_be(&bytes)
    }

    fn odd_modulus(rng: &mut Rng, k: usize) -> Vec<u64> {
        let mut n = rng.limbs(k);
        n[0] |= 1;
        if n[k - 1] == 0 {
            n[k - 1] = 1 + rng.next() % 1000;
        }
        n
    }

    fn reduce(x: &mut [u64], n: &[u64]) {
        let r = big(x).rem(&big(n));
        let bytes = r.to_bytes_be_padded(x.len() * 8).unwrap();
        x.copy_from_slice(&from_be_bytes(&bytes, x.len()).unwrap());
    }

    #[test]
    fn mul_add_sub_match_biguint_across_sizes() {
        let mut rng = Rng(0xdead_beef_cafe_f00d);
        for k in [1usize, 2, 3, 4, 6, 8, 32, 64] {
            for _ in 0..20 {
                let n = odd_modulus(&mut rng, k);
                let m = Modulus::new(&n).unwrap();
                let (mut a, mut b) = (rng.limbs(k), rng.limbs(k));
                reduce(&mut a, &n);
                reduce(&mut b, &n);
                let (mut am, mut bm, mut pm, mut p) =
                    (vec![0; k], vec![0; k], vec![0; k], vec![0; k]);
                m.enter(&mut am, &a);
                m.enter(&mut bm, &b);
                m.mul(&mut pm, &am, &bm);
                m.leave(&mut p, &pm);
                let expect = big(&a).mulmod(&big(&b), &big(&n));
                assert_eq!(big(&p), expect, "mul k={k}");

                let mut s = vec![0; k];
                m.add(&mut s, &a, &b);
                assert_eq!(big(&s), big(&a).add(&big(&b)).rem(&big(&n)), "add k={k}");
                let mut d = vec![0; k];
                m.sub(&mut d, &a, &b);
                let expect = big(&a).add(&big(&n)).sub(&big(&b)).rem(&big(&n));
                assert_eq!(big(&d), expect, "sub k={k}");
            }
        }
    }

    #[test]
    fn one_is_r_mod_n_and_round_trips() {
        let mut rng = Rng(7);
        let n = odd_modulus(&mut rng, 4);
        let m = Modulus::new(&n).unwrap();
        let mut out = vec![0; 4];
        m.leave(&mut out, m.one());
        assert_eq!(out, vec![1, 0, 0, 0]);
    }

    #[test]
    fn pow_matches_biguint() {
        let mut rng = Rng(99);
        let n = odd_modulus(&mut rng, 8);
        let m = Modulus::new(&n).unwrap();
        let mut base = rng.limbs(8);
        reduce(&mut base, &n);
        let exp = [0x1_0001u64];
        let mut bm = vec![0; 8];
        m.enter(&mut bm, &base);
        let mut r = vec![0; 8];
        m.pow_vartime(&mut r, &bm, &exp);
        let mut out = vec![0; 8];
        m.leave(&mut out, &r);
        assert_eq!(
            big(&out),
            big(&base).modpow(&BigUint::from_u32(65537), &big(&n))
        );
    }

    #[test]
    fn rejects_bad_moduli() {
        assert!(Modulus::new(&[]).is_none());
        assert!(Modulus::new(&[4]).is_none());
        assert!(Modulus::new(&[3, 0]).is_none());
        assert!(Modulus::new(&vec![1u64; MAX_LIMBS + 1]).is_none());
        assert!(Modulus::new(&[3]).is_some());
    }

    #[test]
    fn byte_conversions_round_trip_and_bound() {
        let limbs = from_be_bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9], 2).unwrap();
        assert_eq!(limbs, vec![0x0203_0405_0607_0809, 1]);
        let mut out = [0u8; 9];
        to_be_bytes(&limbs, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(from_be_bytes(&[1, 0, 0, 0, 0, 0, 0, 0, 0], 1).is_none());
        assert!(to_be_bytes(&[0, 1], &mut [0u8; 8]).is_none());
    }
}
