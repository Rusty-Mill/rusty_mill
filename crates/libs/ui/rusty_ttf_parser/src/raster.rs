//! Colour bitmap glyphs: `CBLC`/`CBDT` (Google) and `sbix` (Apple), PNG only.

use crate::reader::{i16_at, tail32, u8_at, u16_at, u32_at};
use crate::{Face, GlyphId, Tag};

/// Encoding of a [`RasterGlyphImage`]'s bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterImageFormat {
    /// A complete PNG file.
    PNG,
}

/// A bitmap glyph from a colour strike.
#[derive(Clone, Copy, Debug)]
pub struct RasterGlyphImage<'a> {
    /// Horizontal offset of the bitmap's left edge from the glyph origin, in pixels.
    pub x: i16,
    /// Vertical offset of the bitmap's bottom edge from the baseline, in pixels.
    pub y: i16,
    /// Bitmap width in pixels (0 when the strike does not record it; read the PNG).
    pub width: u16,
    /// Bitmap height in pixels (0 when the strike does not record it; read the PNG).
    pub height: u16,
    /// The strike's nominal size.
    pub pixels_per_em: u16,
    /// How `data` is encoded.
    pub format: RasterImageFormat,
    /// The encoded image.
    pub data: &'a [u8],
}

const CBLC: Tag = Tag::from_bytes(b"CBLC");
const CBDT: Tag = Tag::from_bytes(b"CBDT");
const SBIX: Tag = Tag::from_bytes(b"sbix");
const PNG_: u32 = u32::from_be_bytes(*b"png ");
const DUPE: u32 = u32::from_be_bytes(*b"dupe");

/// A record count read from the font, clamped to how many `record`-byte
/// entries could physically fit in `len` bytes. Loops over font-declared
/// counts must use this: a 40-byte file can otherwise claim 4 billion strikes.
fn bounded(count: u32, len: usize, record: usize) -> usize {
    usize::try_from(count).map_or(usize::MAX, |c| c.min(len / record))
}

/// Pick the strike nearest `wanted`, preferring one at least that large.
fn best_strike(candidates: impl Iterator<Item = (usize, u16)>, wanted: u16) -> Option<usize> {
    candidates
        .min_by_key(|&(_, ppem)| {
            // Larger-or-equal strikes sort before smaller ones.
            let below = ppem < wanted;
            (below, ppem.abs_diff(wanted))
        })
        .map(|(index, _)| index)
}

impl<'a> Face<'a> {
    /// The PNG bitmap for `glyph` from the strike closest to `pixels_per_em`
    /// (preferring a larger strike over a smaller one), from `sbix` or
    /// `CBLC`/`CBDT`. `None` for glyphs with no PNG bitmap, other bitmap
    /// formats, or malformed tables.
    pub fn glyph_raster_image(
        &self,
        glyph: GlyphId,
        pixels_per_em: u16,
    ) -> Option<RasterGlyphImage<'a>> {
        self.sbix_image(glyph, pixels_per_em)
            .or_else(|| self.cbdt_image(glyph, pixels_per_em))
    }

    fn sbix_image(&self, glyph: GlyphId, wanted: u16) -> Option<RasterGlyphImage<'a>> {
        let sbix = self.table_data(SBIX)?;
        let strikes = bounded(u32_at(sbix, 4)?, sbix.len(), 4);
        let strike_at = |i: usize| -> Option<&'a [u8]> {
            tail32(sbix, u32_at(sbix, 8usize.checked_add(i.checked_mul(4)?)?)?)
        };
        let ppems = (0..strikes).filter_map(|i| Some((i, u16_at(strike_at(i)?, 0)?)));
        let strike = strike_at(best_strike(ppems, wanted)?)?;
        let ppem = u16_at(strike, 0)?;

        let mut id = glyph.0;
        for _ in 0..2 {
            // One `dupe` hop at most.
            let slot = 4usize.checked_add(usize::from(id).checked_mul(4)?)?;
            let (start, end) = (u32_at(strike, slot)?, u32_at(strike, slot + 4)?);
            let record = strike.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)?;
            let kind = u32_at(record, 4)?;
            let payload = record.get(8..)?;
            if kind == DUPE {
                id = u16_at(payload, 0)?;
                continue;
            }
            if kind != PNG_ {
                return None;
            }
            return Some(RasterGlyphImage {
                x: i16_at(record, 0)?,
                y: i16_at(record, 2)?,
                width: 0,
                height: 0,
                pixels_per_em: ppem,
                format: RasterImageFormat::PNG,
                data: payload,
            });
        }
        None
    }

    fn cbdt_image(&self, glyph: GlyphId, wanted: u16) -> Option<RasterGlyphImage<'a>> {
        let (cblc, cbdt) = (self.table_data(CBLC)?, self.table_data(CBDT)?);
        let sizes = bounded(u32_at(cblc, 4)?, cblc.len(), 48);
        let size_at = |i: usize| 8usize.checked_add(i.checked_mul(48)?);
        let covers = |i: usize| -> Option<u16> {
            let at = size_at(i)?;
            let (first, last) = (u16_at(cblc, at + 40)?, u16_at(cblc, at + 42)?);
            (first <= glyph.0 && glyph.0 <= last).then_some(u16::from(u8_at(cblc, at + 45)?))
        };
        let strike = best_strike((0..sizes).filter_map(|i| Some((i, covers(i)?))), wanted)?;
        let size = size_at(strike)?;
        let ppem = u16::from(u8_at(cblc, size + 45)?);

        // Find the index subtable covering the glyph.
        let array = usize::try_from(u32_at(cblc, size)?).ok()?;
        let tables = bounded(u32_at(cblc, size + 8)?, cblc.len(), 8);
        let index_sub = (0..tables).find_map(|i| {
            let entry = array.checked_add(i.checked_mul(8)?)?;
            let (first, last) = (u16_at(cblc, entry)?, u16_at(cblc, entry + 2)?);
            if glyph.0 < first || glyph.0 > last {
                return None;
            }
            let at = array.checked_add(usize::try_from(u32_at(cblc, entry + 4)?).ok()?)?;
            Some((at, usize::from(glyph.0 - first)))
        })?;
        let (table, slot) = index_sub;

        let (index_format, image_format) = (u16_at(cblc, table)?, u16_at(cblc, table + 2)?);
        let image_base = usize::try_from(u32_at(cblc, table + 4)?).ok()?;
        let body = table.checked_add(8)?;
        let (start, end, shared_metrics) = match index_format {
            1 | 3 => {
                let width = if index_format == 1 { 4 } else { 2 };
                let read = |i: usize| -> Option<usize> {
                    let at = body.checked_add(i.checked_mul(width)?)?;
                    if width == 4 {
                        usize::try_from(u32_at(cblc, at)?).ok()
                    } else {
                        Some(usize::from(u16_at(cblc, at)?))
                    }
                };
                (read(slot)?, read(slot + 1)?, None)
            }
            2 => {
                let image_size = usize::try_from(u32_at(cblc, body)?).ok()?;
                let metrics = cblc.get(body + 4..body + 12)?;
                let start = slot.checked_mul(image_size)?;
                (start, start.checked_add(image_size)?, Some(metrics))
            }
            _ => return None,
        };
        if end <= start {
            return None;
        }
        let record = cbdt.get(image_base.checked_add(start)?..image_base.checked_add(end)?)?;

        // Image formats 17, 18 and 19 are the PNG ones.
        let (height, width, bearing_x, bearing_y, payload) = match image_format {
            17 => {
                let len = usize::try_from(u32_at(record, 5)?).ok()?;
                (
                    u8_at(record, 0)?,
                    u8_at(record, 1)?,
                    u8_at(record, 2)? as i8,
                    u8_at(record, 3)? as i8,
                    record.get(9..9usize.checked_add(len)?)?,
                )
            }
            18 => {
                let len = usize::try_from(u32_at(record, 8)?).ok()?;
                (
                    u8_at(record, 0)?,
                    u8_at(record, 1)?,
                    u8_at(record, 2)? as i8,
                    u8_at(record, 3)? as i8,
                    record.get(12..12usize.checked_add(len)?)?,
                )
            }
            19 => {
                let m = shared_metrics?;
                let len = usize::try_from(u32_at(record, 0)?).ok()?;
                (
                    u8_at(m, 0)?,
                    u8_at(m, 1)?,
                    u8_at(m, 2)? as i8,
                    u8_at(m, 3)? as i8,
                    record.get(4..4usize.checked_add(len)?)?,
                )
            }
            _ => return None,
        };
        Some(RasterGlyphImage {
            x: i16::from(bearing_x),
            y: i16::from(bearing_y) - i16::from(height),
            width: u16::from(width),
            height: u16::from(height),
            pixels_per_em: ppem,
            format: RasterImageFormat::PNG,
            data: payload,
        })
    }
}
