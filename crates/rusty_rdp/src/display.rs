//! A simple RGBA framebuffer for assembling server bitmap updates.
//!
//! The server sends the desktop as a stream of rectangle updates
//! ([`crate::output`]); each decodes to a top-down RGBA image
//! ([`BitmapData::to_rgba`]). This module accumulates those rectangles into a
//! full-desktop framebuffer so a caller can display or dump the result.
//!
//! It is a plain byte buffer with no rendering dependencies — the pixels are
//! `width * height * 4` bytes of top-down RGBA8888.

use crate::error::{Error, Result};
use crate::output::{BitmapData, PaletteEntry};

/// A top-down RGBA8888 desktop framebuffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framebuffer {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height * 4` bytes of top-down RGBA.
    pub rgba: Vec<u8>,
}

impl Framebuffer {
    /// Create a black, fully-opaque framebuffer of the given size.
    pub fn new(width: usize, height: usize) -> Self {
        let mut rgba = vec![0u8; width * height * 4];
        for a in rgba[3..].iter_mut().step_by(4) {
            *a = 0xFF;
        }
        Framebuffer {
            width,
            height,
            rgba,
        }
    }

    /// Copy a top-down RGBA rectangle to `(x, y)`, clipping to the buffer.
    ///
    /// `src` must be `w * h * 4` bytes. Rows and columns that fall outside the
    /// framebuffer are skipped.
    pub fn blit(&mut self, x: usize, y: usize, w: usize, h: usize, src: &[u8]) -> Result<()> {
        let needed = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::Overflow { field: "blit rect" })?;
        if src.len() < needed {
            return Err(Error::UnexpectedEof {
                needed,
                available: src.len(),
            });
        }
        for row in 0..h {
            let dst_y = y + row;
            if dst_y >= self.height {
                break;
            }
            for col in 0..w {
                let dst_x = x + col;
                if dst_x >= self.width {
                    break;
                }
                let src_off = (row * w + col) * 4;
                let dst_off = (dst_y * self.width + dst_x) * 4;
                self.rgba[dst_off..dst_off + 4].copy_from_slice(&src[src_off..src_off + 4]);
            }
        }
        Ok(())
    }

    /// Decode a bitmap rectangle and blit it at its destination position.
    ///
    /// `palette` supplies the color table for 8bpp data.
    pub fn apply_bitmap(
        &mut self,
        bitmap: &BitmapData,
        palette: Option<&[PaletteEntry]>,
    ) -> Result<()> {
        let rgba = bitmap.to_rgba(palette)?;
        self.blit(
            bitmap.dest_left as usize,
            bitmap.dest_top as usize,
            bitmap.width as usize,
            bitmap.height as usize,
            &rgba,
        )
    }

    /// Serialize the framebuffer as a binary PPM (P6) image.
    ///
    /// PPM is a trivial, dependency-free format that most image viewers open.
    /// The alpha channel is dropped.
    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.reserve(self.width * self.height * 3);
        for px in self.rgba.chunks_exact(4) {
            out.push(px[0]);
            out.push(px[1]);
            out.push(px[2]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(fb: &Framebuffer, x: usize, y: usize) -> [u8; 4] {
        let off = (y * fb.width + x) * 4;
        [
            fb.rgba[off],
            fb.rgba[off + 1],
            fb.rgba[off + 2],
            fb.rgba[off + 3],
        ]
    }

    #[test]
    fn new_is_black_opaque() {
        let fb = Framebuffer::new(2, 2);
        assert_eq!(px(&fb, 0, 0), [0, 0, 0, 0xFF]);
        assert_eq!(px(&fb, 1, 1), [0, 0, 0, 0xFF]);
    }

    #[test]
    fn blit_places_rect() {
        let mut fb = Framebuffer::new(4, 4);
        // A 2x2 red rectangle at (1, 1).
        let red: Vec<u8> = [255, 0, 0, 255]
            .iter()
            .cycle()
            .take(2 * 2 * 4)
            .copied()
            .collect();
        fb.blit(1, 1, 2, 2, &red).unwrap();
        assert_eq!(px(&fb, 0, 0), [0, 0, 0, 0xFF]); // untouched
        assert_eq!(px(&fb, 1, 1), [255, 0, 0, 255]);
        assert_eq!(px(&fb, 2, 2), [255, 0, 0, 255]);
        assert_eq!(px(&fb, 3, 3), [0, 0, 0, 0xFF]); // outside the rect
    }

    #[test]
    fn blit_clips_to_bounds() {
        let mut fb = Framebuffer::new(2, 2);
        // A 2x2 rect at (1, 1) overhangs the buffer; only (1,1) lands.
        let white: Vec<u8> = vec![0xFF; 2 * 2 * 4];
        fb.blit(1, 1, 2, 2, &white).unwrap();
        assert_eq!(px(&fb, 1, 1), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(px(&fb, 0, 0), [0, 0, 0, 0xFF]);
    }

    #[test]
    fn apply_bitmap_decodes_and_blits() {
        // A 2x1 uncompressed 16bpp rectangle (red, green) at (0, 0), bottom-up.
        let red = 0xF800u16.to_le_bytes();
        let green = 0x07E0u16.to_le_bytes();
        let mut pixels = Vec::new();
        pixels.extend_from_slice(&red);
        pixels.extend_from_slice(&green);
        let bitmap = BitmapData::uncompressed(0, 0, 2, 1, 16, pixels);
        let mut fb = Framebuffer::new(2, 1);
        fb.apply_bitmap(&bitmap, None).unwrap();
        assert_eq!(px(&fb, 0, 0), [255, 0, 0, 255]);
        assert_eq!(px(&fb, 1, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn ppm_header_and_size() {
        let fb = Framebuffer::new(2, 1);
        let ppm = fb.to_ppm();
        assert!(ppm.starts_with(b"P6\n2 1\n255\n"));
        // 2 pixels * 3 bytes after the header.
        assert_eq!(ppm.len(), b"P6\n2 1\n255\n".len() + 6);
    }

    #[test]
    fn blit_short_source_errors() {
        let mut fb = Framebuffer::new(4, 4);
        assert!(fb.blit(0, 0, 2, 2, &[0; 4]).is_err());
    }
}
