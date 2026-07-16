//! A minimal unsigned big integer, just enough for RSA public-key encryption.
//!
//! RDP standard security encrypts the 32-byte client random with the server's
//! RSA public key: `c = m^e mod n`. That is the only operation needed, so this
//! type implements comparison, schoolbook multiplication, and a bitwise
//! modular reduction — no division, no signed arithmetic, no general math.
//!
//! Limbs are `u32`, stored little-endian (least-significant limb first), which
//! matches the little-endian byte order RDP uses for RSA moduli and randoms.

/// An arbitrary-precision unsigned integer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigUint {
    /// Little-endian limbs with no trailing zero limbs (except the value 0,
    /// which is the empty vector).
    limbs: Vec<u32>,
}

impl BigUint {
    /// The value zero.
    pub fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }

    /// The value one.
    pub fn one() -> Self {
        BigUint { limbs: vec![1] }
    }

    /// Build from little-endian bytes.
    pub fn from_bytes_le(bytes: &[u8]) -> Self {
        let mut limbs = Vec::with_capacity(bytes.len().div_ceil_(4));
        for chunk in bytes.chunks(4) {
            let mut limb = 0u32;
            for (i, &b) in chunk.iter().enumerate() {
                limb |= (b as u32) << (8 * i);
            }
            limbs.push(limb);
        }
        let mut n = BigUint { limbs };
        n.normalize();
        n
    }

    /// Build from big-endian bytes.
    pub fn from_bytes_be(bytes: &[u8]) -> Self {
        let mut le = bytes.to_vec();
        le.reverse();
        BigUint::from_bytes_le(&le)
    }

    /// Serialise to little-endian bytes, zero-padded to exactly `len` bytes.
    ///
    /// Returns `None` if the value does not fit in `len` bytes.
    pub fn to_bytes_le(&self, len: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; len];
        for (i, &limb) in self.limbs.iter().enumerate() {
            for byte in 0..4 {
                let idx = i * 4 + byte;
                let value = (limb >> (8 * byte)) as u8;
                if idx >= len {
                    if value != 0 {
                        return None;
                    }
                } else {
                    out[idx] = value;
                }
            }
        }
        Some(out)
    }

    /// Returns `true` if the value is zero.
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn normalize(&mut self) {
        while let Some(&0) = self.limbs.last() {
            self.limbs.pop();
        }
    }

    fn bit_len(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    fn bit(&self, i: usize) -> bool {
        let limb = i / 32;
        let off = i % 32;
        self.limbs.get(limb).is_some_and(|&l| (l >> off) & 1 == 1)
    }

    fn cmp(&self, other: &BigUint) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            match self.limbs[i].cmp(&other.limbs[i]) {
                Ordering::Equal => continue,
                non_eq => return non_eq,
            }
        }
        Ordering::Equal
    }

    /// Shift left by one bit, setting the low bit to `carry_in`.
    fn shl1_or(&mut self, carry_in: bool) {
        let mut carry = carry_in as u32;
        for limb in self.limbs.iter_mut() {
            let new_carry = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = new_carry;
        }
        if carry != 0 {
            self.limbs.push(carry);
        }
    }

    /// Subtract `other` from `self` in place, assuming `self >= other`.
    fn sub_assign(&mut self, other: &BigUint) {
        let mut borrow = 0i64;
        for i in 0..self.limbs.len() {
            let rhs = *other.limbs.get(i).unwrap_or(&0) as i64;
            let mut diff = self.limbs[i] as i64 - rhs - borrow;
            if diff < 0 {
                diff += 1 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            self.limbs[i] = diff as u32;
        }
        self.normalize();
    }

    /// Schoolbook multiply.
    fn mul(&self, other: &BigUint) -> BigUint {
        if self.is_zero() || other.is_zero() {
            return BigUint::zero();
        }
        let mut result = vec![0u64; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.limbs.iter().enumerate() {
                let cur = result[i + j] + (a as u64) * (b as u64) + carry;
                result[i + j] = cur & 0xFFFF_FFFF;
                carry = cur >> 32;
            }
            result[i + other.limbs.len()] += carry;
        }
        let mut n = BigUint {
            limbs: result.into_iter().map(|v| v as u32).collect(),
        };
        n.normalize();
        n
    }

    /// Reduce `self` modulo `modulus` (bitwise long division remainder).
    fn rem(&self, modulus: &BigUint) -> BigUint {
        use core::cmp::Ordering;
        debug_assert!(!modulus.is_zero());
        let mut rem = BigUint::zero();
        for i in (0..self.bit_len()).rev() {
            rem.shl1_or(self.bit(i));
            if rem.cmp(modulus) != Ordering::Less {
                rem.sub_assign(modulus);
            }
        }
        rem
    }

    /// Modular multiplication: `(self * other) mod modulus`.
    pub fn mulmod(&self, other: &BigUint, modulus: &BigUint) -> BigUint {
        self.mul(other).rem(modulus)
    }

    /// Modular exponentiation: `self^exp mod modulus` (square-and-multiply).
    pub fn modpow(&self, exp: &BigUint, modulus: &BigUint) -> BigUint {
        if modulus.is_zero() {
            return BigUint::zero();
        }
        let mut result = BigUint::one().rem(modulus);
        let mut base = self.rem(modulus);
        for i in 0..exp.bit_len() {
            if exp.bit(i) {
                result = result.mulmod(&base, modulus);
            }
            base = base.mulmod(&base, modulus);
        }
        result
    }
}

/// Local `div_ceil` to keep the crate's MSRV below 1.73.
trait DivCeil {
    fn div_ceil_(self, d: usize) -> usize;
}
impl DivCeil for usize {
    fn div_ceil_(self, d: usize) -> usize {
        (self + d - 1) / d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_u64(v: u64) -> BigUint {
        BigUint::from_bytes_le(&v.to_le_bytes())
    }

    #[test]
    fn byte_roundtrip_le_and_be() {
        let bytes = [0x01u8, 0x02, 0x03, 0x04, 0x05];
        let n = BigUint::from_bytes_le(&bytes);
        assert_eq!(n.to_bytes_le(5).unwrap(), bytes);
        let be = BigUint::from_bytes_be(&[0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(n, be);
    }

    #[test]
    fn to_bytes_reports_overflow() {
        let n = from_u64(0x1_0000); // needs 3 bytes
        assert!(n.to_bytes_le(1).is_none());
        assert_eq!(n.to_bytes_le(3).unwrap(), [0x00, 0x00, 0x01]);
    }

    #[test]
    fn modpow_small() {
        // 4^13 mod 497 = 445
        assert_eq!(
            from_u64(4).modpow(&from_u64(13), &from_u64(497)),
            from_u64(445)
        );
        // 2^10 mod 1000 = 24
        assert_eq!(
            from_u64(2).modpow(&from_u64(10), &from_u64(1000)),
            from_u64(24)
        );
    }

    #[test]
    fn rsa_textbook_roundtrip() {
        // n = 3233 = 61*53, e = 17, d = 413.
        let n = from_u64(3233);
        let e = from_u64(17);
        let d = from_u64(413);
        let m = from_u64(65);
        let c = m.modpow(&e, &n);
        assert_eq!(c, from_u64(2790));
        assert_eq!(c.modpow(&d, &n), m); // decrypt recovers the message
    }

    #[test]
    fn multiplication_carry() {
        // (2^32 - 1)^2 = 0xFFFFFFFE00000001
        let max = from_u64(0xFFFF_FFFF);
        let product = max.modpow(&from_u64(2), &from_u64(u64::MAX));
        // (2^32-1)^2 mod (2^64-1)
        let expected = from_u64(0xFFFF_FFFE_0000_0001u64.wrapping_rem(u64::MAX));
        assert_eq!(product, expected);
    }
}
