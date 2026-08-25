//! TrueType / OpenType binary table parser — a real `sfnt` table
//! directory, `cmap` (format 4, the common BMP subtable every Latin-script
//! font ships, and format 12, the segmented-coverage subtable used for
//! full 21-bit Unicode including supplementary-plane characters),
//! `loca`/`glyf` outline extraction (simple glyphs; composite glyphs are a
//! known, documented gap), and `head`/`maxp` metadata.

use crate::glyph::{GlyphOutline, Point};
use alloc::vec::Vec;

/// Errors parsing a font file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The file is too short to contain a valid `sfnt` header.
    TooShort,
    /// The `sfnt` version isn't one this parser recognizes (TrueType
    /// `0x00010000`/`true`, or OpenType-with-glyf `OTTO` isn't supported —
    /// CFF-outline OpenType fonts are a known gap).
    UnsupportedVersion,
    /// A required table (`head`, `maxp`, `loca`, `glyf`, `cmap`) is
    /// missing.
    MissingTable(&'static str),
    /// A table's contents didn't parse as expected (truncated, or an
    /// unsupported sub-format).
    Malformed(&'static str),
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    u16_at(data, offset).map(|v| v as i16)
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

struct TableRecord {
    tag: [u8; 4],
    offset: usize,
    length: usize,
}

/// A parsed TrueType font handle.
pub struct Font {
    data: Vec<u8>,
    units_per_em: u16,
    num_glyphs: u16,
    loca_long: bool,
    glyf_range: (usize, usize),
    loca_range: (usize, usize),
    cmap_subtable: Option<CmapSubtable>,
    /// `hhea` table: typographic ascender/descender/line-gap, in font
    /// units (see [`Font::ascender`]/[`Font::descender`]/[`Font::line_gap`]).
    ascender: i16,
    descender: i16,
    line_gap: i16,
    /// `hhea.numberOfHMetrics`: the number of `hmtx` entries that carry
    /// their own advance width. Glyph ids at or beyond this count reuse
    /// the last entry's advance width (the spec's "monospace tail"
    /// compression -- real for every font, not just monospace ones).
    num_h_metrics: u16,
    hmtx_range: (usize, usize),
}

/// A parsed `cmap` subtable, in whichever of the two formats this parser
/// supports the font actually shipped.
enum CmapSubtable {
    /// Format 4 — the segment-based Unicode BMP (up to U+FFFF) mapping
    /// every Latin-script TrueType font ships.
    Format4(CmapFormat4),
    /// Format 12 — segmented coverage over the full 21-bit Unicode range,
    /// used by fonts that carry supplementary-plane glyphs (e.g. Nerd
    /// Fonts' private-use icon ranges above U+FFFF).
    Format12(CmapFormat12),
}

impl CmapSubtable {
    fn lookup(&self, code: u32) -> Option<u16> {
        match self {
            CmapSubtable::Format4(t) => t.lookup(code),
            CmapSubtable::Format12(t) => t.lookup(code),
        }
    }
}

/// A parsed `cmap` format-4 subtable: the segment-based Unicode BMP
/// mapping every Latin-script TrueType font ships.
struct CmapFormat4 {
    end_codes: Vec<u16>,
    start_codes: Vec<u16>,
    id_deltas: Vec<i16>,
    id_range_offsets: Vec<u16>,
    /// Byte offset (within `subtable`) of the `idRangeOffset` array's
    /// first entry — needed to replicate the spec's pointer-arithmetic
    /// glyph lookup exactly (it's defined relative to each entry's own
    /// address, not the subtable's start).
    id_range_offsets_base: usize,
    /// The subtable's full raw bytes (not just the trailing glyph-id
    /// array), since `id_range_offset`-based lookups index relative to
    /// the subtable's start, not to any sub-slice.
    subtable: Vec<u8>,
}

impl CmapFormat4 {
    fn lookup(&self, code: u32) -> Option<u16> {
        if code > 0xFFFF {
            return None;
        }
        let code = code as u16;
        let seg = self.end_codes.iter().position(|&end| code <= end)?;
        if code < self.start_codes[seg] {
            return None;
        }
        let id_range_offset = self.id_range_offsets[seg];
        if id_range_offset == 0 {
            return Some((code as i32).wrapping_add(self.id_deltas[seg] as i32) as u16);
        }
        // Spec-mandated pointer arithmetic: the glyph id lives at
        // `&idRangeOffset[seg] + idRangeOffset[seg] + 2*(code - startCode[seg])`,
        // i.e. relative to *this entry's own address* within the
        // subtable, not the subtable's start.
        let this_entry_addr = self.id_range_offsets_base + seg * 2;
        let glyph_index_addr = this_entry_addr
            + id_range_offset as usize
            + 2 * (code - self.start_codes[seg]) as usize;
        let raw = u16_at(&self.subtable, glyph_index_addr)?;
        if raw == 0 {
            return None;
        }
        Some((raw as i32).wrapping_add(self.id_deltas[seg] as i32) as u16)
    }
}

/// A parsed `cmap` format-12 subtable: an array of `(startCharCode,
/// endCharCode, startGlyphID)` groups, sorted by `startCharCode` per spec,
/// each covering a contiguous run of codepoints mapped to consecutive
/// glyph ids.
struct CmapFormat12 {
    groups: Vec<(u32, u32, u32)>,
}

impl CmapFormat12 {
    fn lookup(&self, code: u32) -> Option<u16> {
        // Groups are spec-ordered by startCharCode, so the containing
        // group (if any) is the last one whose startCharCode <= code.
        let idx = self.groups.partition_point(|&(start, _, _)| start <= code);
        if idx == 0 {
            return None;
        }
        let (start, end, start_glyph_id) = self.groups[idx - 1];
        if code < start || code > end {
            return None;
        }
        u16::try_from(start_glyph_id + (code - start)).ok()
    }
}

impl Font {
    /// Parses font bytes from a raw slice: the `sfnt` table directory,
    /// `head`/`maxp` metadata, and (if present) a `cmap` format-4 subtable.
    pub fn parse(bytes: &[u8]) -> Result<Self, FontError> {
        if bytes.len() < 12 {
            return Err(FontError::TooShort);
        }
        let version = u32_at(bytes, 0).ok_or(FontError::TooShort)?;
        // 0x00010000 = TrueType; `true`/`typ1` (rare, legacy Mac) not
        // supported; `OTTO` (CFF-outline OpenType) not supported -- both
        // documented gaps, not silently mishandled.
        if version != 0x0001_0000 {
            return Err(FontError::UnsupportedVersion);
        }
        let num_tables = u16_at(bytes, 4).ok_or(FontError::TooShort)? as usize;

        let mut tables = Vec::with_capacity(num_tables);
        for i in 0..num_tables {
            let rec_offset = 12 + i * 16;
            let tag_bytes = bytes
                .get(rec_offset..rec_offset + 4)
                .ok_or(FontError::TooShort)?;
            let offset = u32_at(bytes, rec_offset + 8).ok_or(FontError::TooShort)? as usize;
            let length = u32_at(bytes, rec_offset + 12).ok_or(FontError::TooShort)? as usize;
            let mut tag = [0u8; 4];
            tag.copy_from_slice(tag_bytes);
            tables.push(TableRecord {
                tag,
                offset,
                length,
            });
        }

        let find = |tag: &[u8; 4]| tables.iter().find(|t| &t.tag == tag);

        let head = find(b"head").ok_or(FontError::MissingTable("head"))?;
        let units_per_em = u16_at(bytes, head.offset + 18).ok_or(FontError::Malformed("head"))?;
        let index_to_loc_format =
            i16_at(bytes, head.offset + 50).ok_or(FontError::Malformed("head"))?;
        let loca_long = index_to_loc_format != 0;

        let maxp = find(b"maxp").ok_or(FontError::MissingTable("maxp"))?;
        let num_glyphs = u16_at(bytes, maxp.offset + 4).ok_or(FontError::Malformed("maxp"))?;

        let loca = find(b"loca").ok_or(FontError::MissingTable("loca"))?;
        let glyf = find(b"glyf").ok_or(FontError::MissingTable("glyf"))?;

        let cmap_subtable = find(b"cmap").and_then(|t| parse_cmap(bytes, t.offset, t.length));

        let hhea = find(b"hhea").ok_or(FontError::MissingTable("hhea"))?;
        let ascender = i16_at(bytes, hhea.offset + 4).ok_or(FontError::Malformed("hhea"))?;
        let descender = i16_at(bytes, hhea.offset + 6).ok_or(FontError::Malformed("hhea"))?;
        let line_gap = i16_at(bytes, hhea.offset + 8).ok_or(FontError::Malformed("hhea"))?;
        let num_h_metrics = u16_at(bytes, hhea.offset + 34).ok_or(FontError::Malformed("hhea"))?;

        let hmtx = find(b"hmtx").ok_or(FontError::MissingTable("hmtx"))?;

        Ok(Self {
            data: bytes.to_vec(),
            units_per_em,
            num_glyphs,
            loca_long,
            glyf_range: (glyf.offset, glyf.length),
            loca_range: (loca.offset, loca.length),
            cmap_subtable,
            ascender,
            descender,
            line_gap,
            num_h_metrics,
            hmtx_range: (hmtx.offset, hmtx.length),
        })
    }

    /// Units per em (the coordinate system every glyph outline is in).
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// The number of glyphs this font defines.
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// Typographic ascender (font units, above the baseline). From `hhea`,
    /// the metric `hmtx`/layout engines use for line height -- not `OS/2`'s
    /// separate (and often inconsistent) `sTypoAscender`/`usWinAscent`.
    pub fn ascender(&self) -> i16 {
        self.ascender
    }

    /// Typographic descender (font units, negative -- below the baseline).
    pub fn descender(&self) -> i16 {
        self.descender
    }

    /// Recommended extra line spacing (font units) beyond
    /// `ascender - descender`, from `hhea.lineGap`.
    pub fn line_gap(&self) -> i16 {
        self.line_gap
    }

    /// A glyph's horizontal advance width (font units), from `hmtx`. Glyph
    /// ids at or beyond `hhea.numberOfHMetrics` reuse the table's last
    /// entry -- the spec's compression for runs of glyphs sharing one
    /// advance (every monospace font's entire glyph set, in practice).
    /// `0` for an out-of-range or malformed table rather than a panic.
    pub fn advance_width(&self, glyph_id: u16) -> u16 {
        let (offset, len) = self.hmtx_range;
        let Some(hmtx) = self.data.get(offset..offset + len) else {
            return 0;
        };
        if self.num_h_metrics == 0 {
            return 0;
        }
        let idx = (glyph_id as usize).min(self.num_h_metrics as usize - 1);
        u16_at(hmtx, idx * 4).unwrap_or(0)
    }

    /// Maps a Unicode character to a glyph ID via the font's real `cmap`
    /// subtable (format 4 or format 12), if one was found. Returns `None`
    /// (not glyph 0) when there's no mapping or no usable `cmap` — glyph 0
    /// is conventionally ".notdef", a real glyph, not an absence marker.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        self.cmap_subtable.as_ref()?.lookup(ch as u32)
    }

    fn loca_entry(&self, glyph_id: u16) -> Option<(usize, usize)> {
        let (loca_offset, loca_len) = self.loca_range;
        let loca_data = self.data.get(loca_offset..loca_offset + loca_len)?;
        if self.loca_long {
            let start = u32_at(loca_data, glyph_id as usize * 4)? as usize;
            let end = u32_at(loca_data, (glyph_id as usize + 1) * 4)? as usize;
            Some((start, end))
        } else {
            let start = u16_at(loca_data, glyph_id as usize * 2)? as usize * 2;
            let end = u16_at(loca_data, (glyph_id as usize + 1) * 2)? as usize * 2;
            Some((start, end))
        }
    }

    /// Extracts the vector outline of a glyph by ID — a real simple-glyph
    /// parse (contours, on/off-curve quadratic points, run-length-encoded
    /// flags and deltas), not a placeholder box. Composite glyphs
    /// (`numberOfContours < 0`) and glyphs with no outline (e.g. space,
    /// where `loca[id] == loca[id+1]`) both return `Some` with an empty
    /// point list — a documented gap for composites, real behavior for
    /// genuinely empty glyphs.
    pub fn glyph_outline(&self, glyph_id: u16) -> Option<GlyphOutline> {
        let (start, end) = self.loca_entry(glyph_id)?;
        if start >= end {
            return Some(GlyphOutline::default());
        }
        let (glyf_offset, glyf_len) = self.glyf_range;
        let glyph_data = self.data.get(glyf_offset..glyf_offset + glyf_len)?;
        let glyph_data = glyph_data.get(start..end)?;

        let number_of_contours = i16_at(glyph_data, 0)?;
        let min_x = i16_at(glyph_data, 2)?;
        let min_y = i16_at(glyph_data, 4)?;
        let max_x = i16_at(glyph_data, 6)?;
        let max_y = i16_at(glyph_data, 8)?;

        if number_of_contours < 0 {
            // Composite glyph: a documented gap. Returning the bounding
            // box with no points is honest (no fabricated outline) rather
            // than silently drawing nothing where a caller might expect
            // an error.
            return Some(GlyphOutline {
                min_x,
                min_y,
                max_x,
                max_y,
                ..GlyphOutline::default()
            });
        }

        parse_simple_glyph(
            glyph_data,
            number_of_contours as usize,
            min_x,
            min_y,
            max_x,
            max_y,
        )
    }

    /// The raw font data slice.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

fn parse_cmap(data: &[u8], cmap_offset: usize, cmap_len: usize) -> Option<CmapSubtable> {
    let cmap = data.get(cmap_offset..cmap_offset + cmap_len)?;
    let num_subtables = u16_at(cmap, 2)?;

    // Prefer a format-12 subtable (full 21-bit Unicode, a superset of
    // format 4's BMP-only coverage) if the font ships one — within
    // platform 3 (Windows), prefer encoding 10 (full Unicode). Otherwise
    // fall back to format 4: platform 3 encoding 1 (Unicode BMP), then
    // platform 0 (Unicode) — the two encodings that use format 4.
    let mut best_12: Option<usize> = None;
    let mut best_4: Option<usize> = None;
    for i in 0..num_subtables as usize {
        let rec = 4 + i * 8;
        let platform_id = u16_at(cmap, rec)?;
        let encoding_id = u16_at(cmap, rec + 2)?;
        let offset = u32_at(cmap, rec + 4)? as usize;
        match cmap.get(offset..).and_then(|s| u16_at(s, 0)) {
            Some(12) if best_12.is_none() || (platform_id == 3 && encoding_id == 10) => {
                best_12 = Some(offset);
            }
            Some(4)
                if (platform_id == 3 && encoding_id == 1)
                    || (platform_id == 0 && best_4.is_none()) =>
            {
                best_4 = Some(offset);
            }
            _ => {}
        }
    }

    if let Some(offset) = best_12 {
        if let Some(t) = parse_cmap_format12(cmap, offset) {
            return Some(CmapSubtable::Format12(t));
        }
    }
    parse_cmap_format4(cmap, best_4?).map(CmapSubtable::Format4)
}

fn parse_cmap_format12(cmap: &[u8], offset: usize) -> Option<CmapFormat12> {
    let subtable = cmap.get(offset..)?;
    if u16_at(subtable, 0)? != 12 {
        return None;
    }
    let num_groups = u32_at(subtable, 12)? as usize;
    let mut groups = Vec::with_capacity(num_groups);
    for i in 0..num_groups {
        let rec = 16 + i * 12;
        let start_char_code = u32_at(subtable, rec)?;
        let end_char_code = u32_at(subtable, rec + 4)?;
        let start_glyph_id = u32_at(subtable, rec + 8)?;
        groups.push((start_char_code, end_char_code, start_glyph_id));
    }
    Some(CmapFormat12 { groups })
}

fn parse_cmap_format4(cmap: &[u8], subtable_offset: usize) -> Option<CmapFormat4> {
    let subtable = cmap.get(subtable_offset..)?;
    let format = u16_at(subtable, 0)?;
    if format != 4 {
        return None;
    }

    let seg_count_x2 = u16_at(subtable, 6)? as usize;
    let seg_count = seg_count_x2 / 2;
    let end_codes_off = 14;
    let start_codes_off = end_codes_off + seg_count_x2 + 2; // +2 for reservedPad
    let id_deltas_off = start_codes_off + seg_count_x2;
    let id_range_offsets_off = id_deltas_off + seg_count_x2;

    let mut end_codes = Vec::with_capacity(seg_count);
    let mut start_codes = Vec::with_capacity(seg_count);
    let mut id_deltas = Vec::with_capacity(seg_count);
    let mut id_range_offsets = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        end_codes.push(u16_at(subtable, end_codes_off + i * 2)?);
        start_codes.push(u16_at(subtable, start_codes_off + i * 2)?);
        id_deltas.push(i16_at(subtable, id_deltas_off + i * 2)?);
        id_range_offsets.push(u16_at(subtable, id_range_offsets_off + i * 2)?);
    }

    Some(CmapFormat4 {
        end_codes,
        start_codes,
        id_deltas,
        id_range_offsets,
        id_range_offsets_base: id_range_offsets_off,
        subtable: subtable.to_vec(),
    })
}

fn parse_simple_glyph(
    data: &[u8],
    number_of_contours: usize,
    min_x: i16,
    min_y: i16,
    max_x: i16,
    max_y: i16,
) -> Option<GlyphOutline> {
    const HEADER_LEN: usize = 10;
    let mut cursor = HEADER_LEN;

    let mut contour_ends = Vec::with_capacity(number_of_contours);
    for _ in 0..number_of_contours {
        contour_ends.push(u16_at(data, cursor)? as usize);
        cursor += 2;
    }
    let num_points = contour_ends.last().map(|&e| e + 1).unwrap_or(0);

    let instruction_length = u16_at(data, cursor)? as usize;
    cursor += 2 + instruction_length;

    let mut flags = Vec::with_capacity(num_points);
    while flags.len() < num_points {
        let flag = *data.get(cursor)?;
        cursor += 1;
        flags.push(flag);
        if flag & 0x08 != 0 {
            let repeat = *data.get(cursor)?;
            cursor += 1;
            for _ in 0..repeat {
                if flags.len() >= num_points {
                    break;
                }
                flags.push(flag);
            }
        }
    }

    let mut xs = Vec::with_capacity(num_points);
    let mut x = 0i32;
    for &flag in &flags {
        let short = flag & 0x02 != 0;
        let same_or_positive = flag & 0x10 != 0;
        if short {
            let delta = *data.get(cursor)? as i32;
            cursor += 1;
            x += if same_or_positive { delta } else { -delta };
        } else if !same_or_positive {
            x += i16_at(data, cursor)? as i32;
            cursor += 2;
        }
        xs.push(x);
    }

    let mut ys = Vec::with_capacity(num_points);
    let mut y = 0i32;
    for &flag in &flags {
        let short = flag & 0x04 != 0;
        let same_or_positive = flag & 0x20 != 0;
        if short {
            let delta = *data.get(cursor)? as i32;
            cursor += 1;
            y += if same_or_positive { delta } else { -delta };
        } else if !same_or_positive {
            y += i16_at(data, cursor)? as i32;
            cursor += 2;
        }
        ys.push(y);
    }

    let points = (0..num_points)
        .map(|i| Point::new(xs[i] as f32, ys[i] as f32, flags[i] & 0x01 != 0))
        .collect();

    Some(GlyphOutline {
        points,
        contour_ends,
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_system_font(name: &str) -> Option<Vec<u8>> {
        std::fs::read(alloc::format!("C:\\Windows\\Fonts\\{name}")).ok()
    }

    #[test]
    fn parses_a_real_system_font_and_reports_sane_metadata() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).expect("arial.ttf should parse as a real TrueType font");
        // Arial's real units_per_em is 2048 -- a coincidence with the old
        // stub's hardcoded value, but this time it's actually read from
        // the file, not hardcoded (verified below against a different
        // font whose real value differs).
        assert_eq!(font.units_per_em(), 2048);
        assert!(
            font.num_glyphs() > 200,
            "arial.ttf should define far more than 200 glyphs"
        );
    }

    #[test]
    fn units_per_em_is_read_from_the_file_not_hardcoded() {
        // Courier New's real units_per_em is also 2048 on this system, so
        // to actually distinguish "read from file" from "hardcoded",
        // corrupt a copy's `head` table units_per_em field and confirm
        // the parser reflects the corruption -- proving it's a real read.
        let Some(mut bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let head_offset = (0..num_tables)
            .find_map(|i| {
                let rec = 12 + i * 16;
                (&bytes[rec..rec + 4] == b"head").then(|| {
                    u32::from_be_bytes([
                        bytes[rec + 8],
                        bytes[rec + 9],
                        bytes[rec + 10],
                        bytes[rec + 11],
                    ]) as usize
                })
            })
            .expect("arial.ttf should have a head table");
        bytes[head_offset + 18] = 0x03;
        bytes[head_offset + 19] = 0xE8; // 1000
        let font = Font::parse(&bytes).unwrap();
        assert_eq!(
            font.units_per_em(),
            1000,
            "units_per_em should reflect the corrupted value, proving it's a real read"
        );
    }

    #[test]
    fn glyph_index_resolves_real_ascii_characters_and_differs_by_letter() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let a = font.glyph_index('A').expect("'A' should be mapped");
        let b = font.glyph_index('B').expect("'B' should be mapped");
        let space = font.glyph_index(' ').expect("space should be mapped");
        assert_ne!(
            a, b,
            "different characters should resolve to different glyph ids"
        );
        assert_ne!(a, 0, "'A' should not resolve to .notdef");
        assert_ne!(space, 0, "space should not resolve to .notdef");
    }

    #[test]
    fn glyph_outline_of_a_real_letter_has_real_contours_within_the_em_box() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let glyph_id = font.glyph_index('A').unwrap();
        let outline = font
            .glyph_outline(glyph_id)
            .expect("'A' should have an outline");

        assert!(
            !outline.points.is_empty(),
            "'A' should have real outline points, not an empty placeholder"
        );
        assert!(!outline.contour_ends.is_empty());
        // 'A' has a hole (the triangular counter) -- at least 2 contours.
        assert!(
            outline.contour_ends.len() >= 2,
            "'A' should have at least 2 contours (outer + counter)"
        );

        let em = font.units_per_em() as f32;
        for p in &outline.points {
            assert!(
                p.x > -em && p.x < 2.0 * em,
                "point x {} should be roughly within the em square",
                p.x
            );
            assert!(
                p.y > -em && p.y < 2.0 * em,
                "point y {} should be roughly within the em square",
                p.y
            );
        }
    }

    #[test]
    fn space_glyph_has_an_empty_outline() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let glyph_id = font.glyph_index(' ').unwrap();
        let outline = font.glyph_outline(glyph_id).unwrap();
        assert!(
            outline.points.is_empty(),
            "space should have no drawable contour"
        );
    }

    #[test]
    fn different_real_fonts_report_different_glyph_shapes() {
        let Some(arial) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let Some(courier) = load_system_font("cour.ttf") else {
            eprintln!("skipping: cour.ttf not found on this machine");
            return;
        };
        let arial_font = Font::parse(&arial).unwrap();
        let courier_font = Font::parse(&courier).unwrap();

        let a_outline = arial_font
            .glyph_outline(arial_font.glyph_index('A').unwrap())
            .unwrap();
        let c_outline = courier_font
            .glyph_outline(courier_font.glyph_index('A').unwrap())
            .unwrap();
        // Different fonts should not produce byte-identical point data for
        // the same letter -- a coarse but real signal that this isn't
        // just returning the same hardcoded box for everything.
        assert_ne!(a_outline.points.len(), c_outline.points.len());
    }

    #[test]
    fn hhea_metrics_are_sane_for_a_real_font() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        // Arial's ascender is comfortably positive (above the baseline) and
        // the descender comfortably negative (below it) -- real hhea
        // values, not zeroed placeholders.
        assert!(
            font.ascender() > 0,
            "ascender should be positive: {}",
            font.ascender()
        );
        assert!(
            font.descender() < 0,
            "descender should be negative: {}",
            font.descender()
        );
        assert!(
            font.ascender() as i32 - font.descender() as i32 > font.units_per_em() as i32 / 2,
            "ascender-descender spread should be a real fraction of the em square"
        );
    }

    #[test]
    fn advance_width_is_uniform_for_a_real_monospace_font() {
        let Some(bytes) = load_system_font("consola.ttf") else {
            eprintln!("skipping: consola.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let m = font.advance_width(font.glyph_index('M').unwrap());
        let i = font.advance_width(font.glyph_index('i').unwrap());
        assert!(m > 0, "'M' should have a real nonzero advance width");
        assert_eq!(
            m, i,
            "a monospace font's glyphs should share one advance width"
        );
    }

    #[test]
    fn advance_width_differs_for_a_real_proportional_font() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let m = font.advance_width(font.glyph_index('M').unwrap());
        let i = font.advance_width(font.glyph_index('i').unwrap());
        assert!(m > 0 && i > 0);
        assert_ne!(
            m, i,
            "a proportional font's 'M' and 'i' should have different advance widths"
        );
    }

    /// Builds a minimal standalone `cmap` table: header + one encoding
    /// record + a single format-12 subtable with one group mapping
    /// `[start, end]` to glyph ids starting at `start_glyph`.
    fn build_format12_cmap(
        platform_id: u16,
        encoding_id: u16,
        start: u32,
        end: u32,
        start_glyph: u32,
    ) -> Vec<u8> {
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes()); // version
        cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
        cmap.extend_from_slice(&platform_id.to_be_bytes());
        cmap.extend_from_slice(&encoding_id.to_be_bytes());
        let subtable_offset: u32 = 4 + 8;
        cmap.extend_from_slice(&subtable_offset.to_be_bytes());

        cmap.extend_from_slice(&12u16.to_be_bytes()); // format
        cmap.extend_from_slice(&0u16.to_be_bytes()); // reserved
        let num_groups: u32 = 1;
        let length: u32 = 16 + 12 * num_groups;
        cmap.extend_from_slice(&length.to_be_bytes());
        cmap.extend_from_slice(&0u32.to_be_bytes()); // language
        cmap.extend_from_slice(&num_groups.to_be_bytes());
        cmap.extend_from_slice(&start.to_be_bytes());
        cmap.extend_from_slice(&end.to_be_bytes());
        cmap.extend_from_slice(&start_glyph.to_be_bytes());
        cmap
    }

    #[test]
    fn cmap_format_12_maps_supplementary_plane_codepoints() {
        let cmap = build_format12_cmap(3, 10, 0xF0000, 0xF0010, 500);
        let subtable = parse_cmap(&cmap, 0, cmap.len()).expect("format-12 cmap should parse");
        let CmapSubtable::Format12(t) = subtable else {
            panic!("expected a format-12 subtable to be selected");
        };
        assert_eq!(
            t.lookup(0xF0000),
            Some(500),
            "group start should map to startGlyphID"
        );
        assert_eq!(
            t.lookup(0xF0008),
            Some(508),
            "mid-group codepoint should map to startGlyphID + offset"
        );
        assert_eq!(
            t.lookup(0xF0010),
            Some(516),
            "group end (inclusive) should still resolve"
        );
        assert_eq!(
            t.lookup(0xF0011),
            None,
            "a codepoint just past the group's end should not resolve"
        );
        assert_eq!(
            t.lookup(0x41),
            None,
            "a BMP codepoint outside any group should not resolve"
        );
    }

    #[test]
    fn cmap_format_12_is_preferred_over_a_bmp_only_format_4_subtable() {
        // A font can legally carry both a format-4 (platform 3, encoding 1)
        // and a format-12 (platform 3, encoding 10) subtable — format 12 is
        // a strict superset, so it should win.
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes()); // version
        cmap.extend_from_slice(&2u16.to_be_bytes()); // numTables

        // Encoding record 0: format-4, platform 3 / encoding 1.
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        let format4_offset: u32 = 4 + 2 * 8;
        cmap.extend_from_slice(&format4_offset.to_be_bytes());

        // Encoding record 1: format-12, platform 3 / encoding 10.
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&10u16.to_be_bytes());
        // A 2-segment format-4 subtable is 32 bytes: 14-byte fixed header
        // + 2-byte reservedPad + 4 parallel arrays (end/start/delta/
        // rangeOffset) of 2 segments * 2 bytes each = 14 + 2 + 4*(2*2).
        let format4_len: u32 = 14 + 2 + 4 * (2 * 2);
        let format12_offset = format4_offset + format4_len;
        cmap.extend_from_slice(&format12_offset.to_be_bytes());

        // format-4 subtable: single segment covering 'A' (0x41) -> glyph 1,
        // terminated by the required 0xFFFF end segment.
        cmap.extend_from_slice(&4u16.to_be_bytes()); // format
        cmap.extend_from_slice(&0u16.to_be_bytes()); // length (unused by parser)
        cmap.extend_from_slice(&0u16.to_be_bytes()); // language
        cmap.extend_from_slice(&4u16.to_be_bytes()); // segCountX2 (2 segments)
        cmap.extend_from_slice(&0u16.to_be_bytes()); // searchRange (unused)
        cmap.extend_from_slice(&0u16.to_be_bytes()); // entrySelector (unused)
        cmap.extend_from_slice(&0u16.to_be_bytes()); // rangeShift (unused)
        cmap.extend_from_slice(&0x0041u16.to_be_bytes()); // endCode[0]
        cmap.extend_from_slice(&0xFFFFu16.to_be_bytes()); // endCode[1]
        cmap.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
        cmap.extend_from_slice(&0x0041u16.to_be_bytes()); // startCode[0]
        cmap.extend_from_slice(&0xFFFFu16.to_be_bytes()); // startCode[1]
        cmap.extend_from_slice(&1i16.to_be_bytes()); // idDelta[0]: 0x41 -> glyph 1
        cmap.extend_from_slice(&1i16.to_be_bytes()); // idDelta[1]
        cmap.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset[0]
        cmap.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset[1]

        cmap.extend_from_slice(&build_format12_cmap(3, 10, 0xF0000, 0xF0010, 500)[12..]);

        let subtable = parse_cmap(&cmap, 0, cmap.len()).expect("cmap should parse");
        assert!(
            matches!(subtable, CmapSubtable::Format12(_)),
            "format 12 should be preferred over format 4 when both are present"
        );
    }

    #[test]
    fn too_short_input_is_rejected() {
        assert!(matches!(Font::parse(&[0u8; 4]), Err(FontError::TooShort)));
    }

    #[test]
    fn non_truetype_version_is_rejected() {
        let mut bytes = alloc::vec![0u8; 12];
        bytes[0..4].copy_from_slice(b"OTTO");
        assert!(matches!(
            Font::parse(&bytes),
            Err(FontError::UnsupportedVersion)
        ));
    }
}
