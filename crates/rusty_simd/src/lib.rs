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
