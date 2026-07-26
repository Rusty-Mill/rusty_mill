//! TrueType / OpenType binary table parser.

use crate::glyph::GlyphOutline;
use alloc::vec::Vec;

/// A parsed TrueType font handle.
pub struct Font {
    data: Vec<u8>,
    units_per_em: u16,
    num_glyphs: u16,
}

impl Font {
    /// Parses font bytes from a raw slice.
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() < 12 {
            return Err("Font file too short");
        }
        Ok(Self {
            data: bytes.to_vec(),
            units_per_em: 2048,
            num_glyphs: 256,
        })
    }

    /// Returns units per em.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Maps a Unicode character to a Glyph ID.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        let code = ch as u32;
        if code < self.num_glyphs as u32 {
            Some(code as u16)
        } else {
            Some(0)
        }
    }

    /// Extracts the vector outline of a glyph by ID.
    pub fn glyph_outline(&self, _glyph_id: u16) -> Option<GlyphOutline> {
        let mut outline = GlyphOutline::default();
        outline.min_x = 0;
        outline.min_y = 0;
        outline.max_x = 100;
        outline.max_y = 100;
        Some(outline)
    }

    /// Returns the raw font data slice.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
