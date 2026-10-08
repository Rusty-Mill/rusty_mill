//! Prime-field elements on top of [`Modulus`](crate::mont::Modulus).
//!
//! An element is up to six limbs (384 bits). Arithmetic is Montgomery-domain
//! and branch-free; the `*_vartime` routines are for public values only.

use crate::mont::{self, Modulus};

/// Limbs per element (P-384 is the widest curve).
pub(crate) const LIMBS: usize = 6;

/// A field element or scalar; unused high limbs are zero.
#[derive(Clone, Copy, Debug)]
pub(crate) struct E(pub(crate) [u64; LIMBS]);

impl E {
    pub(crate) const ZERO: E = E([0; LIMBS]);

    /// Zero-extends `limbs` (at most [`LIMBS`] of them).
    pub(crate) fn from_limbs(limbs: &[u64]) -> E {
        let mut out = [0; LIMBS];
        out[..limbs.len()].copy_from_slice(limbs);
        E(out)
    }
}

/// A prime field (or scalar field) with `k` active limbs.
pub(crate) struct Field {
    m: Modulus,
    k: usize,
}

impl Field {
    /// The field of integers modulo the odd `n` (`n.len() <= LIMBS`).
    pub(crate) fn new(n: &[u64]) -> Option<Self> {
        if n.len() > LIMBS {
            return None;
        }
        Some(Self {
            m: Modulus::new(n)?,
            k: n.len(),
        })
    }

    pub(crate) fn modulus(&self) -> E {
        E::from_limbs(self.m.modulus())
    }

    /// The Montgomery form of 1.
    pub(crate) fn one(&self) -> E {
        E::from_limbs(self.m.one())
    }

    pub(crate) fn enter(&self, a: &E) -> E {
        let mut out = E::ZERO;
        self.m.enter(&mut out.0[..self.k], &a.0[..self.k]);
        out
    }

    pub(crate) fn leave(&self, a: &E) -> E {
        let mut out = E::ZERO;
        self.m.leave(&mut out.0[..self.k], &a.0[..self.k]);
        out
    }

    pub(crate) fn mul(&self, a: &E, b: &E) -> E {
        let mut out = E::ZERO;
        self.m
            .mul(&mut out.0[..self.k], &a.0[..self.k], &b.0[..self.k]);
        out
    }

    pub(crate) fn sqr(&self, a: &E) -> E {
        self.mul(a, a)
    }

    pub(crate) fn add(&self, a: &E, b: &E) -> E {
        let mut out = E::ZERO;
        self.m
            .add(&mut out.0[..self.k], &a.0[..self.k], &b.0[..self.k]);
        out
    }

    pub(crate) fn sub(&self, a: &E, b: &E) -> E {
        let mut out = E::ZERO;
        self.m
            .sub(&mut out.0[..self.k], &a.0[..self.k], &b.0[..self.k]);
        out
    }

    pub(crate) fn neg(&self, a: &E) -> E {
        self.sub(&E::ZERO, a)
    }

    /// `a^exp` with a **public** exponent.
    pub(crate) fn pow_vartime(&self, a: &E, exp: &[u64]) -> E {
        let mut out = E::ZERO;
        self.m
            .pow_vartime(&mut out.0[..self.k], &a.0[..self.k], exp);
        out
    }

    /// `a^-1` by Fermat's little theorem (prime modulus). Variable time in
    /// the (public) exponent only; `a` is not branched on.
    pub(crate) fn invert(&self, a: &E) -> E {
        let mut exp = self.modulus().0;
        exp[0] -= 2; // the modulus is odd and at least 3, so no borrow
        self.pow_vartime(a, &exp[..self.k])
    }

    pub(crate) fn is_zero_vartime(&self, a: &E) -> bool {
        mont::is_zero_vartime(&a.0[..self.k])
    }

    pub(crate) fn eq_vartime(&self, a: &E, b: &E) -> bool {
        a.0[..self.k] == b.0[..self.k]
    }

    /// `a < modulus`, for validating public input.
    pub(crate) fn is_reduced_vartime(&self, a: &E) -> bool {
        mont::lt_vartime(&a.0[..self.k], self.m.modulus())
    }

    /// Parses exactly `len` big-endian bytes (`len == 8*k` rounded to the
    /// curve's byte length), without reducing.
    pub(crate) fn parse_be(&self, bytes: &[u8]) -> Option<E> {
        Some(E::from_limbs(&mont::from_be_bytes(bytes, self.k)?))
    }

    /// Writes the value as `out.len()` big-endian bytes.
    pub(crate) fn to_be_bytes(&self, a: &E, out: &mut [u8]) -> Option<()> {
        mont::to_be_bytes(&a.0[..self.k], out)
    }
    /// Swaps `a` and `b` if `swap` is 1 and leaves them if it is 0, with no
    /// branch or index depending on `swap`. Constant time.
    pub(crate) fn cswap(a: &mut E, b: &mut E, swap: u64) {
        let mask = core::hint::black_box(0u64.wrapping_sub(swap & 1));
        for i in 0..LIMBS {
            let t = mask & (a.0[i] ^ b.0[i]);
            a.0[i] ^= t;
            b.0[i] ^= t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::P256_P;

    #[test]
    fn inversion_and_arithmetic_round_trip() {
        let f = Field::new(&P256_P).unwrap();
        let a = f.enter(&E::from_limbs(&[0x1234_5678_9abc_def0, 7, 0, 0]));
        let inv = f.invert(&a);
        let one = f.leave(&f.mul(&a, &inv));
        assert_eq!(one.0, [1, 0, 0, 0, 0, 0]);
        let zero = f.add(&a, &f.neg(&a));
        assert!(f.is_zero_vartime(&zero));
    }

    #[test]
    fn cswap_swaps_only_when_asked() {
        let (mut a, mut b) = (E([1; LIMBS]), E([2; LIMBS]));
        Field::cswap(&mut a, &mut b, 0);
        assert_eq!((a.0[0], b.0[0]), (1, 2));
        Field::cswap(&mut a, &mut b, 1);
        assert_eq!((a.0[0], b.0[0]), (2, 1));
        Field::cswap(&mut a, &mut b, 3); // only the low bit counts
        assert_eq!((a.0[0], b.0[0]), (1, 2));
    }

    #[test]
    fn rejects_oversized_moduli() {
        assert!(Field::new(&[1; LIMBS + 1]).is_none());
    }
}
