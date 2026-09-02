//! A minimal unsigned big-integer type: add, subtract, multiply, modulo,
//! modular exponentiation, and byte conversion in both big- and
//! little-endian (the two callers disagree on which is "native" -- JWK
//! `n`/`e` values and PKCS#1 signatures are big-endian; RDP's RSA moduli
//! and randoms are little-endian). Not constant-time -- see the security
//! note on [`modpow`](BigUint::modpow).

use std::cmp::Ordering;

/// An arbitrary-precision unsigned integer, stored little-endian in
/// base-2^32 limbs with no trailing zero limbs (the value zero is the
/// empty vector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigUint {
    limbs: Vec<u32>,
}

impl BigUint {
    pub fn zero() -> Self {
        BigUint { limbs: vec![] }
    }

    pub fn one() -> Self {
        BigUint { limbs: vec![1] }
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

    /// Parses a little-endian byte string (as used by RDP's RSA moduli
    /// and randoms) into a `BigUint`.
    pub fn from_bytes_le(bytes: &[u8]) -> Self {
        // `(len + 3) / 4` rather than `len.div_ceil(4)` -- `rusty_rdp`, one
        // of this crate's two consumers, holds its MSRV below 1.73 (where
        // `div_ceil` stabilized), and this crate follows the more
        // conservative of the two.
        let mut limbs = Vec::with_capacity((bytes.len() + 3) / 4);
        for chunk in bytes.chunks(4) {
            let mut buf = [0u8; 4];
            buf[..chunk.len()].copy_from_slice(chunk);
            limbs.push(u32::from_le_bytes(buf));
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

    /// Encodes this integer as a little-endian byte string, zero-padded
    /// to exactly `len` bytes. Returns `None` if the value doesn't fit
    /// in `len` bytes.
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

    /// `self + other`.
    pub fn add(&self, other: &BigUint) -> BigUint {
        let len = self.limbs.len().max(other.limbs.len());
        let mut result = Vec::with_capacity(len + 1);
        let mut carry = 0u64;
        for i in 0..len {
            let a = *self.limbs.get(i).unwrap_or(&0) as u64;
            let b = *other.limbs.get(i).unwrap_or(&0) as u64;
            let sum = a + b + carry;
            result.push(sum as u32);
            carry = sum >> 32;
        }
        if carry > 0 {
            result.push(carry as u32);
        }
        let mut r = BigUint { limbs: result };
        r.trim();
        r
    }

    /// `self - other`, assuming `self >= other`.
    pub fn sub(&self, other: &BigUint) -> BigUint {
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
    pub fn mul(&self, other: &BigUint) -> BigUint {
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
    pub fn bit(&self, i: usize) -> bool {
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

    /// `self % modulus`. `modulus` must be non-zero.
    pub fn rem(&self, modulus: &BigUint) -> BigUint {
        self.divmod(modulus).1
    }

    /// `(self * other) mod modulus`.
    pub fn mulmod(&self, other: &BigUint, modulus: &BigUint) -> BigUint {
        self.mul(other).rem(modulus)
    }

    /// Modular exponentiation: `self^exponent mod modulus`, via
    /// square-and-multiply. Efficient as long as `exponent` is small
    /// (true for RSA public exponents like 65537) -- this is not a
    /// constant-time implementation and must never be used with a
    /// secret exponent (i.e. never for RSA *signing* or *decryption*,
    /// only verification/encryption with a public exponent).
    pub fn modpow(&self, exponent: &BigUint, modulus: &BigUint) -> BigUint {
        if modulus.is_zero() || modulus.compare(&BigUint::one()) == Ordering::Equal {
            return BigUint::zero();
        }
        let mut result = BigUint::one();
        let mut base = self.rem(modulus);
        for i in 0..exponent.bit_length() {
            if exponent.bit(i) {
                result = result.mulmod(&base, modulus);
            }
            base = base.mulmod(&base, modulus);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_u64(v: u64) -> BigUint {
        BigUint::from_bytes_be(&v.to_be_bytes())
    }

    #[test]
    fn bytes_roundtrip_be() {
        let bytes = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let n = BigUint::from_bytes_be(&bytes);
        assert_eq!(n.to_bytes_be_padded(8).unwrap(), bytes);
    }

    #[test]
    fn bytes_roundtrip_le_and_cross_endian_equality() {
        let bytes = [0x01u8, 0x02, 0x03, 0x04, 0x05];
        let n = BigUint::from_bytes_le(&bytes);
        assert_eq!(n.to_bytes_le(5).unwrap(), bytes);
        let be = BigUint::from_bytes_be(&[0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(n, be);
    }

    #[test]
    fn to_bytes_le_reports_overflow() {
        let n = from_u64(0x1_0000); // needs 3 bytes
        assert!(n.to_bytes_le(1).is_none());
        assert_eq!(n.to_bytes_le(3).unwrap(), [0x00, 0x00, 0x01]);
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
    fn addition() {
        let a = BigUint::from_bytes_be(&[0xff, 0xff, 0xff, 0xff]);
        let b = BigUint::from_u32(1);
        // 0xffffffff + 1 overflows a single limb.
        assert_eq!(
            a.add(&b),
            BigUint::from_bytes_be(&[0x01, 0x00, 0x00, 0x00, 0x00])
        );

        let c = BigUint::from_u32(123456789);
        let d = BigUint::from_u32(987654321);
        assert_eq!(c.add(&d), BigUint::from_u32(1111111110));
    }

    #[test]
    fn multiplication() {
        let a = BigUint::from_u32(123456789);
        let b = BigUint::from_u32(987654321);
        let product = a.mul(&b);
        // 123456789 * 987654321 = 121932631112635269
        let expected = from_u64(121932631112635269);
        assert_eq!(product, expected);
    }

    #[test]
    fn multiplication_carry() {
        // (2^32 - 1)^2 mod (2^64 - 1) -- exercises carry propagation
        // across limbs during both the multiply and the reduction.
        let max = from_u64(0xFFFF_FFFF);
        let product = max.modpow(&from_u64(2), &from_u64(u64::MAX));
        let expected = from_u64(0xFFFF_FFFE_0000_0001u64.wrapping_rem(u64::MAX));
        assert_eq!(product, expected);
    }

    #[test]
    fn division_and_remainder() {
        let a = from_u64(1_000_000_007);
        let b = BigUint::from_u32(97);
        let (q, r) = a.divmod(&b);
        // 1000000007 = 97 * 10309278 + 41
        assert_eq!(q, BigUint::from_u32(10309278));
        assert_eq!(r, BigUint::from_u32(41));
    }

    #[test]
    fn modpow_small() {
        // 4^13 mod 497 = 445 (textbook RSA example)
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
    fn modpow_matches_python_for_large_values() {
        // pow(123456789, 65537, 1000000000000000003) computed independently.
        let base = from_u64(123456789);
        let exp = BigUint::from_u32(65537);
        let modulus = from_u64(1_000_000_000_000_000_003);
        let result = base.modpow(&exp, &modulus);
        let expected = from_u64(269114687631967892);
        assert_eq!(result, expected);
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
    fn modpow_by_zero_or_one_modulus_returns_zero_rather_than_panicking() {
        assert_eq!(
            from_u64(5).modpow(&from_u64(3), &BigUint::zero()),
            BigUint::zero()
        );
        assert_eq!(
            from_u64(5).modpow(&from_u64(3), &BigUint::one()),
            BigUint::zero()
        );
    }
}
