#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! # `rusty-ttf-parser`
//!
//! A small, `#![no_std]`, allocation-free, dependency-free reader for the parts
//! of TrueType/OpenType fonts that text *shaping and colour rendering* need and
//! outline rasterisers do not:
//!
//! - the table directory, including TrueType Collections ([`Face::parse`]);
//! - `cmap` character mapping ([`Face::glyph_index`]);
//! - `GSUB` features and lookups ([`gsub`], [`opentype_layout`]);
//! - PNG colour bitmaps from `sbix` and `CBLC`/`CBDT`
//!   ([`Face::glyph_raster_image`]).
//!
//! Outlines, metrics and rasterisation live in `rusty_font`; this crate is the
//! layout-table complement, and replaces `rusty_term`'s use of `ttf-parser`.
//!
//! Every read is bounds-checked and returns `Option`/`Result`; malformed input
//! never panics. Parsed tables borrow the font bytes and decode lazily.
//!
//! Names follow `ttf-parser` where there is an equivalent, to keep migration
//! mechanical; the surface is intentionally a small subset.

mod cmap;
pub mod gsub;
pub mod opentype_layout;
mod raster;
mod reader;

pub use raster::{RasterGlyphImage, RasterImageFormat};
pub use reader::{FromData, FromSlice, LazyArray16, OffsetArray16};

use reader::{u16_at, u32_at};

/// A glyph index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u16);

/// A four-byte table or feature tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(pub u32);

impl Tag {
    /// Build a tag from its four ASCII bytes, e.g. `b"liga"`.
    pub const fn from_bytes(bytes: &[u8; 4]) -> Self {
        Tag(u32::from_be_bytes(*bytes))
    }
}

/// Why [`Face::parse`] refused a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceParsingError {
    /// The file does not start with a known sfnt or collection header.
    UnknownMagic,
    /// The requested face index is not in the collection.
    FaceIndexOutOfBounds,
    /// A required table (`head`, `maxp`) is missing or truncated.
    MalformedFont,
}

impl core::fmt::Display for FaceParsingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            FaceParsingError::UnknownMagic => "unknown font file signature",
            FaceParsingError::FaceIndexOutOfBounds => "face index out of bounds",
            FaceParsingError::MalformedFont => "malformed or truncated font",
        })
    }
}

impl core::error::Error for FaceParsingError {}

/// Tables decoded on demand by [`Face::tables`].
#[derive(Clone, Copy, Debug)]
pub struct Tables<'a> {
    /// The `GSUB` table, if present and well-formed.
    pub gsub: Option<gsub::Table<'a>>,
}

/// A parsed font face borrowing the font file's bytes.
#[derive(Clone, Copy, Debug)]
pub struct Face<'a> {
    data: &'a [u8],
    /// Table directory records (16 bytes each), already bounds-checked.
    records: &'a [u8],
    units_per_em: u16,
    glyph_count: u16,
    cmap: Option<cmap::Subtable<'a>>,
}

impl<'a> Face<'a> {
    /// Parse face `index` of a font file (`0` for a plain `.ttf`/`.otf`).
    pub fn parse(data: &'a [u8], index: u32) -> Result<Self, FaceParsingError> {
        let directory = match u32_at(data, 0).ok_or(FaceParsingError::UnknownMagic)? {
            0x7474_6366 => {
                // 'ttcf': header, version, numFonts, offsets[numFonts].
                let count = u32_at(data, 8).ok_or(FaceParsingError::MalformedFont)?;
                if index >= count {
                    return Err(FaceParsingError::FaceIndexOutOfBounds);
                }
                let at = 12usize
                    .checked_add(
                        usize::try_from(index).map_err(|_| FaceParsingError::MalformedFont)? * 4,
                    )
                    .ok_or(FaceParsingError::MalformedFont)?;
                let offset = u32_at(data, at).ok_or(FaceParsingError::MalformedFont)?;
                reader::tail32(data, offset).ok_or(FaceParsingError::MalformedFont)?
            }
            // TrueType, 'OTTO', 'true', 'typ1'.
            0x0001_0000 | 0x4F54_544F | 0x7472_7565 | 0x7479_7031 => {
                if index != 0 {
                    return Err(FaceParsingError::FaceIndexOutOfBounds);
                }
                data
            }
            _ => return Err(FaceParsingError::UnknownMagic),
        };
        let table_count = usize::from(u16_at(directory, 4).ok_or(FaceParsingError::MalformedFont)?);
        let records = directory
            .get(12..12 + table_count * 16)
            .ok_or(FaceParsingError::MalformedFont)?;

        let mut face = Face {
            data,
            records,
            units_per_em: 0,
            glyph_count: 0,
            cmap: None,
        };
        let head = face
            .table_data(Tag::from_bytes(b"head"))
            .ok_or(FaceParsingError::MalformedFont)?;
        face.units_per_em = u16_at(head, 18)
            .filter(|upem| *upem != 0)
            .ok_or(FaceParsingError::MalformedFont)?;
        let maxp = face
            .table_data(Tag::from_bytes(b"maxp"))
            .ok_or(FaceParsingError::MalformedFont)?;
        face.glyph_count = u16_at(maxp, 4).ok_or(FaceParsingError::MalformedFont)?;
        face.cmap = face
            .table_data(Tag::from_bytes(b"cmap"))
            .and_then(cmap::select);
        Ok(face)
    }

    /// The raw bytes of table `tag`, clamped to the file, or `None` if absent.
    pub fn table_data(&self, tag: Tag) -> Option<&'a [u8]> {
        self.records.chunks_exact(16).find_map(|record| {
            if u32_at(record, 0)? != tag.0 {
                return None;
            }
            let offset = usize::try_from(u32_at(record, 8)?).ok()?;
            let length = usize::try_from(u32_at(record, 12)?).ok()?;
            let end = offset.saturating_add(length).min(self.data.len());
            self.data.get(offset..end)
        })
    }

    /// Design units per em square (never 0).
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Number of glyphs, from `maxp`.
    pub fn number_of_glyphs(&self) -> u16 {
        self.glyph_count
    }

    /// The glyph for `c`, or `None` if the font does not map it.
    pub fn glyph_index(&self, c: char) -> Option<GlyphId> {
        self.cmap?.glyph_index(u32::from(c)).map(GlyphId)
    }

    /// Decode the layout tables this crate understands.
    pub fn tables(&self) -> Tables<'a> {
        Tables {
            gsub: self
                .table_data(Tag::from_bytes(b"GSUB"))
                .and_then(gsub::Table::parse),
        }
    }
}
