//! GHASH (NIST SP 800-38D section 6.4) without a table.
//!
//! The carry-less 64x64 product is built from ordinary integer multiplies on operands whose
//! set bits are four positions apart, so partial sums never carry into a neighbouring bit
//! (the technique BearSSL calls `bmul64`). No table is indexed by the hash key or the data
//! and nothing branches on either. **Assumption:** a 64-bit integer multiply takes
//! data-independent time on the targets this crate is used on (x86-64 and aarch64
//! application cores). That is not true of every CPU and is recorded as an assumption of the
//! design, not as something this code establishes.

/// Low 64 bits of the carry-less product of `x` and `y`.
fn bmul64(x: u64, y: u64) -> u64 {
    const M: [u64; 4] = [
        0x1111_1111_1111_1111,
        0x2222_2222_2222_2222,
        0x4444_4444_4444_4444,
        0x8888_8888_8888_8888,
    ];
    let (x0, x1, x2, x3) = (x & M[0], x & M[1], x & M[2], x & M[3]);
    let (y0, y1, y2, y3) = (y & M[0], y & M[1], y & M[2], y & M[3]);
    let z0 = x0.wrapping_mul(y0) ^ x1.wrapping_mul(y3) ^ x2.wrapping_mul(y2) ^ x3.wrapping_mul(y1);
    let z1 = x0.wrapping_mul(y1) ^ x1.wrapping_mul(y0) ^ x2.wrapping_mul(y3) ^ x3.wrapping_mul(y2);
    let z2 = x0.wrapping_mul(y2) ^ x1.wrapping_mul(y1) ^ x2.wrapping_mul(y0) ^ x3.wrapping_mul(y3);
    let z3 = x0.wrapping_mul(y3) ^ x1.wrapping_mul(y2) ^ x2.wrapping_mul(y1) ^ x3.wrapping_mul(y0);
    (z0 & M[0]) | (z1 & M[1]) | (z2 & M[2]) | (z3 & M[3])
}

/// The full 128-bit carry-less product. The high half comes from the low half of the product
/// of the bit-reversed operands: reversing within 64 bits maps product bit `k` to `126 - k`.
fn clmul64(a: u64, b: u64) -> u128 {
    let lo = bmul64(a, b);
    let hi = bmul64(a.reverse_bits(), b.reverse_bits()).reverse_bits() >> 1;
    (u128::from(hi) << 64) | u128::from(lo)
}

/// Multiplication in GF(2^128) as GCM defines it. `a` and `b` are blocks read big-endian: the
/// most significant bit is the coefficient of x^0, so reversing the bits gives ordinary
/// polynomials (bit `i` is the coefficient of x^i), and the result is reversed back.
pub(crate) fn gf_mul(a: u128, b: u128) -> u128 {
    let (a, b) = (a.reverse_bits(), b.reverse_bits());
    let (al, ah, bl, bh) = (a as u64, (a >> 64) as u64, b as u64, (b >> 64) as u64);
    // Karatsuba: c = ll + mid * x^64 + hh * x^128.
    let ll = clmul64(al, bl);
    let hh = clmul64(ah, bh);
    let mid = clmul64(al ^ ah, bl ^ bh) ^ ll ^ hh;
    let low = ll ^ (mid << 64);
    let high = hh ^ (mid >> 64);
    // Reduce by x^128 = x^7 + x^2 + x + 1. Shifting `high` left drops its top bits; those are
    // folded in a second, smaller step (they are at most 7 bits wide).
    let overflow = (high >> 127) ^ (high >> 126) ^ (high >> 121);
    let mut r = low ^ high ^ (high << 1) ^ (high << 2) ^ (high << 7);
    r ^= overflow ^ (overflow << 1) ^ (overflow << 2) ^ (overflow << 7);
    r.reverse_bits()
}

/// GHASH under hash key `h` over `aad` and `ct`, each zero-padded to 16 bytes, followed by the
/// 64-bit bit lengths of both. Lengths are public.
pub(crate) fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> u128 {
    let mut y = 0u128;
    for part in [aad, ct] {
        for chunk in part.chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            y = gf_mul(y ^ u128::from_be_bytes(block), h);
        }
    }
    let lens = ((aad.len() as u128 * 8) << 64) | (ct.len() as u128 * 8);
    gf_mul(y ^ lens, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bit-by-bit multiplication from the specification (SP 800-38D algorithm 1). Variable
    /// time; an oracle for tests only.
    fn gf_mul_ref(x: u128, y: u128) -> u128 {
        let mut z = 0u128;
        let mut v = y;
        for i in 0..128 {
            if (x >> (127 - i)) & 1 == 1 {
                z ^= v;
            }
            let lsb = v & 1;
            v >>= 1;
            if lsb == 1 {
                v ^= 0xe1 << 120;
            }
        }
        z
    }

    #[test]
    fn matches_the_specification_algorithm_on_edge_and_pseudorandom_inputs() {
        let mut s = 0x9e37_79b9_7f4a_7c15u128 | (1 << 100);
        let mut next = || {
            s ^= s << 29;
            s ^= s >> 11;
            s ^= s << 37;
            s
        };
        let edges = [
            0u128,
            1,
            1 << 127,
            u128::MAX,
            0x8000_0000_0000_0000,
            1 << 64,
        ];
        for &a in &edges {
            for &b in &edges {
                assert_eq!(gf_mul(a, b), gf_mul_ref(a, b), "{a:#x} * {b:#x}");
            }
        }
        for _ in 0..2000 {
            let (a, b) = (next(), next());
            assert_eq!(gf_mul(a, b), gf_mul_ref(a, b), "{a:#x} * {b:#x}");
        }
    }
}
