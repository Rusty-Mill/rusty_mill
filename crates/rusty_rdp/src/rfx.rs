//! RemoteFX bitmap codec (MS-RDPRFX), std-only.
//!
//! RemoteFX is what [`crate::gfx::WireToSurface1Pdu`] carries when
//! `codec_id` is [`crate::gfx::CODECID_CAVIDEO`]: `bitmapData` is a
//! [`TileSet`] — a per-tile RLGR-entropy-coded, 3-level 5/3 discrete
//! wavelet transform (DWT) compressed YCbCr encoding of a grid of 64×64
//! pixel tiles.
//!
//! ## Pipeline
//!
//! Each of a tile's three components (Y, Cb, Cr) is decoded independently
//! through the same three steps, wired together by [`Tile::decode_rgb`]:
//!
//! 1. [`rlgr1_decode`] — entropy-decodes the component's byte stream back
//!    into 4096 signed coefficients, adaptively switching between
//!    run-length and Golomb-Rice coding (MS-RDPRFX 3.1.8.1.7.1).
//! 2. [`dequantize`] — left-shifts each of the ten DWT sub-bands by its own
//!    per-tile quantization factor ([`CodecQuant`]).
//! 3. [`dwt_decode`] — reverses the 3-level 5/3 lifting-scheme DWT
//!    (MS-RDPRFX 3.1.8.2.4) to reconstruct a 64×64 block of fixed-point
//!    (11.5) component values.
//!
//! [`ycbcr_to_rgb`] then converts each reconstructed Y/Cb/Cr triple to RGB.
//! A [`TileSet`] picks its entropy coding via [`EntropyAlgorithm`] — RLGR1
//! ([`rlgr1_decode`]) or the alternate RLGR3 ([`rlgr3_decode`]), which codes
//! coefficients two at a time (faster to encode, marginally worse
//! compression).
//!
//! ## Control PDUs
//!
//! A `TileSet` doesn't arrive alone: MS-RDPRFX wraps it in a small sequence
//! of control messages, each tagged with its own `TS_RFX_BLOCKT.blockType`
//! ([`peek_block_type`] reads this without consuming the buffer, to route
//! to the matching `decode`). Once per stream: [`SyncPdu`] (first message,
//! confirms the wire-format version), [`CodecVersionsPdu`], [`ChannelsPdu`]
//! (one entry per monitor), and [`ContextPdu`] (encoding properties). Then,
//! repeating per frame: [`FrameBeginPdu`], one or more
//! [`RegionPdu`]/[`TileSet`] pairs (the region lists which rectangles
//! changed; an empty one means the whole frame), and [`FrameEndPdu`].
//!
//! ## Verification
//!
//! The RLGR1/RLGR3 decode pseudocode and every wire layout in this module
//! (`TS_RFX_TILE`/`TS_RFX_TILESET`/`TS_RFX_CODEC_QUANT` and the control
//! PDUs above) are transcribed directly from the MS-RDPRFX Open
//! Specifications pages — RLGR3's `GetMinBits` bit-width semantics
//! (ambiguous from the doc comment alone: "returns ceil(log2(val))") were
//! additionally cross-checked against FreeRDP's `rfx_rlgr.c`, which
//! computes it as the operand's own bit-length via a leading-zero count.
//! The DWT lifting arithmetic and the sub-band-to-buffer-offset/
//! quantization-index mapping were cross-checked against FreeRDP's
//! `rfx_dwt.c`/`rfx_decode.c`/`rfx_quantization.c` reference implementation,
//! since the spec's own DWT equations are rendered as an image rather than
//! text. The YCbCr→RGB matrix uses the standard ITU-R BT.601 coefficients
//! used by public RemoteFX decoders, for the same reason (MS-RDPRFX's own
//! transform diagram is image-only); this is the one formula in this
//! module not transcribed from spec text.
//!
//! **Not yet implemented:** the cache PDUs (`SURFACETOCACHE`,
//! `CACHETOSURFACE`, `CACHEIMPORTOFFER`/`REPLY`) and surface composition
//! (`SOLIDFILL`, `SURFACETOSURFACE`) — those live in [`crate::gfx`], not
//! this module, since they're MS-RDPEGFX PDUs rather than MS-RDPRFX ones —
//! and encoding (the server-side direction): [`dwt_decode`] in particular
//! has no matching forward transform in this module (unlike
//! [`rlgr1_decode`]/[`rlgr3_decode`], each round-trip tested against a
//! private mirror-image encoder); [`dwt_decode`] is instead verified by
//! hand against the lifting equations for inputs where the expected output
//! is computable by hand (see the module's tests).

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// Width and height, in pixels, of one RemoteFX tile.
pub const TILE_SIZE: usize = 64;
/// Number of DWT coefficients per tile component (64×64).
pub const COEFF_COUNT: usize = TILE_SIZE * TILE_SIZE;

// ---------------------------------------------------------------------------
// RLGR1 entropy coding (MS-RDPRFX 3.1.8.1.7.1 / 3.1.8.1.7.3)
// ---------------------------------------------------------------------------

const KPMAX: i32 = 80;
const LSGR: i32 = 3;
const UP_GR: i32 = 4;
const DN_GR: i32 = 6;
const UQ_GR: i32 = 3;
const DQ_GR: i32 = 3;

/// A forward-only, MSB-first bit reader.
struct BitReader<'a> {
    buf: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        BitReader { buf, bit_pos: 0 }
    }

    fn get_bit(&mut self) -> Result<u32> {
        let byte = self.bit_pos / 8;
        let bit = 7 - (self.bit_pos % 8);
        let b = *self.buf.get(byte).ok_or(Error::UnexpectedEof {
            needed: 1,
            available: 0,
        })?;
        self.bit_pos += 1;
        Ok(((b >> bit) & 1) as u32)
    }

    fn get_bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.get_bit()?;
        }
        Ok(v)
    }
}

/// A matching MSB-first bit writer, used to build the round-trip tests.
#[cfg(test)]
#[derive(Default)]
struct BitWriter {
    buf: Vec<u8>,
    bit_pos: usize,
}

#[cfg(test)]
impl BitWriter {
    fn put_bit(&mut self, v: u32) {
        if self.bit_pos % 8 == 0 {
            self.buf.push(0);
        }
        if v != 0 {
            let byte = self.bit_pos / 8;
            let bit = 7 - (self.bit_pos % 8);
            self.buf[byte] |= 1 << bit;
        }
        self.bit_pos += 1;
    }

    fn put_bits(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.put_bit((v >> i) & 1);
        }
    }

    fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

fn update_param(param: &mut i32, delta: i32) -> i32 {
    *param = (*param + delta).clamp(0, KPMAX);
    *param >> LSGR
}

/// Converts an RLGR "2*magnitude - sign" coded value back to a signed
/// integer: even values are non-negative, odd values negative.
fn int_from_2mag_sign(v: u32) -> i32 {
    if v == 0 {
        return 0;
    }
    let mag = ((v + 1) >> 1) as i32;
    if v & 1 == 1 {
        -mag
    } else {
        mag
    }
}

/// Encodes a non-negative integer as "2*magnitude - sign" (the inverse of
/// [`int_from_2mag_sign`]), for the round-trip encoder used in tests.
#[cfg(test)]
fn to_2mag_sign(v: i32) -> u32 {
    if v == 0 {
        0
    } else if v > 0 {
        (v as u32) * 2
    } else {
        (-v as u32) * 2 - 1
    }
}

fn get_gr_code(r: &mut BitReader<'_>, krp: &mut i32, kr: &mut i32) -> Result<u32> {
    let mut vk = 0i32;
    while r.get_bit()? == 1 {
        vk += 1;
    }
    let mag = ((vk as u32) << (*kr as u32)) | r.get_bits(*kr as u32)?;
    if vk == 0 {
        *kr = update_param(krp, -2);
    } else if vk != 1 {
        *kr = update_param(krp, vk);
    }
    Ok(mag)
}

#[cfg(test)]
fn put_gr_code(w: &mut BitWriter, value: u32, krp: &mut i32, kr: &mut i32) {
    let vk = value >> (*kr as u32);
    for _ in 0..vk {
        w.put_bit(1);
    }
    w.put_bit(0);
    w.put_bits(value & ((1u32 << (*kr as u32)) - 1), *kr as u32);
    let vk = vk as i32;
    if vk == 0 {
        *kr = update_param(krp, -2);
    } else if vk != 1 {
        *kr = update_param(krp, vk);
    }
}

/// Which RLGR variant to decode with — see [`rlgr_decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RlgrMode {
    Rlgr1,
    Rlgr3,
}

/// Decode `count` coefficients from an RLGR-encoded byte stream
/// (MS-RDPRFX 3.1.8.1.7.1–3.1.8.1.7.4). Run-length mode is identical
/// between the two variants; only Golomb-Rice mode differs — RLGR1 codes
/// one coefficient's `2*magnitude-sign` value at a time, while RLGR3 codes
/// the *sum* of the next two coefficients' `2*magnitude-sign` values, then
/// binary-codes the first one using exactly as many bits as the sum needs
/// (`GetMinBits`) and derives the second as the remainder.
fn rlgr_decode(data: &[u8], count: usize, mode: RlgrMode) -> Result<Vec<i16>> {
    let mut r = BitReader::new(data);
    let mut out: Vec<i16> = Vec::with_capacity(count);
    let mut k: i32 = 1;
    let mut kp: i32 = k << LSGR;
    let mut kr: i32 = 1;
    let mut krp: i32 = kr << LSGR;

    let write_zeroes = |out: &mut Vec<i16>, n: usize| {
        let space = count.saturating_sub(out.len());
        out.resize(out.len() + n.min(space), 0);
    };
    let write_value = |out: &mut Vec<i16>, v: i32| {
        if out.len() < count {
            out.push(v.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        }
    };

    while out.len() < count {
        if k != 0 {
            // Run-length mode (shared by both variants).
            while r.get_bit()? == 0 {
                write_zeroes(&mut out, 1usize << k);
                k = update_param(&mut kp, UP_GR);
            }
            if out.len() < count {
                let run = r.get_bits(k as u32)? as usize;
                write_zeroes(&mut out, run);
            }
            if out.len() < count {
                let sign = r.get_bit()?;
                let mag = get_gr_code(&mut r, &mut krp, &mut kr)? as i32 + 1;
                write_value(&mut out, if sign != 0 { -mag } else { mag });
                k = update_param(&mut kp, -DN_GR);
            }
        } else {
            // Golomb-Rice mode.
            let mag = get_gr_code(&mut r, &mut krp, &mut kr)?;
            match mode {
                RlgrMode::Rlgr1 => {
                    if mag == 0 {
                        write_value(&mut out, 0);
                        k = update_param(&mut kp, UQ_GR);
                    } else {
                        write_value(&mut out, int_from_2mag_sign(mag));
                        k = update_param(&mut kp, -DQ_GR);
                    }
                }
                RlgrMode::Rlgr3 => {
                    // `mag` is the sum of two coefficients' 2*magnitude-sign
                    // values. `n_idx` (GetMinBits) is `mag`'s own bit width —
                    // the fewest bits that can represent every value in
                    // `0..=mag`, since the first term can be anywhere in
                    // that range.
                    let n_idx = 32 - mag.leading_zeros();
                    let val1 = if n_idx > 0 { r.get_bits(n_idx)? } else { 0 };
                    let val2 = mag - val1;
                    if val1 != 0 && val2 != 0 {
                        k = update_param(&mut kp, -2 * DQ_GR);
                    } else if val1 == 0 && val2 == 0 {
                        k = update_param(&mut kp, 2 * UQ_GR);
                    }
                    write_value(&mut out, int_from_2mag_sign(val1));
                    write_value(&mut out, int_from_2mag_sign(val2));
                }
            }
        }
    }
    Ok(out)
}

/// Decode `count` coefficients from an RLGR1-encoded byte stream
/// (MS-RDPRFX 3.1.8.1.7.1/3.1.8.1.7.3, `RLGR1` mode).
pub fn rlgr1_decode(data: &[u8], count: usize) -> Result<Vec<i16>> {
    rlgr_decode(data, count, RlgrMode::Rlgr1)
}

/// Decode `count` coefficients from an RLGR3-encoded byte stream
/// (MS-RDPRFX 3.1.8.1.7.2/3.1.8.1.7.4, `RLGR3` mode) — the alternate
/// entropy coding a [`TileSet`] can select via `CLW_ENTROPY_RLGR3`
/// (faster to encode, marginally worse compression than RLGR1).
pub fn rlgr3_decode(data: &[u8], count: usize) -> Result<Vec<i16>> {
    rlgr_decode(data, count, RlgrMode::Rlgr3)
}

/// Encode `values` with RLGR1 — the mirror image of [`rlgr1_decode`], kept
/// private and used only to round-trip test the decoder against a
/// symmetric implementation of the same specification pseudocode.
#[cfg(test)]
fn rlgr1_encode(values: &[i16]) -> Vec<u8> {
    let mut w = BitWriter::default();
    let mut k: i32 = 1;
    let mut kp: i32 = k << LSGR;
    let mut kr: i32 = 1;
    let mut krp: i32 = kr << LSGR;

    let mut i = 0usize;
    while i < values.len() {
        if k != 0 {
            let mut run = 0usize;
            while i + run < values.len() && values[i + run] == 0 {
                run += 1;
            }
            let mut remaining = run;
            while remaining >= (1usize << k) {
                w.put_bit(0);
                remaining -= 1usize << k;
                k = update_param(&mut kp, UP_GR);
            }
            w.put_bit(1);
            w.put_bits(remaining as u32, k as u32);
            i += run;
            if i < values.len() {
                let v = values[i] as i32;
                w.put_bit(if v < 0 { 1 } else { 0 });
                put_gr_code(&mut w, v.unsigned_abs() - 1, &mut krp, &mut kr);
                k = update_param(&mut kp, -DN_GR);
                i += 1;
            }
        } else {
            let v = values[i] as i32;
            if v == 0 {
                put_gr_code(&mut w, 0, &mut krp, &mut kr);
                k = update_param(&mut kp, UQ_GR);
            } else {
                put_gr_code(&mut w, to_2mag_sign(v), &mut krp, &mut kr);
                k = update_param(&mut kp, -DQ_GR);
            }
            i += 1;
        }
    }
    w.into_vec()
}

/// Encode `values` with RLGR3 — the mirror image of [`rlgr3_decode`], kept
/// private and used only to round-trip test the decoder. Run-length mode is
/// shared with [`rlgr1_encode`]; Golomb-Rice mode pairs up consecutive
/// values, GR-coding their `2*magnitude-sign` sum and binary-coding the
/// first one with just enough bits to cover that sum's range. A trailing
/// unpaired value (an odd-length input) is padded with an implicit `0`
/// second term, matching how the decoder's second `WriteValue` call is a
/// no-op once `count` values have already been produced.
#[cfg(test)]
fn rlgr3_encode(values: &[i16]) -> Vec<u8> {
    let mut w = BitWriter::default();
    let mut k: i32 = 1;
    let mut kp: i32 = k << LSGR;
    let mut kr: i32 = 1;
    let mut krp: i32 = kr << LSGR;

    let mut i = 0usize;
    while i < values.len() {
        if k != 0 {
            let mut run = 0usize;
            while i + run < values.len() && values[i + run] == 0 {
                run += 1;
            }
            let mut remaining = run;
            while remaining >= (1usize << k) {
                w.put_bit(0);
                remaining -= 1usize << k;
                k = update_param(&mut kp, UP_GR);
            }
            w.put_bit(1);
            w.put_bits(remaining as u32, k as u32);
            i += run;
            if i < values.len() {
                let v = values[i] as i32;
                w.put_bit(if v < 0 { 1 } else { 0 });
                put_gr_code(&mut w, v.unsigned_abs() - 1, &mut krp, &mut kr);
                k = update_param(&mut kp, -DN_GR);
                i += 1;
            }
        } else {
            let val1 = to_2mag_sign(values[i] as i32);
            let val2 = if i + 1 < values.len() {
                to_2mag_sign(values[i + 1] as i32)
            } else {
                0
            };
            let sum = val1 + val2;
            put_gr_code(&mut w, sum, &mut krp, &mut kr);
            let n_idx = 32 - sum.leading_zeros();
            if n_idx > 0 {
                w.put_bits(val1, n_idx);
            }
            if val1 != 0 && val2 != 0 {
                k = update_param(&mut kp, -2 * DQ_GR);
            } else if val1 == 0 && val2 == 0 {
                k = update_param(&mut kp, 2 * UQ_GR);
            }
            i += if i + 1 < values.len() { 2 } else { 1 };
        }
    }
    w.into_vec()
}

// ---------------------------------------------------------------------------
// Inverse DWT (MS-RDPRFX 3.1.8.2.4) — 5/3 lifting scheme, 3 levels
// ---------------------------------------------------------------------------

/// Coefficient buffer offsets and lengths for the three sub-bands of one
/// DWT level. Level 3 additionally stores `LL3`, the DC sub-band, right
/// after its `HH3`.
struct LevelOffsets {
    hl: usize,
    lh: usize,
    hh: usize,
    width: usize,
}

const LEVEL1: LevelOffsets = LevelOffsets {
    hl: 0,
    lh: 1024,
    hh: 2048,
    width: 32,
};
const LEVEL2: LevelOffsets = LevelOffsets {
    hl: 3072,
    lh: 3328,
    hh: 3584,
    width: 16,
};
const LEVEL3: LevelOffsets = LevelOffsets {
    hl: 3840,
    lh: 3904,
    hh: 3968,
    width: 8,
};
/// Offset of the `LL3` (DC) sub-band, the seed for the level-3 inverse step.
const LL3_OFFSET: usize = 4032;

/// One in-place inverse 2D DWT step: reconstructs a `2*width` square block
/// from four `width`-square sub-bands (`HL`, `LH`, `HH`, and an `LL` sourced
/// from `ll_offset`), writing the result back starting at `base`.
fn idwt_block(buffer: &mut [i16; COEFF_COUNT], base: usize, ll_offset: usize, width: usize) {
    let total_width = width * 2;
    let mut idwt = vec![0i16; total_width * total_width];

    // Horizontal pass: combine each row's HL/LL into a low-pass row, and
    // LH/HH into a high-pass row, interleaving even/odd samples.
    for y in 0..width {
        let hl = &buffer[base + y * width..base + y * width + width];
        let ll = &buffer[ll_offset + y * width..ll_offset + y * width + width];
        let lh_off = base + width * width + y * width;
        let hh_off = base + 2 * width * width + y * width;
        let lh = &buffer[lh_off..lh_off + width];
        let hh = &buffer[hh_off..hh_off + width];

        let l_row = &mut idwt[y * total_width..y * total_width + total_width];
        l_row[0] = (ll[0] as i32 - ((hl[0] as i32 * 2 + 1) >> 1)) as i16;
        for n in 1..width {
            let x = n * 2;
            l_row[x] = (ll[n] as i32 - ((hl[n - 1] as i32 + hl[n] as i32 + 1) >> 1)) as i16;
        }
        #[allow(clippy::needless_range_loop)]
        for n in 0..width - 1 {
            let x = n * 2;
            let v = (hl[n] as i32) * 2 + ((l_row[x] as i32 + l_row[x + 2] as i32) >> 1);
            l_row[x + 1] = v as i16;
        }
        {
            let n = width - 1;
            let x = n * 2;
            l_row[x + 1] = ((hl[n] as i32) * 2 + l_row[x] as i32) as i16;
        }

        let h_row_off = width * width * 2 + y * total_width;
        let h_row = &mut idwt[h_row_off..h_row_off + total_width];
        h_row[0] = (lh[0] as i32 - ((hh[0] as i32 * 2 + 1) >> 1)) as i16;
        for n in 1..width {
            let x = n * 2;
            h_row[x] = (lh[n] as i32 - ((hh[n - 1] as i32 + hh[n] as i32 + 1) >> 1)) as i16;
        }
        #[allow(clippy::needless_range_loop)]
        for n in 0..width - 1 {
            let x = n * 2;
            let v = (hh[n] as i32) * 2 + ((h_row[x] as i32 + h_row[x + 2] as i32) >> 1);
            h_row[x + 1] = v as i16;
        }
        {
            let n = width - 1;
            let x = n * 2;
            h_row[x + 1] = ((hh[n] as i32) * 2 + h_row[x] as i32) as i16;
        }
    }

    // Vertical pass: combine the intermediate L/H rows column-wise into the
    // final reconstructed block, written back into `buffer` at `base`.
    for x in 0..total_width {
        let l_col = |n: usize| idwt[n * total_width + x] as i32;
        let h_col = |n: usize| idwt[(width + n) * total_width + x] as i32;

        let mut dst = vec![0i16; total_width];
        dst[0] = (l_col(0) - ((h_col(0) * 2 + 1) >> 1)) as i16;
        for n in 1..width {
            dst[2 * n] = (l_col(n) - ((h_col(n - 1) + h_col(n) + 1) >> 1)) as i16;
        }
        for n in 0..width - 1 {
            let v = h_col(n) * 2 + ((dst[2 * n] as i32 + dst[2 * n + 2] as i32) >> 1);
            dst[2 * n + 1] = v as i16;
        }
        {
            let n = width - 1;
            dst[2 * n + 1] = (h_col(n) * 2 + ((dst[2 * n] as i32 * 2) >> 1)) as i16;
        }

        for (row, value) in dst.into_iter().enumerate() {
            buffer[base + row * total_width + x] = value;
        }
    }
}

/// Reverse the 3-level 5/3 DWT in place: `coeffs` holds the ten sub-bands
/// in `HL1, LH1, HH1, HL2, LH2, HH2, HL3, LH3, HH3, LL3` order on entry, and
/// a 64×64 row-major block of reconstructed component values on return.
pub fn dwt_decode(coeffs: &mut [i16; COEFF_COUNT]) {
    idwt_block(coeffs, LEVEL3.hl, LL3_OFFSET, LEVEL3.width);
    idwt_block(coeffs, LEVEL2.hl, LEVEL3.hl, LEVEL2.width);
    idwt_block(coeffs, LEVEL1.hl, LEVEL2.hl, LEVEL1.width);
}

// ---------------------------------------------------------------------------
// Quantization (MS-RDPRFX 2.2.2.1.5 / 3.1.8.2.3)
// ---------------------------------------------------------------------------

/// `TS_RFX_CODEC_QUANT` — the ten 4-bit scalar quantization factors for one
/// tile's DWT sub-bands, each in the range 6 to 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecQuant {
    /// `[LL3, LH3, HL3, HH3, LH2, HL2, HH2, LH1, HL1, HH1]`, the wire order.
    pub factors: [u8; 10],
}

const IDX_LL3: usize = 0;
const IDX_LH3: usize = 1;
const IDX_HL3: usize = 2;
const IDX_HH3: usize = 3;
const IDX_LH2: usize = 4;
const IDX_HL2: usize = 5;
const IDX_HH2: usize = 6;
const IDX_LH1: usize = 7;
const IDX_HL1: usize = 8;
const IDX_HH1: usize = 9;

impl CodecQuant {
    /// Encode to the 5-byte packed wire form.
    pub fn encode(&self) -> [u8; 5] {
        let f = &self.factors;
        [
            (f[0] << 4) | f[1],
            (f[2] << 4) | f[3],
            (f[4] << 4) | f[5],
            (f[6] << 4) | f[7],
            (f[8] << 4) | f[9],
        ]
    }

    /// Decode from the 5-byte packed wire form.
    pub fn decode(bytes: [u8; 5]) -> CodecQuant {
        let mut f = [0u8; 10];
        for (i, b) in bytes.iter().enumerate() {
            f[i * 2] = b >> 4;
            f[i * 2 + 1] = b & 0x0F;
        }
        CodecQuant { factors: f }
    }
}

/// Sub-band regions of the coefficient buffer, paired with their
/// [`CodecQuant`] index, in the order [`dwt_decode`] expects them.
const SUBBAND_LAYOUT: [(usize, usize, usize); 10] = [
    (LEVEL1.hl, 1024, IDX_HL1),
    (LEVEL1.lh, 1024, IDX_LH1),
    (LEVEL1.hh, 1024, IDX_HH1),
    (LEVEL2.hl, 256, IDX_HL2),
    (LEVEL2.lh, 256, IDX_LH2),
    (LEVEL2.hh, 256, IDX_HH2),
    (LEVEL3.hl, 64, IDX_HL3),
    (LEVEL3.lh, 64, IDX_LH3),
    (LEVEL3.hh, 64, IDX_HH3),
    (LL3_OFFSET, 64, IDX_LL3),
];

/// Left-shift each sub-band of `coeffs` by `quant.factors[idx] - 1`
/// (MS-RDPRFX 3.1.8.2.3), reversing the encoder's scalar quantization.
pub fn dequantize(coeffs: &mut [i16; COEFF_COUNT], quant: &CodecQuant) {
    for &(offset, len, idx) in &SUBBAND_LAYOUT {
        let shift = quant.factors[idx].saturating_sub(1) as u32;
        for c in &mut coeffs[offset..offset + len] {
            let v = (*c as i32) << shift;
            *c = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }
}

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

/// Convert one fixed-point (11.5: 5 fractional bits) Y/Cb/Cr triple, as
/// reconstructed by [`dwt_decode`], to 8-bit RGB using the standard
/// ITU-R BT.601 matrix.
pub fn ycbcr_to_rgb(y: i16, cb: i16, cr: i16) -> (u8, u8, u8) {
    let y = (y as f32) / 32.0 + 128.0;
    let cb = (cb as f32) / 32.0;
    let cr = (cr as f32) / 32.0;

    let r = y + 1.402 * cr;
    let g = y - 0.344136 * cb - 0.714136 * cr;
    let b = y + 1.772 * cb;

    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// Wire structures: TS_RFX_TILE / TS_RFX_TILESET
// ---------------------------------------------------------------------------

const CBT_TILE: u16 = 0xCAC3;
const WBT_EXTENSION: u16 = 0xCCC7;
const CBT_TILESET: u16 = 0xCAC2;

/// `CLW_ENTROPY_RLGR1`.
const CLW_ENTROPY_RLGR1: u8 = 0x01;
/// `CLW_ENTROPY_RLGR3`.
const CLW_ENTROPY_RLGR3: u8 = 0x04;

/// `et` — the entropy algorithm a [`TileSet`] selects
/// (`TS_RFX_TILESET.properties.et`, MS-RDPRFX 2.2.2.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropyAlgorithm {
    /// `CLW_ENTROPY_RLGR1` — codes one coefficient per Golomb-Rice step
    /// ([`rlgr1_decode`]).
    Rlgr1,
    /// `CLW_ENTROPY_RLGR3` — codes a pair of coefficients per Golomb-Rice
    /// step ([`rlgr3_decode`]); faster to encode, marginally worse
    /// compression than RLGR1.
    Rlgr3,
}

impl EntropyAlgorithm {
    fn from_et(et: u8) -> Result<EntropyAlgorithm> {
        match et {
            CLW_ENTROPY_RLGR1 => Ok(EntropyAlgorithm::Rlgr1),
            CLW_ENTROPY_RLGR3 => Ok(EntropyAlgorithm::Rlgr3),
            other => Err(Error::InvalidValue {
                field: "TS_RFX_TILESET properties.et",
                value: other.to_string(),
            }),
        }
    }

    fn to_et(self) -> u8 {
        match self {
            EntropyAlgorithm::Rlgr1 => CLW_ENTROPY_RLGR1,
            EntropyAlgorithm::Rlgr3 => CLW_ENTROPY_RLGR3,
        }
    }

    fn decode(self, data: &[u8], count: usize) -> Result<Vec<i16>> {
        match self {
            EntropyAlgorithm::Rlgr1 => rlgr1_decode(data, count),
            EntropyAlgorithm::Rlgr3 => rlgr3_decode(data, count),
        }
    }
}

/// Decode one component (Y, Cb, or Cr) of a tile: entropy-decode its byte
/// stream with `entropy`, dequantize with `quant`, and reverse the DWT.
pub fn decode_component(
    data: &[u8],
    quant: &CodecQuant,
    entropy: EntropyAlgorithm,
) -> Result<[i16; COEFF_COUNT]> {
    let decoded = entropy.decode(data, COEFF_COUNT)?;
    let mut coeffs = [0i16; COEFF_COUNT];
    coeffs.copy_from_slice(&decoded);
    dequantize(&mut coeffs, quant);
    dwt_decode(&mut coeffs);
    Ok(coeffs)
}

/// `TS_RFX_TILE` — the position and RLGR-encoded Y/Cb/Cr data for one
/// 64×64 tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// Index into the containing [`TileSet`]'s `quant_vals` for the
    /// Y-component sub-bands.
    pub quant_idx_y: u8,
    /// Index into `quant_vals` for the Cb-component sub-bands.
    pub quant_idx_cb: u8,
    /// Index into `quant_vals` for the Cr-component sub-bands.
    pub quant_idx_cr: u8,
    /// X-index of this tile in the screen tile grid.
    pub x_idx: u16,
    /// Y-index of this tile in the screen tile grid.
    pub y_idx: u16,
    /// RLGR1-encoded Y-component data.
    pub y_data: Vec<u8>,
    /// RLGR1-encoded Cb-component data.
    pub cb_data: Vec<u8>,
    /// RLGR1-encoded Cr-component data.
    pub cr_data: Vec<u8>,
}

impl Tile {
    /// Encode to bytes (`TS_RFX_TILE`, including its `BlockT` header).
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u8(self.quant_idx_y);
        body.write_u8(self.quant_idx_cb);
        body.write_u8(self.quant_idx_cr);
        body.write_u16_le(self.x_idx);
        body.write_u16_le(self.y_idx);
        body.write_u16_le(self.y_data.len() as u16);
        body.write_u16_le(self.cb_data.len() as u16);
        body.write_u16_le(self.cr_data.len() as u16);
        body.write_bytes(&self.y_data);
        body.write_bytes(&self.cb_data);
        body.write_bytes(&self.cr_data);

        let block_len = 6 + body.len();
        let mut w = Writer::with_capacity(block_len);
        w.write_u16_le(CBT_TILE);
        w.write_u32_le(block_len as u32);
        w.write_bytes(body.as_slice());
        w.into_vec()
    }

    /// Decode from bytes (`TS_RFX_TILE`, including its `BlockT` header).
    pub fn decode(buf: &[u8]) -> Result<Tile> {
        let mut r = Reader::new(buf);
        let block_type = r.read_u16_le()?;
        if block_type != CBT_TILE {
            return Err(Error::InvalidValue {
                field: "TS_RFX_TILE blockType",
                value: format!("0x{block_type:04X} (expected 0x{CBT_TILE:04X})"),
            });
        }
        let block_len = r.read_u32_le()? as usize;
        if block_len != buf.len() {
            return Err(Error::InvalidLength {
                field: "TS_RFX_TILE blockLen",
                length: block_len,
            });
        }
        let quant_idx_y = r.read_u8()?;
        let quant_idx_cb = r.read_u8()?;
        let quant_idx_cr = r.read_u8()?;
        let x_idx = r.read_u16_le()?;
        let y_idx = r.read_u16_le()?;
        let y_len = r.read_u16_le()? as usize;
        let cb_len = r.read_u16_le()? as usize;
        let cr_len = r.read_u16_le()? as usize;
        let y_data = r.read_bytes(y_len)?.to_vec();
        let cb_data = r.read_bytes(cb_len)?.to_vec();
        let cr_data = r.read_bytes(cr_len)?.to_vec();
        Ok(Tile {
            quant_idx_y,
            quant_idx_cb,
            quant_idx_cr,
            x_idx,
            y_idx,
            y_data,
            cb_data,
            cr_data,
        })
    }

    /// Fully decode this tile to a 64×64 row-major RGB pixel buffer
    /// (`TILE_SIZE * TILE_SIZE * 3` bytes), looking up its three
    /// [`CodecQuant`]s by index in `quant_vals` and entropy-decoding with
    /// `entropy` (both as supplied by the containing [`TileSet`]).
    pub fn decode_rgb(
        &self,
        quant_vals: &[CodecQuant],
        entropy: EntropyAlgorithm,
    ) -> Result<Vec<u8>> {
        let lookup = |idx: u8| {
            quant_vals
                .get(idx as usize)
                .copied()
                .ok_or(Error::InvalidValue {
                    field: "TS_RFX_TILE quantIdx",
                    value: idx.to_string(),
                })
        };
        let y = decode_component(&self.y_data, &lookup(self.quant_idx_y)?, entropy)?;
        let cb = decode_component(&self.cb_data, &lookup(self.quant_idx_cb)?, entropy)?;
        let cr = decode_component(&self.cr_data, &lookup(self.quant_idx_cr)?, entropy)?;

        let mut rgb = vec![0u8; COEFF_COUNT * 3];
        for i in 0..COEFF_COUNT {
            let (r, g, b) = ycbcr_to_rgb(y[i], cb[i], cr[i]);
            rgb[i * 3] = r;
            rgb[i * 3 + 1] = g;
            rgb[i * 3 + 2] = b;
        }
        Ok(rgb)
    }
}

/// `TS_RFX_TILESET` — the quantization tables and encoded tile data for an
/// arbitrary number of changed tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSet {
    /// The entropy algorithm `tiles`' Y/Cb/Cr streams are encoded with
    /// (`properties.et`).
    pub entropy_algorithm: EntropyAlgorithm,
    /// The quantization factor sets tiles reference by index.
    pub quant_vals: Vec<CodecQuant>,
    /// The encoded tiles.
    pub tiles: Vec<Tile>,
}

impl TileSet {
    /// Encode to bytes (`TS_RFX_TILESET`, including its `CodecChannelT`
    /// header).
    pub fn encode(&self) -> Vec<u8> {
        let mut tiles_data = Writer::new();
        for tile in &self.tiles {
            tiles_data.write_bytes(&tile.encode());
        }

        let mut body = Writer::new();
        body.write_u8(0); // codecId, ignored by the decoder role here.
        body.write_u8(0); // channelId, likewise.
        body.write_u16_le(CBT_TILESET);
        body.write_u16_le(0); // idx, MUST be zero.
                              // properties: lt=1, flags=0, cct=COL_CONV_ICT, xft=CLW_XFORM_DWT_53_A,
                              // et=self.entropy_algorithm, qt=SCALAR_QUANTIZATION.
        let properties: u16 = (1 << 15)
            | (0b01 << 10)
            | (0b0001 << 6)
            | ((self.entropy_algorithm.to_et() as u16) << 2)
            | 0b01;
        body.write_u16_le(properties);
        body.write_u8(self.quant_vals.len() as u8);
        body.write_u8(TILE_SIZE as u8);
        body.write_u16_le(self.tiles.len() as u16);
        body.write_u32_le(tiles_data.len() as u32);
        for q in &self.quant_vals {
            body.write_bytes(&q.encode());
        }
        body.write_bytes(tiles_data.as_slice());

        let block_len = 6 + body.len();
        let mut w = Writer::with_capacity(block_len);
        w.write_u16_le(WBT_EXTENSION);
        w.write_u32_le(block_len as u32);
        w.write_bytes(body.as_slice());
        w.into_vec()
    }

    /// Decode from bytes (`TS_RFX_TILESET`, including its `CodecChannelT`
    /// header).
    pub fn decode(buf: &[u8]) -> Result<TileSet> {
        let mut r = Reader::new(buf);
        let block_type = r.read_u16_le()?;
        if block_type != WBT_EXTENSION {
            return Err(Error::InvalidValue {
                field: "TS_RFX_TILESET blockType",
                value: format!("0x{block_type:04X} (expected 0x{WBT_EXTENSION:04X})"),
            });
        }
        let block_len = r.read_u32_le()? as usize;
        if block_len != buf.len() {
            return Err(Error::InvalidLength {
                field: "TS_RFX_TILESET blockLen",
                length: block_len,
            });
        }
        let _codec_id = r.read_u8()?;
        let _channel_id = r.read_u8()?;
        let subtype = r.read_u16_le()?;
        if subtype != CBT_TILESET {
            return Err(Error::InvalidValue {
                field: "TS_RFX_TILESET subtype",
                value: format!("0x{subtype:04X} (expected 0x{CBT_TILESET:04X})"),
            });
        }
        let _idx = r.read_u16_le()?;
        let properties = r.read_u16_le()?;
        let entropy_algorithm = EntropyAlgorithm::from_et(((properties >> 2) & 0x0F) as u8)?;
        let num_quant = r.read_u8()?;
        let _tile_size = r.read_u8()?;
        let num_tiles = r.read_u16_le()?;
        let tiles_data_size = r.read_u32_le()? as usize;

        let mut quant_vals = Vec::with_capacity(num_quant as usize);
        for _ in 0..num_quant {
            let bytes: [u8; 5] = r.read_array()?;
            quant_vals.push(CodecQuant::decode(bytes));
        }

        let tiles_buf = r.read_bytes(tiles_data_size)?;
        let mut tr = Reader::new(tiles_buf);
        let mut tiles = Vec::with_capacity(num_tiles as usize);
        for _ in 0..num_tiles {
            // Peek the blockLen to know how many bytes this tile occupies.
            let mut peek = Reader::new(tr.peek_remaining());
            let _block_type = peek.read_u16_le()?;
            let block_len = peek.read_u32_le()? as usize;
            let tile_bytes = tr.read_bytes(block_len)?;
            tiles.push(Tile::decode(tile_bytes)?);
        }

        Ok(TileSet {
            entropy_algorithm,
            quant_vals,
            tiles,
        })
    }
}

// ---------------------------------------------------------------------------
// Control PDUs: TS_RFX_SYNC / CODEC_VERSIONS / CHANNELS / CONTEXT / REGION /
// FRAME_BEGIN / FRAME_END — the messages that wrap a TileSet on the wire.
// ---------------------------------------------------------------------------

// TS_RFX_BLOCKT blockType values (MS-RDPRFX 2.2.2.1.1) — the shared tag at
// the start of every message in a RemoteFX stream.
/// `WBT_SYNC` — a [`SyncPdu`].
pub const WBT_SYNC: u16 = 0xCCC0;
/// `WBT_CODEC_VERSIONS` — a [`CodecVersionsPdu`].
pub const WBT_CODEC_VERSIONS: u16 = 0xCCC1;
/// `WBT_CHANNELS` — a [`ChannelsPdu`].
pub const WBT_CHANNELS: u16 = 0xCCC2;
/// `WBT_CONTEXT` — a [`ContextPdu`].
pub const WBT_CONTEXT: u16 = 0xCCC3;
/// `WBT_FRAME_BEGIN` — a [`FrameBeginPdu`].
pub const WBT_FRAME_BEGIN: u16 = 0xCCC4;
/// `WBT_FRAME_END` — a [`FrameEndPdu`].
pub const WBT_FRAME_END: u16 = 0xCCC5;
/// `WBT_REGION` — a [`RegionPdu`].
pub const WBT_REGION: u16 = 0xCCC6;

const WF_MAGIC: u32 = 0xCACC_ACCA;
const WF_VERSION_1_0: u16 = 0x0100;
const CT_TILE_64X64: u16 = 0x0040;
const CBT_REGION: u16 = 0xCAC1;

/// Peek a message's `TS_RFX_BLOCKT.blockType` without consuming the buffer,
/// to route to the matching `decode` before calling it — every message in a
/// MS-RDPRFX stream starts with this 2-byte tag (one of the `WBT_*`/`CBT_*`
/// constants).
pub fn peek_block_type(buf: &[u8]) -> Result<u16> {
    let mut r = Reader::new(buf);
    r.read_u16_le()
}

/// Validate and consume a `TS_RFX_BLOCKT` header: `blockType` (checked
/// against `expected`) and `blockLen` (checked against `total_len`, since
/// every message here is decoded from exactly one block's bytes).
fn decode_block_header(r: &mut Reader<'_>, expected: u16, total_len: usize) -> Result<()> {
    let block_type = r.read_u16_le()?;
    if block_type != expected {
        return Err(Error::InvalidValue {
            field: "TS_RFX_BLOCKT blockType",
            value: format!("0x{block_type:04X} (expected 0x{expected:04X})"),
        });
    }
    let block_len = r.read_u32_le()? as usize;
    if block_len != total_len {
        return Err(Error::InvalidLength {
            field: "TS_RFX_BLOCKT blockLen",
            length: block_len,
        });
    }
    Ok(())
}

/// Validate and consume a `TS_RFX_CODEC_CHANNELT` header: the `blockType`/
/// `blockLen` of [`decode_block_header`], plus `codecId`/`channelId`
/// (read but not checked — MS-RDPRFX has the decoder ignore both).
fn decode_codec_channel_header(r: &mut Reader<'_>, expected: u16, total_len: usize) -> Result<()> {
    decode_block_header(r, expected, total_len)?;
    let _codec_id = r.read_u8()?;
    let _channel_id = r.read_u8()?;
    Ok(())
}

/// Encode `body` behind a 6-byte `TS_RFX_BLOCKT` header.
fn encode_block(block_type: u16, body: &[u8]) -> Vec<u8> {
    let block_len = 6 + body.len();
    let mut w = Writer::with_capacity(block_len);
    w.write_u16_le(block_type);
    w.write_u32_le(block_len as u32);
    w.write_bytes(body);
    w.into_vec()
}

/// Encode `body` behind an 8-byte `TS_RFX_CODEC_CHANNELT` header
/// (`codecId` fixed at `0x01` per spec; `channelId` as given).
fn encode_codec_channel_block(block_type: u16, channel_id: u8, body: &[u8]) -> Vec<u8> {
    let block_len = 8 + body.len();
    let mut w = Writer::with_capacity(block_len);
    w.write_u16_le(block_type);
    w.write_u32_le(block_len as u32);
    w.write_u8(1); // codecId, MUST be 0x01.
    w.write_u8(channel_id);
    w.write_bytes(body);
    w.into_vec()
}

/// `TS_RFX_SYNC` — MUST be the first message in an encoded stream; the
/// decoder examines it to confirm the wire-format version it's about to
/// parse is one it understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncPdu;

impl SyncPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(6);
        body.write_u32_le(WF_MAGIC);
        body.write_u16_le(WF_VERSION_1_0);
        encode_block(WBT_SYNC, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<SyncPdu> {
        let mut r = Reader::new(buf);
        decode_block_header(&mut r, WBT_SYNC, buf.len())?;
        let magic = r.read_u32_le()?;
        if magic != WF_MAGIC {
            return Err(Error::InvalidValue {
                field: "TS_RFX_SYNC magic",
                value: format!("0x{magic:08X} (expected 0x{WF_MAGIC:08X})"),
            });
        }
        let version = r.read_u16_le()?;
        if version != WF_VERSION_1_0 {
            return Err(Error::InvalidValue {
                field: "TS_RFX_SYNC version",
                value: format!("0x{version:04X} (expected 0x{WF_VERSION_1_0:04X})"),
            });
        }
        Ok(SyncPdu)
    }
}

/// `TS_RFX_CODEC_VERSIONS` — indicates the RemoteFX codec version in use.
/// Every field is a fixed spec value, so this carries no data of its own;
/// `decode` validates the wire bytes rather than exposing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CodecVersionsPdu;

impl CodecVersionsPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(4);
        body.write_u8(1); // numCodecs, MUST be 0x01.
        body.write_u8(1); // codecId, MUST be 0x01.
        body.write_u16_le(WF_VERSION_1_0);
        encode_block(WBT_CODEC_VERSIONS, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CodecVersionsPdu> {
        let mut r = Reader::new(buf);
        decode_block_header(&mut r, WBT_CODEC_VERSIONS, buf.len())?;
        let num_codecs = r.read_u8()?;
        if num_codecs != 1 {
            return Err(Error::InvalidValue {
                field: "TS_RFX_CODEC_VERSIONS numCodecs",
                value: num_codecs.to_string(),
            });
        }
        let _codec_id = r.read_u8()?;
        let version = r.read_u16_le()?;
        if version != WF_VERSION_1_0 {
            return Err(Error::InvalidValue {
                field: "TS_RFX_CODEC_VERSIONS codecs.version",
                value: format!("0x{version:04X} (expected 0x{WF_VERSION_1_0:04X})"),
            });
        }
        Ok(CodecVersionsPdu)
    }
}

/// `TS_RFX_CHANNELT` — one channel's frame dimensions, within a
/// [`ChannelsPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RfxChannel {
    /// Channel identifier. MS-RDPRFX requires `0x00` in practice (one
    /// channel per stream).
    pub channel_id: u8,
    /// Frame width, conventionally 1 to 4096.
    pub width: i16,
    /// Frame height, conventionally 1 to 2048.
    pub height: i16,
}

impl RfxChannel {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u8(self.channel_id);
        w.write_u16_le(self.width as u16);
        w.write_u16_le(self.height as u16);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<RfxChannel> {
        Ok(RfxChannel {
            channel_id: r.read_u8()?,
            width: r.read_u16_le()? as i16,
            height: r.read_u16_le()? as i16,
        })
    }
}

/// `TS_RFX_CHANNELS` — the list of channels; one entry per active monitor
/// at minimum (MS-RDPRFX allows more).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelsPdu {
    /// The channel list.
    pub channels: Vec<RfxChannel>,
}

impl ChannelsPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(1 + self.channels.len() * 5);
        body.write_u8(self.channels.len() as u8);
        for ch in &self.channels {
            ch.encode_into(&mut body);
        }
        encode_block(WBT_CHANNELS, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ChannelsPdu> {
        let mut r = Reader::new(buf);
        decode_block_header(&mut r, WBT_CHANNELS, buf.len())?;
        let num_channels = r.read_u8()?;
        let mut channels = Vec::with_capacity(num_channels as usize);
        for _ in 0..num_channels {
            channels.push(RfxChannel::decode_from(&mut r)?);
        }
        Ok(ChannelsPdu { channels })
    }
}

/// `TS_RFX_CONTEXT` — the encoding properties in use for the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextPdu {
    /// `true` for `CODEC_MODE` (image mode); `false` for video mode.
    pub image_mode: bool,
    /// The entropy algorithm nominally in use. MS-RDPRFX has the decoder
    /// ignore this in favor of each [`TileSet`]'s own
    /// [`TileSet::entropy_algorithm`], but the encoder still sets it.
    pub entropy_algorithm: EntropyAlgorithm,
}

impl ContextPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(5);
        body.write_u8(0); // ctxId, MUST be 0x00.
        body.write_u16_le(CT_TILE_64X64);
        let flags: u16 = if self.image_mode { 0x02 } else { 0 };
        // properties: flags(3) | cct=COL_CONV_ICT(2) | xft=CLW_XFORM_DWT_53_A(4)
        // | et(4) | qt=SCALAR_QUANTIZATION(2) | r(1).
        let properties: u16 = (flags << 13)
            | (0b01 << 11)
            | (0b0001 << 7)
            | ((self.entropy_algorithm.to_et() as u16) << 3)
            | (0b01 << 1);
        body.write_u16_le(properties);
        encode_codec_channel_block(WBT_CONTEXT, 0xFF, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ContextPdu> {
        let mut r = Reader::new(buf);
        decode_codec_channel_header(&mut r, WBT_CONTEXT, buf.len())?;
        let _ctx_id = r.read_u8()?;
        let _tile_size = r.read_u16_le()?;
        let properties = r.read_u16_le()?;
        // flags occupies bits 13-15; CODEC_MODE (0x02) is the field's own
        // bit 1, i.e. absolute bit 14.
        let image_mode = (properties >> 14) & 0b1 != 0;
        let entropy_algorithm = EntropyAlgorithm::from_et(((properties >> 3) & 0x0F) as u8)?;
        Ok(ContextPdu {
            image_mode,
            entropy_algorithm,
        })
    }
}

/// `TS_RFX_RECT` — a change rectangle within a [`RegionPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RfxRect {
    /// X-coordinate.
    pub x: u16,
    /// Y-coordinate.
    pub y: u16,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
}

impl RfxRect {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u16_le(self.x);
        w.write_u16_le(self.y);
        w.write_u16_le(self.width);
        w.write_u16_le(self.height);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<RfxRect> {
        Ok(RfxRect {
            x: r.read_u16_le()?,
            y: r.read_u16_le()?,
            width: r.read_u16_le()?,
            height: r.read_u16_le()?,
        })
    }
}

/// `TS_RFX_REGION` — the list of change rectangles for the [`TileSet`]
/// message(s) immediately following. An empty `rects` means the whole
/// channel frame changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPdu {
    /// The change rectangles; empty means the whole frame.
    pub rects: Vec<RfxRect>,
}

impl RegionPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(3 + self.rects.len() * 8 + 4);
        body.write_u8(0x01); // regionFlags: lrf=1, reserved=0.
        body.write_u16_le(self.rects.len() as u16);
        for rect in &self.rects {
            rect.encode_into(&mut body);
        }
        body.write_u16_le(CBT_REGION);
        body.write_u16_le(1); // numTilesets, MUST be 0x0001.
        encode_codec_channel_block(WBT_REGION, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<RegionPdu> {
        let mut r = Reader::new(buf);
        decode_codec_channel_header(&mut r, WBT_REGION, buf.len())?;
        let _region_flags = r.read_u8()?;
        let num_rects = r.read_u16_le()?;
        let mut rects = Vec::with_capacity(num_rects as usize);
        for _ in 0..num_rects {
            rects.push(RfxRect::decode_from(&mut r)?);
        }
        let region_type = r.read_u16_le()?;
        if region_type != CBT_REGION {
            return Err(Error::InvalidValue {
                field: "TS_RFX_REGION regionType",
                value: format!("0x{region_type:04X} (expected 0x{CBT_REGION:04X})"),
            });
        }
        let _num_tilesets = r.read_u16_le()?;
        Ok(RegionPdu { rects })
    }
}

/// `TS_RFX_FRAME_BEGIN` — marks the start of a new frame; `num_regions`
/// [`RegionPdu`]/[`TileSet`] pairs follow before the matching
/// [`FrameEndPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBeginPdu {
    /// Index of this frame in the video sequence (video mode only; ignored
    /// in image mode).
    pub frame_idx: u32,
    /// Number of [`RegionPdu`] messages following.
    pub num_regions: i16,
}

impl FrameBeginPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(6);
        body.write_u32_le(self.frame_idx);
        body.write_u16_le(self.num_regions as u16);
        encode_codec_channel_block(WBT_FRAME_BEGIN, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FrameBeginPdu> {
        let mut r = Reader::new(buf);
        decode_codec_channel_header(&mut r, WBT_FRAME_BEGIN, buf.len())?;
        let frame_idx = r.read_u32_le()?;
        let num_regions = r.read_u16_le()? as i16;
        Ok(FrameBeginPdu {
            frame_idx,
            num_regions,
        })
    }
}

/// `TS_RFX_FRAME_END` — marks the end of a frame; carries no data beyond
/// its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameEndPdu;

impl FrameEndPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        encode_codec_channel_block(WBT_FRAME_END, 0, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FrameEndPdu> {
        let mut r = Reader::new(buf);
        decode_codec_channel_header(&mut r, WBT_FRAME_END, buf.len())?;
        Ok(FrameEndPdu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlgr1_hand_traced_single_value() {
        // Hand-traced against the spec pseudocode: bits 1,0,0,0,1 decode a
        // single value of +2 from the initial k=1/kr=1 state (see the
        // module's development notes — RL-mode escape exits immediately,
        // a zero-length run, a positive sign, and a GR-code magnitude of 1
        // giving mag = 1 + 1 = 2).
        let data = [0b1000_1000u8];
        assert_eq!(rlgr1_decode(&data, 1).unwrap(), vec![2]);
    }

    #[test]
    fn rlgr1_roundtrip_various_sequences() {
        let cases: Vec<Vec<i16>> = vec![
            vec![0; 16],
            vec![1, -1, 2, -2, 3, -3, 0, 0, 0, 5],
            (0..64).map(|i| ((i * 37) % 17) as i16 - 8).collect(),
            vec![0; 100]
                .into_iter()
                .chain(vec![42, -42])
                .chain(vec![0; 50])
                .collect(),
            vec![100, -100, 200, -200, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        ];
        for values in cases {
            let encoded = rlgr1_encode(&values);
            let decoded = rlgr1_decode(&encoded, values.len()).unwrap();
            assert_eq!(decoded, values);
        }
    }

    #[test]
    fn rlgr1_all_zero_run_longer_than_escape_step() {
        let values = vec![0i16; 500];
        let encoded = rlgr1_encode(&values);
        assert_eq!(rlgr1_decode(&encoded, 500).unwrap(), values);
    }

    #[test]
    fn rlgr3_roundtrip_various_sequences() {
        let cases: Vec<Vec<i16>> = vec![
            vec![0; 16],
            vec![1, -1, 2, -2, 3, -3, 0, 0, 0, 5],
            (0..64).map(|i| ((i * 37) % 17) as i16 - 8).collect(),
            vec![0; 100]
                .into_iter()
                .chain(vec![42, -42])
                .chain(vec![0; 50])
                .collect(),
            vec![100, -100, 200, -200, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            // A run entirely in GR mode with paired zero/nonzero values, to
            // exercise every (val1, val2) zero/nonzero combination.
            vec![0, 0, 1, 0, 0, 1, 1, 1, -1, -1],
        ];
        for values in cases {
            let encoded = rlgr3_encode(&values);
            let decoded = rlgr3_decode(&encoded, values.len()).unwrap();
            assert_eq!(decoded, values, "roundtrip mismatch for {values:?}");
        }
    }

    #[test]
    fn rlgr3_roundtrips_odd_length_with_implicit_trailing_zero() {
        let values = vec![1i16, -1, 2, -2, 3];
        let encoded = rlgr3_encode(&values);
        assert_eq!(rlgr3_decode(&encoded, values.len()).unwrap(), values);
    }

    #[test]
    fn rlgr3_all_zero_run_longer_than_escape_step() {
        let values = vec![0i16; 500];
        let encoded = rlgr3_encode(&values);
        assert_eq!(rlgr3_decode(&encoded, 500).unwrap(), values);
    }

    #[test]
    fn rlgr1_and_rlgr3_diverge_on_the_same_input() {
        // Same coefficient stream, different entropy coding: the encoded
        // bytes should (almost certainly) differ once GR mode is reached.
        let values: Vec<i16> = (0..64).map(|i| ((i * 13) % 9) as i16 - 4).collect();
        let rlgr1 = rlgr1_encode(&values);
        let rlgr3 = rlgr3_encode(&values);
        assert_ne!(rlgr1, rlgr3);
        assert_eq!(rlgr1_decode(&rlgr1, values.len()).unwrap(), values);
        assert_eq!(rlgr3_decode(&rlgr3, values.len()).unwrap(), values);
    }

    #[test]
    fn dwt_decode_all_zero_stays_zero() {
        let mut coeffs = [0i16; COEFF_COUNT];
        dwt_decode(&mut coeffs);
        assert_eq!(coeffs, [0i16; COEFF_COUNT]);
    }

    #[test]
    fn dwt_decode_constant_dc_band_reconstructs_flat_image() {
        // Hand-verified against the lifting equations: when every AC
        // (non-LL3) sub-band is zero, each level's inverse step degenerates
        // to `l_row[x] = ll[n]` and `l_row[x+1] = (l_row[x]+l_row[x+2])/2`
        // (both exactly `ll[n]` when `ll` is constant, since doubling and
        // halving a constant is lossless), so a constant `LL3` sub-band
        // reconstructs to a perfectly flat 64x64 image of that same value
        // at every one of the three levels.
        for c in [0i16, 1, -1, 17, -200, 4000] {
            let mut coeffs = [0i16; COEFF_COUNT];
            for v in &mut coeffs[LL3_OFFSET..LL3_OFFSET + 64] {
                *v = c;
            }
            dwt_decode(&mut coeffs);
            assert!(
                coeffs.iter().all(|&v| v == c),
                "constant LL3={c} did not reconstruct to a flat image"
            );
        }
    }

    #[test]
    fn codec_quant_roundtrip() {
        let quant = CodecQuant {
            factors: [6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        };
        assert_eq!(CodecQuant::decode(quant.encode()), quant);
    }

    #[test]
    fn codec_quant_wire_shape() {
        // Nibble packing: byte0 = LL3<<4|LH3, ..., byte4 = HL1<<4|HH1.
        let quant = CodecQuant {
            factors: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        };
        assert_eq!(quant.encode(), [0x12, 0x34, 0x56, 0x78, 0x9A]);
    }

    #[test]
    fn dequantize_shifts_each_subband_by_factor_minus_one() {
        let mut coeffs = [1i16; COEFF_COUNT];
        // All factors 7 -> shift by 6 everywhere -> every coefficient 64.
        let quant = CodecQuant { factors: [7; 10] };
        dequantize(&mut coeffs, &quant);
        assert!(coeffs.iter().all(|&c| c == 64));
    }

    #[test]
    fn ycbcr_to_rgb_gray_is_gray() {
        // Y=0 (after +128 level shift => 128), Cb=Cr=0 -> neutral gray.
        let (r, g, b) = ycbcr_to_rgb(0, 0, 0);
        assert_eq!((r, g, b), (128, 128, 128));
    }

    #[test]
    fn ycbcr_to_rgb_clamps() {
        // Extreme Y saturates white; extreme negative saturates black.
        let (r, g, b) = ycbcr_to_rgb(i16::MAX, 0, 0);
        assert_eq!((r, g, b), (255, 255, 255));
        let (r, g, b) = ycbcr_to_rgb(i16::MIN, 0, 0);
        assert_eq!((r, g, b), (0, 0, 0));
    }

    #[test]
    fn tile_wire_roundtrip() {
        let tile = Tile {
            quant_idx_y: 0,
            quant_idx_cb: 0,
            quant_idx_cr: 0,
            x_idx: 3,
            y_idx: 5,
            y_data: vec![0xAA; 10],
            cb_data: vec![0xBB; 4],
            cr_data: vec![0xCC; 7],
        };
        assert_eq!(Tile::decode(&tile.encode()).unwrap(), tile);
    }

    #[test]
    fn tile_rejects_wrong_block_type() {
        let mut bytes = Tile {
            quant_idx_y: 0,
            quant_idx_cb: 0,
            quant_idx_cr: 0,
            x_idx: 0,
            y_idx: 0,
            y_data: vec![],
            cb_data: vec![],
            cr_data: vec![],
        }
        .encode();
        bytes[0] = 0x00;
        assert!(Tile::decode(&bytes).is_err());
    }

    /// Build a `TS_RFX_TILE` whose components RLGR1-encode all-zero
    /// coefficient buffers (the simplest exactly-decodable case, since
    /// dequantizing/inverse-DWT-ing an all-zero buffer stays all zero) to
    /// exercise the full `decode_rgb` pipeline end to end.
    fn flat_tile(x_idx: u16, y_idx: u16) -> Tile {
        let zeros = rlgr1_encode(&[0i16; COEFF_COUNT]);
        Tile {
            quant_idx_y: 0,
            quant_idx_cb: 0,
            quant_idx_cr: 0,
            x_idx,
            y_idx,
            y_data: zeros.clone(),
            cb_data: zeros.clone(),
            cr_data: zeros,
        }
    }

    #[test]
    fn tile_decode_rgb_all_zero_is_neutral_gray() {
        let tile = flat_tile(0, 0);
        let quant_vals = vec![CodecQuant { factors: [10; 10] }];
        let rgb = tile
            .decode_rgb(&quant_vals, EntropyAlgorithm::Rlgr1)
            .unwrap();
        assert_eq!(rgb.len(), COEFF_COUNT * 3);
        assert!(rgb.chunks(3).all(|p| p == [128, 128, 128]));
    }

    #[test]
    fn tileset_wire_roundtrip() {
        let tileset = TileSet {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quant_vals: vec![
                CodecQuant { factors: [10; 10] },
                CodecQuant { factors: [8; 10] },
            ],
            tiles: vec![flat_tile(0, 0), flat_tile(1, 0), flat_tile(0, 1)],
        };
        let decoded = TileSet::decode(&tileset.encode()).unwrap();
        assert_eq!(decoded, tileset);
    }

    #[test]
    fn tileset_wire_roundtrip_rlgr3() {
        let tileset = TileSet {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quant_vals: vec![CodecQuant { factors: [9; 10] }],
            tiles: vec![flat_tile(0, 0)],
        };
        let decoded = TileSet::decode(&tileset.encode()).unwrap();
        assert_eq!(decoded, tileset);
        assert_eq!(decoded.entropy_algorithm, EntropyAlgorithm::Rlgr3);
    }

    #[test]
    fn tileset_decode_rgb_end_to_end() {
        let tileset = TileSet {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quant_vals: vec![CodecQuant { factors: [10; 10] }],
            tiles: vec![flat_tile(2, 1)],
        };
        let encoded = tileset.encode();
        let decoded = TileSet::decode(&encoded).unwrap();
        let rgb = decoded.tiles[0]
            .decode_rgb(&decoded.quant_vals, decoded.entropy_algorithm)
            .unwrap();
        assert!(rgb.chunks(3).all(|p| p == [128, 128, 128]));
    }

    #[test]
    fn tileset_rejects_unknown_entropy_algorithm() {
        let mut bytes = TileSet {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quant_vals: vec![],
            tiles: vec![],
        }
        .encode();
        // properties is at byte offset 12 (6-byte CodecChannelT header +
        // subtype(2) + idx(2)); et occupies bits 2-5, currently 0b0001.
        // Clear and set it to an unassigned value (0b0000).
        bytes[12] &= !0b0011_1100;
        assert!(TileSet::decode(&bytes).is_err());
    }

    #[test]
    fn sync_pdu_wire_shape_and_roundtrip() {
        let bytes = SyncPdu.encode();
        // blockType(2)+blockLen(4)+magic(4)+version(2) = 12 bytes.
        assert_eq!(bytes.len(), 12);
        assert_eq!(peek_block_type(&bytes).unwrap(), WBT_SYNC);
        assert_eq!(SyncPdu::decode(&bytes).unwrap(), SyncPdu);
    }

    #[test]
    fn sync_pdu_rejects_wrong_magic() {
        let mut bytes = SyncPdu.encode();
        bytes[6] ^= 0xFF; // corrupt the first magic byte
        assert!(SyncPdu::decode(&bytes).is_err());
    }

    #[test]
    fn sync_pdu_rejects_wrong_version() {
        let mut bytes = SyncPdu.encode();
        let len = bytes.len();
        bytes[len - 1] = 0x02; // corrupt the version's high byte
        assert!(SyncPdu::decode(&bytes).is_err());
    }

    #[test]
    fn codec_versions_pdu_wire_shape_and_roundtrip() {
        let bytes = CodecVersionsPdu.encode();
        // blockType(2)+blockLen(4)+numCodecs(1)+codecId(1)+version(2) = 10.
        assert_eq!(bytes.len(), 10);
        assert_eq!(peek_block_type(&bytes).unwrap(), WBT_CODEC_VERSIONS);
        assert_eq!(CodecVersionsPdu::decode(&bytes).unwrap(), CodecVersionsPdu);
    }

    #[test]
    fn channels_pdu_roundtrip_multiple() {
        let pdu = ChannelsPdu {
            channels: vec![
                RfxChannel {
                    channel_id: 0,
                    width: 1920,
                    height: 1080,
                },
                RfxChannel {
                    channel_id: 0,
                    width: 1024,
                    height: 768,
                },
            ],
        };
        assert_eq!(peek_block_type(&pdu.encode()).unwrap(), WBT_CHANNELS);
        assert_eq!(ChannelsPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn channels_pdu_roundtrip_empty() {
        let pdu = ChannelsPdu { channels: vec![] };
        assert_eq!(ChannelsPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn context_pdu_roundtrip_image_mode_rlgr1() {
        let pdu = ContextPdu {
            image_mode: true,
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
        };
        assert_eq!(peek_block_type(&pdu.encode()).unwrap(), WBT_CONTEXT);
        assert_eq!(ContextPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn context_pdu_roundtrip_video_mode_rlgr3() {
        let pdu = ContextPdu {
            image_mode: false,
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
        };
        assert_eq!(ContextPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn context_pdu_image_mode_bit_is_the_documented_bit() {
        // CODEC_MODE (image mode) is a distinct bit from the entropy field;
        // flipping entropy alone must not flip image_mode, and vice versa.
        let image_rlgr1 = ContextPdu {
            image_mode: true,
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
        };
        let video_rlgr1 = ContextPdu {
            image_mode: false,
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
        };
        let image_rlgr3 = ContextPdu {
            image_mode: true,
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
        };
        assert!(
            ContextPdu::decode(&image_rlgr1.encode())
                .unwrap()
                .image_mode
        );
        assert!(
            !ContextPdu::decode(&video_rlgr1.encode())
                .unwrap()
                .image_mode
        );
        assert_eq!(
            ContextPdu::decode(&image_rlgr3.encode())
                .unwrap()
                .entropy_algorithm,
            EntropyAlgorithm::Rlgr3
        );
        assert!(
            ContextPdu::decode(&image_rlgr3.encode())
                .unwrap()
                .image_mode
        );
    }

    #[test]
    fn region_pdu_roundtrip_with_rects() {
        let pdu = RegionPdu {
            rects: vec![
                RfxRect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
                RfxRect {
                    x: 64,
                    y: 0,
                    width: 128,
                    height: 96,
                },
            ],
        };
        assert_eq!(peek_block_type(&pdu.encode()).unwrap(), WBT_REGION);
        assert_eq!(RegionPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn region_pdu_roundtrip_empty_means_whole_frame() {
        let pdu = RegionPdu { rects: vec![] };
        assert_eq!(RegionPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn region_pdu_rejects_wrong_region_type() {
        let mut bytes = RegionPdu { rects: vec![] }.encode();
        let len = bytes.len();
        // regionType is the 2 bytes right before the trailing numTilesets(2).
        bytes[len - 4] ^= 0xFF;
        assert!(RegionPdu::decode(&bytes).is_err());
    }

    #[test]
    fn frame_begin_pdu_roundtrip() {
        let pdu = FrameBeginPdu {
            frame_idx: 7,
            num_regions: 1,
        };
        assert_eq!(peek_block_type(&pdu.encode()).unwrap(), WBT_FRAME_BEGIN);
        assert_eq!(FrameBeginPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn frame_end_pdu_wire_shape_and_roundtrip() {
        let bytes = FrameEndPdu.encode();
        assert_eq!(bytes.len(), 8); // CodecChannelT only, no body.
        assert_eq!(peek_block_type(&bytes).unwrap(), WBT_FRAME_END);
        assert_eq!(FrameEndPdu::decode(&bytes).unwrap(), FrameEndPdu);
    }

    /// Simulate a full RemoteFX image-mode frame: the one-time SYNC/
    /// CODEC_VERSIONS/CHANNELS/CONTEXT headers, then a FRAME_BEGIN/REGION/
    /// TILESET/FRAME_END quartet, dispatching each message by
    /// `peek_block_type` the way a caller streaming a live capture would.
    #[test]
    fn full_remotefx_stream_sequence() {
        let mut messages: Vec<Vec<u8>> = vec![
            SyncPdu.encode(),
            CodecVersionsPdu.encode(),
            ChannelsPdu {
                channels: vec![RfxChannel {
                    channel_id: 0,
                    width: 1024,
                    height: 768,
                }],
            }
            .encode(),
            ContextPdu {
                image_mode: true,
                entropy_algorithm: EntropyAlgorithm::Rlgr1,
            }
            .encode(),
            FrameBeginPdu {
                frame_idx: 0,
                num_regions: 1,
            }
            .encode(),
            RegionPdu {
                rects: vec![RfxRect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                }],
            }
            .encode(),
        ];
        messages.push(
            TileSet {
                entropy_algorithm: EntropyAlgorithm::Rlgr1,
                quant_vals: vec![CodecQuant { factors: [10; 10] }],
                tiles: vec![flat_tile(0, 0)],
            }
            .encode(),
        );
        messages.push(FrameEndPdu.encode());

        let expected_types = [
            WBT_SYNC,
            WBT_CODEC_VERSIONS,
            WBT_CHANNELS,
            WBT_CONTEXT,
            WBT_FRAME_BEGIN,
            WBT_REGION,
            WBT_EXTENSION,
            WBT_FRAME_END,
        ];
        for (msg, &expected) in messages.iter().zip(&expected_types) {
            assert_eq!(peek_block_type(msg).unwrap(), expected);
        }

        // Decode each by its dispatched type and confirm the payload.
        assert_eq!(SyncPdu::decode(&messages[0]).unwrap(), SyncPdu);
        assert_eq!(
            CodecVersionsPdu::decode(&messages[1]).unwrap(),
            CodecVersionsPdu
        );
        assert_eq!(ChannelsPdu::decode(&messages[2]).unwrap().channels.len(), 1);
        let context = ContextPdu::decode(&messages[3]).unwrap();
        assert!(context.image_mode);
        let frame_begin = FrameBeginPdu::decode(&messages[4]).unwrap();
        assert_eq!(frame_begin.num_regions, 1);
        let region = RegionPdu::decode(&messages[5]).unwrap();
        assert_eq!(region.rects.len(), 1);
        let tileset = TileSet::decode(&messages[6]).unwrap();
        let rgb = tileset.tiles[0]
            .decode_rgb(&tileset.quant_vals, tileset.entropy_algorithm)
            .unwrap();
        assert!(rgb.chunks(3).all(|p| p == [128, 128, 128]));
        assert_eq!(FrameEndPdu::decode(&messages[7]).unwrap(), FrameEndPdu);
    }
}
