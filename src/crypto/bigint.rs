//! A minimal unsigned big-integer type, used only for RSA signature
//! verification ([`crate::jwt::rsa`]). Deliberately small: it implements
//! exactly the operations RSA verification needs (multiply, modulo,
//! modular exponentiation with a small public exponent) and nothing more.

use std::cmp::Ordering;

/// An arbitrary-precision unsigned integer, stored little-endian in
/// base-2^32 limbs with no leading (i.e. trailing, in this order) zero
/// limbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigUint {
    limbs: Vec<u32>,
}

impl BigUint {
    pub fn zero() -> Self {
        BigUint { limbs: vec![] }
    }

    pub fn from_u32(v: u32) -> Self {
        if v == 0 {
            BigUint::zero()
        } else {
            BigUint { limbs: vec![v] }
        }
    }

    /// Parses a big-endian byte string (as used by JWK `n`/`e` values and
    /// PKCS#1 signatures) into a `BigUint`.
    pub fn from_bytes_be(bytes: &[u8]) -> Self {
        let mut limbs = Vec::with_capacity(bytes.len() / 4 + 1);
        let mut chunks = bytes.rchunks(4);
        for chunk in &mut chunks {
            let mut buf = [0u8; 4];
            buf[4 - chunk.len()..].copy_from_slice(chunk);
            limbs.push(u32::from_be_bytes(buf));
        }
        let mut n = BigUint { limbs };
        n.trim();
        n
    }

    /// Encodes this integer as a big-endian byte string, left-padded
    /// with zeros to exactly `len` bytes (RFC 8017's `I2OSP`). Returns
    /// `None` if the value doesn't fit in `len` bytes.
    pub fn to_bytes_be_padded(&self, len: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; self.limbs.len() * 4];
        for (i, limb) in self.limbs.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&limb.to_le_bytes());
        }
        // `out` is little-endian bytes; reverse to big-endian and strip
        // any leading zero bytes from the most-significant end.
        out.reverse();
        while out.first() == Some(&0) && out.len() > 1 {
            out.remove(0);
        }
        if out == [0] {
            out.clear();
        }
        if out.len() > len {
            return None;
        }
        let mut padded = vec![0u8; len - out.len()];
        padded.extend_from_slice(&out);
        Some(padded)
    }

    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    fn trim(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    pub fn bit_length(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(top) => (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    /// Compares two values by magnitude.
    pub fn compare(&self, other: &BigUint) -> Ordering {
        if self.limbs.len() != other.limbs.len() {
            return self.limbs.len().cmp(&other.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            if self.limbs[i] != other.limbs[i] {
                return self.limbs[i].cmp(&other.limbs[i]);
            }
        }
        Ordering::Equal
    }

    /// `self - other`, assuming `self >= other`.
    fn sub(&self, other: &BigUint) -> BigUint {
        let mut result = Vec::with_capacity(self.limbs.len());
        let mut borrow: i64 = 0;
        for i in 0..self.limbs.len() {
            let a = self.limbs[i] as i64;
            let b = *other.limbs.get(i).unwrap_or(&0) as i64;
            let mut diff = a - b - borrow;
            if diff < 0 {
                diff += 1i64 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            result.push(diff as u32);
        }
        let mut r = BigUint { limbs: result };
        r.trim();
        r
    }

    /// `self * other`, schoolbook multiplication.
    fn mul(&self, other: &BigUint) -> BigUint {
        if self.is_zero() || other.is_zero() {
            return BigUint::zero();
        }
        let mut result = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            if a == 0 {
                continue;
            }
            let mut carry: u64 = 0;
            for (j, &b) in other.limbs.iter().enumerate() {
                let idx = i + j;
                let product = a as u64 * b as u64 + result[idx] as u64 + carry;
                result[idx] = product as u32;
                carry = product >> 32;
            }
            let mut idx = i + other.limbs.len();
            while carry > 0 {
                let sum = result[idx] as u64 + carry;
                result[idx] = sum as u32;
                carry = sum >> 32;
                idx += 1;
            }
        }
        let mut r = BigUint { limbs: result };
        r.trim();
        r
    }

    /// Shifts left by one bit.
    fn shl1(&self) -> BigUint {
        let mut result = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry = 0u32;
        for &limb in &self.limbs {
            let new_carry = limb >> 31;
            result.push((limb << 1) | carry);
            carry = new_carry;
        }
        if carry != 0 {
            result.push(carry);
        }
        let mut r = BigUint { limbs: result };
        r.trim();
        r
    }

    /// Returns `self`'s bit at position `i` (0 = least significant).
    fn bit(&self, i: usize) -> bool {
        let limb = i / 32;
        let offset = i % 32;
        self.limbs
            .get(limb)
            .map(|l| (l >> offset) & 1 == 1)
            .unwrap_or(false)
    }

    /// `self / divisor`, `self % divisor`, via binary long division.
    /// `divisor` must be non-zero.
    fn divmod(&self, divisor: &BigUint) -> (BigUint, BigUint) {
        assert!(!divisor.is_zero(), "division by zero");
        if self.compare(divisor) == Ordering::Less {
            return (BigUint::zero(), self.clone());
        }
        let mut quotient = vec![0u32; self.limbs.len()];
        let mut remainder = BigUint::zero();
        for i in (0..self.bit_length()).rev() {
            remainder = remainder.shl1();
            if self.bit(i) {
                if remainder.limbs.is_empty() {
                    remainder.limbs.push(1);
                } else {
                    remainder.limbs[0] |= 1;
                }
            }
            if remainder.compare(divisor) != Ordering::Less {
                remainder = remainder.sub(divisor);
                quotient[i / 32] |= 1 << (i % 32);
            }
        }
        let mut q = BigUint { limbs: quotient };
        q.trim();
        (q, remainder)
    }

    /// `self % modulus`.
    pub fn rem(&self, modulus: &BigUint) -> BigUint {
        self.divmod(modulus).1
    }

    /// Modular exponentiation: `self^exponent mod modulus`, via
    /// square-and-multiply. Efficient as long as `exponent` is small
    /// (true for RSA public exponents like 65537) -- this is not a
    /// constant-time implementation and must never be used with a
    /// secret exponent (i.e. never for RSA *signing*, only verification).
    pub fn modpow(&self, exponent: &BigUint, modulus: &BigUint) -> BigUint {
        if modulus.compare(&BigUint::from_u32(1)) == Ordering::Equal {
            return BigUint::zero();
        }
        let mut result = BigUint::from_u32(1);
        let mut base = self.rem(modulus);
        for i in 0..exponent.bit_length() {
            if exponent.bit(i) {
                result = result.mul(&base).rem(modulus);
            }
            base = base.mul(&base).rem(modulus);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_roundtrip() {
        let bytes = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let n = BigUint::from_bytes_be(&bytes);
        assert_eq!(n.to_bytes_be_padded(8).unwrap(), bytes);
    }

    #[test]
    fn padding_and_stripping() {
        let n = BigUint::from_bytes_be(&[0x00, 0x00, 0x01]);
        assert_eq!(n.to_bytes_be_padded(1).unwrap(), vec![0x01]);
        assert_eq!(
            n.to_bytes_be_padded(4).unwrap(),
            vec![0x00, 0x00, 0x00, 0x01]
        );
    }

    #[test]
    fn multiplication() {
        let a = BigUint::from_u32(123456789);
        let b = BigUint::from_u32(987654321);
        let product = a.mul(&b);
        // 123456789 * 987654321 = 121932631112635269
        let expected = BigUint::from_bytes_be(&121932631112635269u64.to_be_bytes());
        assert_eq!(product, expected);
    }

    #[test]
    fn division_and_remainder() {
        let a = BigUint::from_bytes_be(&1_000_000_007u64.to_be_bytes());
        let b = BigUint::from_u32(97);
        let (q, r) = a.divmod(&b);
        // 1000000007 = 97 * 10309278 + 41
        assert_eq!(q, BigUint::from_u32(10309278));
        assert_eq!(r, BigUint::from_u32(41));
    }

    #[test]
    fn modpow_small() {
        // 4^13 mod 497 = 445 (textbook RSA example)
        let base = BigUint::from_u32(4);
        let exp = BigUint::from_u32(13);
        let modulus = BigUint::from_u32(497);
        assert_eq!(base.modpow(&exp, &modulus), BigUint::from_u32(445));
    }

    #[test]
    fn modpow_matches_python_for_large_values() {
        // pow(123456789, 65537, 1000000000000000003) computed independently.
        let base = BigUint::from_bytes_be(&123456789u64.to_be_bytes());
        let exp = BigUint::from_u32(65537);
        let modulus = BigUint::from_bytes_be(&1_000_000_000_000_000_003u64.to_be_bytes());
        let result = base.modpow(&exp, &modulus);
        let expected = BigUint::from_bytes_be(&269114687631967892u64.to_be_bytes());
        assert_eq!(result, expected);
    }
}
