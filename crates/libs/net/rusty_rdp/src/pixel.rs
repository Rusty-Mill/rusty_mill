//! Pixel-format unpacking to RGBA.
//!
//! RDP delivers bitmap pixels in one of several packed formats. Once a
//! rectangle has been decoded (and RLE-decompressed, see [`crate::rle`]) its
//! bytes are still in that native format; this module expands them into a
//! plain top-down **RGBA8888** framebuffer that a display can consume.
//!
//! Supported source formats:
//!
//! | bpp | layout | notes |
//! |-----|--------|-------|
//! | 8   | palette index | needs the color table from a Palette Update |
//! | 15  | `0RRRRRGGGGGBBBBB` (LE) | 5/5/5 |
//! | 16  | `RRRRRGGGGGGBBBBB` (LE) | 5/6/5 |
//! | 24  | `B G R` | one byte each |
//! | 32  | `B G R X` | X ignored, alpha forced opaque |
//!
//! RDP bitmap scanlines are conventionally **bottom-up** (both uncompressed
//! `TS_BITMAP_DATA` and the [`crate::rle`] decoder emit the bottom row first),
//! so [`to_rgba`] flips by default. Pass `bottom_up = false` for a source that
//! is already top-down.

use crate::error::{Error, Result};
use crate::output::PaletteEntry;

/// Bytes emitted per pixel in the RGBA output.
pub const RGBA_BYTES: usize = 4;

fn source_bytes_per_pixel(bits: u16) -> Result<usize> {
    match bits {
        8 => Ok(1),
        15 | 16 => Ok(2),
        24 => Ok(3),
        32 => Ok(4),
        other => Err(Error::InvalidValue {
            field: "pixel bitsPerPixel",
            value: other.to_string(),
        }),
    }
}

/// Expand a 5-bit channel to 8 bits.
fn expand5(v: u16) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

/// Expand a 6-bit channel to 8 bits.
fn expand6(v: u16) -> u8 {
    ((v << 2) | (v >> 4)) as u8
}

/// Convert one native pixel value to `(r, g, b)`.
fn to_rgb(
    bits_per_pixel: u16,
    pixel: &[u8],
    palette: Option<&[PaletteEntry]>,
) -> Result<(u8, u8, u8)> {
    Ok(match bits_per_pixel {
        8 => {
            let idx = pixel[0] as usize;
            let table = palette.ok_or(Error::InvalidValue {
                field: "palette",
                value: "required for 8bpp".to_string(),
            })?;
            let entry = table.get(idx).ok_or(Error::InvalidValue {
                field: "palette index",
                value: idx.to_string(),
            })?;
            (entry.red, entry.green, entry.blue)
        }
        15 => {
            let v = u16::from_le_bytes([pixel[0], pixel[1]]);
            (
                expand5((v >> 10) & 0x1F),
                expand5((v >> 5) & 0x1F),
                expand5(v & 0x1F),
            )
        }
        16 => {
            let v = u16::from_le_bytes([pixel[0], pixel[1]]);
            (
                expand5((v >> 11) & 0x1F),
                expand6((v >> 5) & 0x3F),
                expand5(v & 0x1F),
            )
        }
        24 => (pixel[2], pixel[1], pixel[0]), // stored B, G, R
        32 => (pixel[2], pixel[1], pixel[0]), // stored B, G, R, X
        other => {
            return Err(Error::InvalidValue {
                field: "pixel bitsPerPixel",
                value: other.to_string(),
            });
        }
    })
}

/// Unpack native pixel bytes into a top-down RGBA8888 buffer.
///
/// `pixels` must hold `width * height * bytesPerPixel` bytes. Returns
/// `width * height * 4` bytes, row-major top-down, alpha forced to `0xFF`.
///
/// Set `bottom_up` when the source's first row is the bottom of the image
/// (the RDP default).
pub fn to_rgba(
    pixels: &[u8],
    width: usize,
    height: usize,
    bits_per_pixel: u16,
    palette: Option<&[PaletteEntry]>,
    bottom_up: bool,
) -> Result<Vec<u8>> {
    let bpp = source_bytes_per_pixel(bits_per_pixel)?;
    let row_bytes = width
        .checked_mul(bpp)
        .ok_or(Error::Overflow { field: "pixel row" })?;
    let needed = row_bytes.checked_mul(height).ok_or(Error::Overflow {
        field: "pixel buffer",
    })?;
    if pixels.len() < needed {
        return Err(Error::UnexpectedEof {
            needed,
            available: pixels.len(),
        });
    }

    let mut out = vec![0u8; width * height * RGBA_BYTES];
    for y in 0..height {
        let src_row = if bottom_up { height - 1 - y } else { y };
        let src_base = src_row * row_bytes;
        let dst_base = y * width * RGBA_BYTES;
        for x in 0..width {
            let src = &pixels[src_base + x * bpp..src_base + x * bpp + bpp];
            let (r, g, b) = to_rgb(bits_per_pixel, src, palette)?;
            let dst = dst_base + x * RGBA_BYTES;
            out[dst] = r;
            out[dst + 1] = g;
            out[dst + 2] = b;
            out[dst + 3] = 0xFF;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal(entries: &[(u8, u8, u8)]) -> Vec<PaletteEntry> {
        entries
            .iter()
            .map(|&(red, green, blue)| PaletteEntry { red, green, blue })
            .collect()
    }

    #[test]
    fn indexed_8bpp_uses_palette() {
        let palette = pal(&[(0, 0, 0), (10, 20, 30), (255, 255, 255)]);
        // One row, top-down: indices 2, 1.
        let out = to_rgba(&[2, 1], 2, 1, 8, Some(&palette), false).unwrap();
        assert_eq!(out, [255, 255, 255, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn rgb565_channels() {
        // Red, green, blue maxima in 5-6-5.
        let red = 0xF800u16.to_le_bytes();
        let green = 0x07E0u16.to_le_bytes();
        let blue = 0x001Fu16.to_le_bytes();
        let mut px = Vec::new();
        px.extend_from_slice(&red);
        px.extend_from_slice(&green);
        px.extend_from_slice(&blue);
        let out = to_rgba(&px, 3, 1, 16, None, false).unwrap();
        assert_eq!(out, [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn rgb555_channels() {
        let red = 0x7C00u16.to_le_bytes();
        let out = to_rgba(&red, 1, 1, 15, None, false).unwrap();
        assert_eq!(out, [255, 0, 0, 255]);
    }

    #[test]
    fn bgr24_is_reordered() {
        // Stored blue, green, red.
        let out = to_rgba(&[0x11, 0x22, 0x33], 1, 1, 24, None, false).unwrap();
        assert_eq!(out, [0x33, 0x22, 0x11, 0xFF]);
    }

    #[test]
    fn bgrx32_forces_opaque_alpha() {
        // Stored B, G, R, X (X ignored).
        let out = to_rgba(&[0x11, 0x22, 0x33, 0x00], 1, 1, 32, None, false).unwrap();
        assert_eq!(out, [0x33, 0x22, 0x11, 0xFF]);
    }

    #[test]
    fn bottom_up_flips_rows() {
        // Two rows of one 24bpp pixel each; bottom-up source.
        let bottom = [0xAA, 0xAA, 0xAA];
        let top = [0xBB, 0xBB, 0xBB];
        let mut px = Vec::new();
        px.extend_from_slice(&bottom); // source row 0 = bottom
        px.extend_from_slice(&top); // source row 1 = top
        let out = to_rgba(&px, 1, 2, 24, None, true).unwrap();
        // Output row 0 should be the top of the image (0xBB).
        assert_eq!(&out[..4], &[0xBB, 0xBB, 0xBB, 0xFF]);
        assert_eq!(&out[4..], &[0xAA, 0xAA, 0xAA, 0xFF]);
    }

    #[test]
    fn missing_palette_errors() {
        assert!(matches!(
            to_rgba(&[0], 1, 1, 8, None, false).unwrap_err(),
            Error::InvalidValue {
                field: "palette",
                ..
            }
        ));
    }

    #[test]
    fn short_buffer_errors() {
        assert!(matches!(
            to_rgba(&[0x00], 2, 2, 16, None, false).unwrap_err(),
            Error::UnexpectedEof { .. }
        ));
    }

    #[test]
    fn unsupported_depth_errors() {
        assert!(to_rgba(&[0; 4], 1, 1, 4, None, false).is_err());
    }
}
