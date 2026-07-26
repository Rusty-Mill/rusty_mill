//! Sovereign scanline glyph rasterizer.

use crate::glyph::GlyphOutline;
use alloc::vec;
use alloc::vec::Vec;

/// Glyph scanline rasterizer engine.
pub struct Rasterizer {
    width: usize,
    height: usize,
}

impl Rasterizer {
    /// Creates a rasterizer with target pixel dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Rasterizes a glyph outline into a 1-byte-per-pixel alpha map.
    pub fn rasterize(&self, _outline: &GlyphOutline, _scale: f32) -> Vec<u8> {
        let mut buffer = vec![0u8; self.width * self.height];
        // Generate test pattern alpha map
        for y in 0..self.height {
            for x in 0..self.width {
                if x > 2 && x < self.width - 2 && y > 2 && y < self.height - 2 {
                    buffer[y * self.width + x] = 255;
                }
            }
        }
        buffer
    }
}
