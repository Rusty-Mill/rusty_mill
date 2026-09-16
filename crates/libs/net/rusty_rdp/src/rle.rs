//! Interleaved RLE bitmap decompression (MS-RDPEGDI 2.2.2.5 / 3.1.9).
//!
//! RDP's classic bitmap codec is a byte-oriented, interleaved run-length
//! scheme that predicts each pixel from the one in the previously decoded
//! scanline. It works on pixels of 1, 2, or 3 bytes (8/15/16/24 bpp).
//!
//! The compressed stream is a series of *orders*, each a control byte
//! (optionally followed by a length and colour data) selecting one of:
//!
//! * **background run** — copy the pixel from the previous scanline;
//! * **foreground run** — previous-scanline pixel XOR the current mix colour;
//! * **fill-or-mix (FOM)** — a bitmask chooses background or foreground per
//!   pixel;
//! * **colour run** — a solid colour;
//! * **colour image** — literal pixels copied from the stream;
//! * **dithered (bicolour) run** — two colours alternating;
//! * **black / white** runs.
//!
//! Each order comes in a *regular*, *lite*, and *mega-mega* size form, plus a
//! few fixed *special* orders. The output places scanlines in transmission
//! order (the first decoded line at offset 0); the pixel bytes for each line
//! are little-endian, matching an uncompressed `TS_BITMAP_DATA` stream.
//!
//! This implementation follows the well-known reference decoder shared by
//! rdesktop and FreeRDP.

use crate::error::{Error, Result};

/// A tiny bounds-checked byte reader for the compressed stream.
struct In<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> In<'a> {
    fn new(buf: &'a [u8]) -> Self {
        In { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn next(&mut self) -> Result<u8> {
        let b = *self.buf.get(self.pos).ok_or(Error::UnexpectedEof {
            needed: 1,
            available: 0,
        })?;
        self.pos += 1;
        Ok(b)
    }

    /// Read one `bpp`-byte little-endian pixel.
    fn pixel(&mut self, bpp: usize) -> Result<u32> {
        let mut v = 0u32;
        for i in 0..bpp {
            v |= (self.next()? as u32) << (8 * i);
        }
        Ok(v)
    }
}

/// Number of bytes per pixel for a supported bit depth.
fn bytes_per_pixel(bits: u16) -> Result<usize> {
    match bits {
        8 => Ok(1),
        15 | 16 => Ok(2),
        24 => Ok(3),
        other => Err(Error::InvalidValue {
            field: "RLE bitsPerPixel",
            value: other.to_string(),
        }),
    }
}

/// Decompress an interleaved-RLE bitmap into raw little-endian pixel bytes.
///
/// The result is `width * height * bytesPerPixel` bytes, scanlines in decode
/// order. Returns an error for an unsupported depth or a malformed stream.
pub fn decompress_bitmap(
    input: &[u8],
    width: usize,
    height: usize,
    bits_per_pixel: u16,
) -> Result<Vec<u8>> {
    let bpp = bytes_per_pixel(bits_per_pixel)?;
    let row_bytes = width
        .checked_mul(bpp)
        .ok_or(Error::Overflow { field: "RLE row" })?;
    let total = row_bytes.checked_mul(height).ok_or(Error::Overflow {
        field: "RLE output",
    })?;
    let mut out = vec![0u8; total];
    let white = if bpp >= 4 {
        u32::MAX
    } else {
        (1u32 << (8 * bpp)) - 1
    };

    let mut r = In::new(input);
    // Scanline bookkeeping: `line` is the offset of the current row, `prev`
    // the previously decoded row (None on the first line). `rows` counts rows
    // already started so a new line lands at the next offset.
    let mut line: usize = 0;
    let mut prev: Option<usize> = None;
    let mut rows_started: usize = 0;
    let mut x = width; // force a new line before the first pixel

    let mut last_opcode: i32 = -1;
    let mut insert_mix = false;
    let mut bicolour = false;
    let mut mix: u32 = white;
    let mut colour1: u32 = 0;
    let mut colour2: u32 = 0;
    let mut mask: u8 = 0;
    let mut mixmask: u8;

    let set = |out: &mut [u8], off: usize, x: usize, val: u32| {
        let p = off + x * bpp;
        for i in 0..bpp {
            out[p + i] = (val >> (8 * i)) as u8;
        }
    };
    let get = |out: &[u8], off: usize, x: usize| -> u32 {
        let p = off + x * bpp;
        let mut v = 0u32;
        for i in 0..bpp {
            v |= (out[p + i] as u32) << (8 * i);
        }
        v
    };

    while !r.done() {
        let mut fom_mask: u8 = 0;
        let code = r.next()?;
        let mut opcode = (code >> 4) as i32;
        let mut count: i64;
        let offset;

        match opcode {
            0xC..=0xE => {
                // Lite orders.
                opcode -= 6;
                count = (code & 0x0F) as i64;
                offset = 16;
            }
            0xF => {
                // Mega-mega and special orders.
                opcode = (code & 0x0F) as i32;
                if opcode < 9 {
                    let lo = r.next()? as i64;
                    let hi = r.next()? as i64;
                    count = lo | (hi << 8);
                } else {
                    count = if opcode < 0xB { 8 } else { 1 };
                }
                offset = 0;
            }
            _ => {
                // Regular orders.
                opcode >>= 1;
                count = (code & 0x1F) as i64;
                offset = 32;
            }
        }

        if offset != 0 {
            let is_fom = opcode == 2 || opcode == 7;
            if count == 0 {
                count = if is_fom {
                    r.next()? as i64 + 1
                } else {
                    r.next()? as i64 + offset
                };
            } else if is_fom {
                count <<= 3;
            }
        }

        // Read any colour operands and normalise the opcode.
        match opcode {
            0 => {
                if last_opcode == opcode && !(x == width && prev.is_none()) {
                    insert_mix = true;
                }
            }
            8 => {
                colour1 = r.pixel(bpp)?;
                colour2 = r.pixel(bpp)?;
            }
            3 => {
                colour2 = r.pixel(bpp)?;
            }
            6 | 7 => {
                mix = r.pixel(bpp)?;
                opcode -= 5;
            }
            9 => {
                mask = 0x03;
                opcode = 2;
                fom_mask = 3;
            }
            0xA => {
                mask = 0x05;
                opcode = 2;
                fom_mask = 5;
            }
            _ => {}
        }
        last_opcode = opcode;
        mixmask = 0;

        while count > 0 {
            if x >= width {
                if rows_started >= height {
                    return Err(Error::InvalidValue {
                        field: "RLE height",
                        value: "too many scanlines".to_string(),
                    });
                }
                x = 0;
                prev = if rows_started == 0 { None } else { Some(line) };
                line = rows_started * row_bytes;
                rows_started += 1;
            }

            match opcode {
                0 => {
                    // Background run.
                    if insert_mix {
                        let v = match prev {
                            None => mix,
                            Some(p) => get(&out, p, x) ^ mix,
                        };
                        set(&mut out, line, x, v);
                        insert_mix = false;
                        count -= 1;
                        x += 1;
                    }
                    while count > 0 && x < width {
                        let v = match prev {
                            None => 0,
                            Some(p) => get(&out, p, x),
                        };
                        set(&mut out, line, x, v);
                        count -= 1;
                        x += 1;
                    }
                }
                1 => {
                    // Foreground (mix) run.
                    while count > 0 && x < width {
                        let v = match prev {
                            None => mix,
                            Some(p) => get(&out, p, x) ^ mix,
                        };
                        set(&mut out, line, x, v);
                        count -= 1;
                        x += 1;
                    }
                }
                2 => {
                    // Fill-or-mix run/image.
                    while count > 0 && x < width {
                        mixmask <<= 1;
                        if mixmask == 0 {
                            mask = if fom_mask != 0 { fom_mask } else { r.next()? };
                            mixmask = 1;
                        }
                        let bit = mask & mixmask != 0;
                        let v = match prev {
                            None => {
                                if bit {
                                    mix
                                } else {
                                    0
                                }
                            }
                            Some(p) => {
                                let above = get(&out, p, x);
                                if bit {
                                    above ^ mix
                                } else {
                                    above
                                }
                            }
                        };
                        set(&mut out, line, x, v);
                        count -= 1;
                        x += 1;
                    }
                }
                3 => {
                    // Colour run.
                    while count > 0 && x < width {
                        set(&mut out, line, x, colour2);
                        count -= 1;
                        x += 1;
                    }
                }
                4 => {
                    // Colour image (literal pixels).
                    while count > 0 && x < width {
                        let v = r.pixel(bpp)?;
                        set(&mut out, line, x, v);
                        count -= 1;
                        x += 1;
                    }
                }
                8 => {
                    // Dithered / bicolour run.
                    while count > 0 && x < width {
                        if bicolour {
                            set(&mut out, line, x, colour2);
                            bicolour = false;
                        } else {
                            set(&mut out, line, x, colour1);
                            bicolour = true;
                            count += 1;
                        }
                        count -= 1;
                        x += 1;
                    }
                }
                0xD => {
                    // White run.
                    while count > 0 && x < width {
                        set(&mut out, line, x, white);
                        count -= 1;
                        x += 1;
                    }
                }
                0xE => {
                    // Black run.
                    while count > 0 && x < width {
                        set(&mut out, line, x, 0);
                        count -= 1;
                        x += 1;
                    }
                }
                other => {
                    return Err(Error::InvalidValue {
                        field: "RLE opcode",
                        value: other.to_string(),
                    });
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_run_first_line_is_black() {
        // Regular background run, length 4, on the first line → all zero.
        let out = decompress_bitmap(&[0x04], 4, 1, 8).unwrap();
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn colour_run() {
        // Regular colour run (0x60 | 3), colour 0xAB → three 0xAB pixels.
        let out = decompress_bitmap(&[0x63, 0xAB], 3, 1, 8).unwrap();
        assert_eq!(out, [0xAB, 0xAB, 0xAB]);
    }

    #[test]
    fn colour_image_literal_pixels() {
        // Regular colour image (0x80 | 2) copies two literal pixels.
        let out = decompress_bitmap(&[0x82, 0x11, 0x22], 2, 1, 8).unwrap();
        assert_eq!(out, [0x11, 0x22]);
    }

    #[test]
    fn white_and_black_specials() {
        assert_eq!(decompress_bitmap(&[0xFD], 1, 1, 8).unwrap(), [0xFF]);
        assert_eq!(decompress_bitmap(&[0xFE], 1, 1, 8).unwrap(), [0x00]);
    }

    #[test]
    fn foreground_run_uses_default_mix() {
        // Regular foreground run (0x20 | 2), no SetMix → mix defaults to all
        // ones, first line has no previous, so pixels are the mix value.
        let out = decompress_bitmap(&[0x22], 2, 1, 8).unwrap();
        assert_eq!(out, [0xFF, 0xFF]);
    }

    #[test]
    fn set_mix_then_mix_run() {
        // Lite SetMix/Mix (0xC0 | 2) with mix 0x55 on the first line.
        let out = decompress_bitmap(&[0xC2, 0x55], 2, 1, 8).unwrap();
        assert_eq!(out, [0x55, 0x55]);
    }

    #[test]
    fn dithered_bicolour_run() {
        // Lite dithered (0xE0 | 2) → two colour pairs across four pixels.
        let out = decompress_bitmap(&[0xE2, 0x11, 0x22], 4, 1, 8).unwrap();
        assert_eq!(out, [0x11, 0x22, 0x11, 0x22]);
    }

    #[test]
    fn background_run_copies_previous_line() {
        // Line 0 (decoded first): colour run of 0x33.
        // Line 1: background run copies the previous line verbatim.
        let out = decompress_bitmap(&[0x62, 0x33, 0x02], 2, 2, 8).unwrap();
        assert_eq!(out, [0x33, 0x33, 0x33, 0x33]);
    }

    #[test]
    fn mix_run_xors_previous_line() {
        // Line 0: colour run 0x0F. Line 1: SetMix/Mix 0x03 → 0x0F ^ 0x03.
        let out = decompress_bitmap(&[0x62, 0x0F, 0xC2, 0x03], 2, 2, 8).unwrap();
        assert_eq!(out, [0x0F, 0x0F, 0x0C, 0x0C]);
    }

    #[test]
    fn fill_or_mix_special_fgbg() {
        // SPECIAL_FGBG_1 (0xF9): FOM run, count 8, fixed mask 0x03, first line.
        // mask bit set → mix (0xFF default), clear → 0. Mask 0x03 = bits 0,1.
        let out = decompress_bitmap(&[0xF9], 8, 1, 8).unwrap();
        assert_eq!(out, [0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn sixteen_bpp_colour_run() {
        // Colour run of one 16bpp pixel 0x1F00 (little-endian on the wire).
        let out = decompress_bitmap(&[0x61, 0x00, 0x1F], 1, 1, 16).unwrap();
        assert_eq!(out, [0x00, 0x1F]);
    }

    #[test]
    fn twenty_four_bpp_colour_image() {
        // One 24bpp literal pixel (3 bytes).
        let out = decompress_bitmap(&[0x81, 0x11, 0x22, 0x33], 1, 1, 24).unwrap();
        assert_eq!(out, [0x11, 0x22, 0x33]);
    }

    #[test]
    fn rejects_unsupported_depth() {
        assert!(matches!(
            decompress_bitmap(&[0x00], 1, 1, 32).unwrap_err(),
            Error::InvalidValue {
                field: "RLE bitsPerPixel",
                ..
            }
        ));
    }

    #[test]
    fn truncated_stream_errors() {
        // Colour run announces a colour byte that is missing.
        assert!(decompress_bitmap(&[0x61], 1, 1, 8).is_err());
    }
}
