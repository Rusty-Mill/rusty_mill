//! Exact parts of a layout's length, for judging focus moves.

use super::MAX_DEPTH;

/// A length or position along a layout, the whole being
/// 100^[`MAX_DEPTH`]: every split takes a whole percentage, so a layout
/// [`MAX_DEPTH`] deep divides it without remainder, where a float would
/// lose blocks far smaller than its precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Exact([u64; 4]); // Most significant first, so Ord is numeric.

impl Exact {
    pub(super) const ZERO: Self = Self([0; 4]);

    /// The whole layout, 100^[`MAX_DEPTH`] < 2^213.
    pub(super) fn whole() -> Self {
        (0..MAX_DEPTH).fold(Self([0, 0, 0, 1]), |n, _| n.times(100))
    }

    /// `percent` hundredths of `self`, rounded down.
    pub(super) fn percent(self, percent: u8) -> Self {
        self.over(100).times(u64::from(percent))
    }

    pub(super) fn plus(self, other: Self) -> Self {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for i in (0..4).rev() {
            let sum = u128::from(self.0[i]) + u128::from(other.0[i]) + carry;
            out[i] = sum as u64; // Low half; the high half carries.
            carry = sum >> 64;
        }
        Self(out)
    }

    /// `self - other`, or zero when `other` is larger.
    pub(super) fn minus(self, other: Self) -> Self {
        if other >= self {
            return Self::ZERO;
        }
        let mut out = [0; 4];
        let mut borrow = false;
        for i in (0..4).rev() {
            let (d, b1) = self.0[i].overflowing_sub(other.0[i]);
            let (d, b2) = d.overflowing_sub(u64::from(borrow));
            out[i] = d;
            borrow = b1 || b2;
        }
        Self(out)
    }

    fn times(self, k: u64) -> Self {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for i in (0..4).rev() {
            let product = u128::from(self.0[i]) * u128::from(k) + carry;
            out[i] = product as u64; // Low half; the high half carries.
            carry = product >> 64;
        }
        Self(out)
    }

    fn over(self, k: u64) -> Self {
        let mut out = [0; 4];
        let mut rest = 0u128;
        for (i, limb) in self.0.iter().enumerate() {
            let n = (rest << 64) | u128::from(*limb);
            out[i] = (n / u128::from(k)) as u64; // rest < k, so it fits.
            rest = n % u128::from(k);
        }
        Self(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_divides_evenly_to_the_deepest_split() {
        let deepest = (0..MAX_DEPTH).fold(Exact::whole(), |n, _| n.percent(1));
        assert_eq!(deepest, Exact([0, 0, 0, 1]));
        let back = (0..MAX_DEPTH).fold(deepest, |n, _| n.times(100));
        assert_eq!(back, Exact::whole());
    }

    #[test]
    fn sums_and_differences_carry_across_limbs() {
        let max = Exact([0, 0, 0, u64::MAX]);
        let one = Exact([0, 0, 0, 1]);
        assert_eq!(max.plus(one), Exact([0, 0, 1, 0]));
        assert_eq!(Exact([0, 0, 1, 0]).minus(one), max);
        assert_eq!(one.minus(max), Exact::ZERO);
        assert!(Exact([0, 0, 1, 0]) > max);
    }
}
