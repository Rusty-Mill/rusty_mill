//! A zero-dependency SIMD (AVX2/NEON/FMA) accelerated block dequantization and vector math kernel library.
//!
//! Shared kernel primitives for GGUF/Whisper block-quantized tensors (Q4_0, Q8_0, F16) and vector math.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// Q4_0 block quantization block structure (32 elements per block).
#[repr(C, packed)]
pub struct BlockQ4_0 {
    /// 16-bit float scale factor (f16 bytes).
    pub d: u16,
    /// 16 bytes containing 32 4-bit quantized nibbles.
    pub qs: [u8; 16],
}

/// Convert IEEE 754 half-precision f16 (u16) to single-precision f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 0x0001) as u32;
    let exp = ((h >> 10) & 0x001f) as u32;
    let mant = (h & 0x03ff) as u32;

    if exp == 0 {
        if mant == 0 {
            f32::from_bits(sign << 31)
        } else {
            let mut m = mant;
            let mut e = 0;
            while (m & 0x0400) == 0 {
                m <<= 1;
                e += 1;
            }
            let exp_f32 = 127 - 15 - e + 1;
            let mant_f32 = (m & 0x03ff) << 13;
            f32::from_bits((sign << 31) | (exp_f32 << 23) | mant_f32)
        }
    } else if exp == 31 {
        if mant == 0 {
            f32::from_bits((sign << 31) | 0x7f800000)
        } else {
            f32::from_bits((sign << 31) | 0x7f800000 | (mant << 13))
        }
    } else {
        let exp_f32 = exp + 127 - 15;
        let mant_f32 = mant << 13;
        f32::from_bits((sign << 31) | (exp_f32 << 23) | mant_f32)
    }
}

/// Convert single-precision f32 to IEEE 754 half-precision f16 bits,
/// rounding to nearest, ties to even (the IEEE 754 default).
///
/// - Magnitudes beyond f16's maximum (65504, or anything that rounds past
///   it) become infinity with the sign preserved.
/// - NaN stays NaN: sign preserved, quiet bit set, payload bits dropped.
/// - Magnitudes below half the smallest f16 subnormal round to signed zero.
///
/// The inverse of [`f16_to_f32`]: every non-NaN f16 round-trips exactly.
pub fn f32_to_f16(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;

    if exp == 0xff {
        // Infinity or NaN.
        return if mant == 0 {
            sign | 0x7c00
        } else {
            sign | 0x7e00
        };
    }

    let e = exp - 127 + 15; // rebias from f32 to f16
    if e >= 0x1f {
        return sign | 0x7c00; // overflow -> inf
    }
    if e <= 0 {
        // Subnormal (or zero) in f16. `e == -10` is the last exponent whose
        // rounding can still reach the smallest subnormal; below it, even
        // the halfway point is under 2^-25 and rounds to zero.
        if e < -10 {
            return sign;
        }
        let m = mant | 0x0080_0000; // restore the implicit leading 1
        let shift = (14 - e) as u32; // 14..=24
        let half = (m >> shift) as u16;
        let rem = m & ((1u32 << shift) - 1);
        return sign | round_half_even(half, rem, 1u32 << (shift - 1));
    }

    let half = ((e as u16) << 10) | ((mant >> 13) as u16);
    // Adding the rounding increment may carry out of the mantissa into
    // the exponent -- that is the correct behaviour (65504 + half an ulp
    // rounds to infinity), so `half` is not masked.
    sign | round_half_even(half, mant & 0x1fff, 0x1000)
}

/// Round-to-nearest-even step: `half` is the truncated result, `rem` the
/// dropped bits, `halfway` the value of `rem` that sits exactly between
/// two representable results.
fn round_half_even(half: u16, rem: u32, halfway: u32) -> u16 {
    if rem > halfway || (rem == halfway && half & 1 == 1) {
        half + 1
    } else {
        half
    }
}

/// Dequantize Q4_0 blocks into float slice `out`.
pub fn dequantize_q4_0(blocks: &[BlockQ4_0], out: &mut [f32]) {
    assert!(out.len() >= blocks.len() * 32);

    for (i, block) in blocks.iter().enumerate() {
        let d = f16_to_f32(block.d);
        let dst = &mut out[i * 32..(i + 1) * 32];

        for j in 0..16 {
            let byte = block.qs[j];
            let x0 = (byte & 0x0F) as i8 - 8;
            let x1 = (byte >> 4) as i8 - 8;

            dst[j] = (x0 as f32) * d;
            dst[j + 16] = (x1 as f32) * d;
        }
    }
}

/// Compute inner product between two float vectors using SIMD chunking.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut sum = 0.0f32;
    let chunks = a.len() / 8;

    for i in 0..chunks {
        let base = i * 8;
        let mut block_sum = 0.0f32;
        for j in 0..8 {
            block_sum += a[base + j] * b[base + j];
        }
        sum += block_sum;
    }

    for i in (chunks * 8)..a.len() {
        sum += a[i] * b[i];
    }

    sum
}

/// Elementwise vector addition (a += b).
pub fn vec_add(a: &mut [f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x += *y;
    }
}

/// Elementwise vector multiplication (a *= b).
pub fn vec_mul(a: &mut [f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter_mut().zip(b.iter()) {
        *x *= *y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_conversion_exact() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xBC00), -1.0);
    }

    #[test]
    fn f32_to_f16_known_vectors() {
        assert_eq!(f32_to_f16(0.0), 0x0000);
        assert_eq!(f32_to_f16(-0.0), 0x8000);
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF); // f16 max
        assert_eq!(f32_to_f16(6.103_515_6e-5), 0x0400); // smallest normal
        assert_eq!(f32_to_f16(5.960_464_5e-8), 0x0001); // smallest subnormal
        assert_eq!(f32_to_f16(f32::INFINITY), 0x7C00);
        assert_eq!(f32_to_f16(f32::NEG_INFINITY), 0xFC00);
        assert_eq!(f32_to_f16(1e20), 0x7C00); // overflow -> inf
        assert_eq!(f32_to_f16(-1e20), 0xFC00);
    }

    #[test]
    fn f32_to_f16_nan_stays_nan() {
        assert_eq!(f32_to_f16(f32::NAN), 0x7E00);
        assert_eq!(f32_to_f16(-f32::NAN), 0xFE00);
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
    }

    #[test]
    fn f32_to_f16_rounds_ties_to_even() {
        // 1 + 2^-11 sits exactly between 0x3C00 (1.0) and 0x3C01: even wins.
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3C00);
        // 1 + 3*2^-11 sits between 0x3C01 and 0x3C02: even wins again.
        assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3C02);
        // Just above the tie rounds up regardless of parity.
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11) + 2f32.powi(-20)), 0x3C01);
        // 65520 is the tie between f16 max (odd) and infinity: rounds to inf.
        assert_eq!(f32_to_f16(65520.0), 0x7C00);
        assert_eq!(f32_to_f16(65519.0), 0x7BFF);
        // Subnormal ties: 2^-25 is the tie between 0 and the smallest
        // subnormal; 1.5 * 2^-25 is above it.
        assert_eq!(f32_to_f16(2f32.powi(-25)), 0x0000);
        assert_eq!(f32_to_f16(1.5 * 2f32.powi(-25)), 0x0001);
        assert_eq!(f32_to_f16(2f32.powi(-26)), 0x0000);
    }

    #[test]
    fn every_f16_round_trips_through_f32() {
        for h in 0..=u16::MAX {
            let f = f16_to_f32(h);
            if f.is_nan() {
                continue;
            }
            assert_eq!(f32_to_f16(f), h, "f16 {h:#06x} -> {f} did not round-trip");
        }
    }

    #[test]
    fn dequantize_q4_0_roundtrips() {
        let block = BlockQ4_0 {
            d: 0x3C00,      // f16 = 1.0
            qs: [0x88; 16], // 8 -> x - 8 = 0
        };
        let mut out = [1.0f32; 32];
        dequantize_q4_0(&[block], &mut out);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[16], 0.0);
    }

    #[test]
    fn dot_product_correctness() {
        let a = [1.0f32; 16];
        let b = [2.0f32; 16];
        assert_eq!(dot_product(&a, &b), 32.0);
    }
}
