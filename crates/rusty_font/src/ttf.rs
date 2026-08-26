//! TrueType / OpenType binary table parser — a real `sfnt` table
//! directory, `cmap` (format 4, the common BMP subtable every Latin-script
//! font ships, and format 12, the segmented-coverage subtable used for
//! full 21-bit Unicode including supplementary-plane characters), glyph
//! outline extraction (TrueType `loca`/`glyf` — simple glyphs, and
//! composite glyphs assembled from their component records — or CFF-flavor
//! OpenType's `CFF ` table via [`crate::cff`]'s Type 2 charstring
//! interpreter), and `head`/`maxp` metadata.

use crate::cff::{self, CffTable};
use crate::glyph::{GlyphOutline, Point};
use alloc::vec::Vec;

/// Errors parsing a font file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The file is too short to contain a valid `sfnt` header.
    TooShort,
    /// The `sfnt` version isn't one this parser recognizes: TrueType
    /// (`0x00010000`) and CFF-outline OpenType (`OTTO`) are both
    /// supported; legacy Mac `true`/`typ1` are not.
    UnsupportedVersion,
    /// A required table (`head`, `maxp`, `cmap`, `hhea`, `hmtx`, and
    /// either `loca`+`glyf` or `CFF `) is missing.
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

/// Reads an `F2Dot14` fixed-point value (2 integer bits, 14 fraction
/// bits) — the encoding a composite glyph's component transform uses.
fn f2dot14_at(data: &[u8], offset: usize) -> Option<f32> {
    i16_at(data, offset).map(|v| v as f32 / 16384.0)
}

struct TableRecord {
    tag: [u8; 4],
    offset: usize,
    length: usize,
}

/// Where a `Font`'s glyph outlines come from — the two mutually exclusive
/// shapes an `sfnt` container's outline data can take.
enum OutlineSource {
    /// TrueType `loca`/`glyf`.
    TrueType {
        loca_long: bool,
        glyf_range: (usize, usize),
        loca_range: (usize, usize),
    },
    /// CFF-flavor OpenType (`OTTO`)'s `CFF ` table.
    Cff(CffTable),
}

/// A parsed TrueType or CFF-flavor OpenType font handle.
pub struct Font {
    data: Vec<u8>,
    units_per_em: u16,
    num_glyphs: u16,
    outline_source: OutlineSource,
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

/// `sfnt` version tag for CFF-flavor OpenType (`OTTO`, big-endian ASCII).
const OTTO_VERSION: u32 = 0x4F54_544F;

impl Font {
    /// Parses font bytes from a raw slice: the `sfnt` table directory,
    /// `head`/`maxp` metadata, a `cmap` subtable (format 4 or 12) if
    /// present, and the glyph outline source -- TrueType `loca`/`glyf`, or
    /// (for an `OTTO`-tagged font) a CFF table.
    pub fn parse(bytes: &[u8]) -> Result<Self, FontError> {
        if bytes.len() < 12 {
            return Err(FontError::TooShort);
        }
        let version = u32_at(bytes, 0).ok_or(FontError::TooShort)?;
        // 0x00010000 = TrueType; OTTO = CFF-outline OpenType; `true`/`typ1`
        // (rare, legacy Mac) are not supported -- a documented gap, not
        // silently mishandled.
        if version != 0x0001_0000 && version != OTTO_VERSION {
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

        let outline_source = if version == OTTO_VERSION {
            let cff_table = find(b"CFF ").ok_or(FontError::MissingTable("CFF "))?;
            let cff = cff::parse_cff_table(bytes, cff_table.offset, cff_table.length)
                .ok_or(FontError::Malformed("CFF "))?;
            OutlineSource::Cff(cff)
        } else {
            let loca = find(b"loca").ok_or(FontError::MissingTable("loca"))?;
            let glyf = find(b"glyf").ok_or(FontError::MissingTable("glyf"))?;
            OutlineSource::TrueType {
                loca_long,
                glyf_range: (glyf.offset, glyf.length),
                loca_range: (loca.offset, loca.length),
            }
        };

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
            outline_source,
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
        let OutlineSource::TrueType {
            loca_long,
            loca_range: (loca_offset, loca_len),
            ..
        } = &self.outline_source
        else {
            return None;
        };
        let loca_data = self.data.get(*loca_offset..*loca_offset + *loca_len)?;
        if *loca_long {
            let start = u32_at(loca_data, glyph_id as usize * 4)? as usize;
            let end = u32_at(loca_data, (glyph_id as usize + 1) * 4)? as usize;
            Some((start, end))
        } else {
            let start = u16_at(loca_data, glyph_id as usize * 2)? as usize * 2;
            let end = u16_at(loca_data, (glyph_id as usize + 1) * 2)? as usize * 2;
            Some((start, end))
        }
    }

    /// Composite glyphs can in principle reference each other to an
    /// unbounded (or, in a malformed font, cyclic) depth; this caps how
    /// far [`Font::glyph_outline`] will recurse before giving up on
    /// further nesting and falling back to that glyph's bounding box —
    /// well beyond the 1-2 levels a real accented-Latin composite uses.
    const MAX_COMPOSITE_DEPTH: u32 = 8;

    /// Extracts the vector outline of a glyph by ID.
    ///
    /// For a TrueType font: a real simple-glyph parse (contours, on/off-curve
    /// quadratic points, run-length-encoded flags and deltas) or, for a
    /// composite glyph (`numberOfContours < 0`), the real assembled
    /// outline: each component's referenced glyph resolved recursively and
    /// its points transformed (2x2 matrix, or scale, plus an offset) into
    /// the composite's coordinate space. A component using point-matching
    /// (`ARGS_ARE_XY_VALUES` unset, rare in practice) is a documented
    /// remaining gap — that one component is skipped rather than
    /// fabricating a wrong position. Glyphs with no outline (e.g. space,
    /// where `loca[id] == loca[id+1]`) return `Some` with an empty point
    /// list — real behavior, not a placeholder.
    ///
    /// For a CFF-flavor OpenType font: the glyph's Type 2 charstring,
    /// interpreted and flattened into on-curve line segments (see
    /// [`crate::cff`] — CFF's cubic Bézier curves aren't representable in
    /// this crate's TrueType-shaped on/off-curve quadratic point model, so
    /// they're approximated rather than preserved exactly).
    pub fn glyph_outline(&self, glyph_id: u16) -> Option<GlyphOutline> {
        match &self.outline_source {
            OutlineSource::Cff(table) => cff::glyph_outline(&self.data, table, glyph_id),
            OutlineSource::TrueType { .. } => self.glyph_outline_at_depth(glyph_id, 0),
        }
    }

    fn glyph_outline_at_depth(&self, glyph_id: u16, depth: u32) -> Option<GlyphOutline> {
        let (start, end) = self.loca_entry(glyph_id)?;
        if start >= end {
            return Some(GlyphOutline::default());
        }
        let OutlineSource::TrueType { glyf_range, .. } = &self.outline_source else {
            return None;
        };
        let (glyf_offset, glyf_len) = *glyf_range;
        let glyph_data = self.data.get(glyf_offset..glyf_offset + glyf_len)?;
        let glyph_data = glyph_data.get(start..end)?;

        let number_of_contours = i16_at(glyph_data, 0)?;
        let min_x = i16_at(glyph_data, 2)?;
        let min_y = i16_at(glyph_data, 4)?;
        let max_x = i16_at(glyph_data, 6)?;
        let max_y = i16_at(glyph_data, 8)?;

        if number_of_contours < 0 {
            let bbox_only = || {
                Some(GlyphOutline {
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                    ..GlyphOutline::default()
                })
            };
            if depth >= Self::MAX_COMPOSITE_DEPTH {
                return bbox_only();
            }
            // Fall back to the bounding-box-only placeholder if the
            // component records themselves are malformed -- the glyph
            // still exists (unlike an out-of-range id, which is a real
            // `None`), so degrading gracefully is more honest than
            // failing the whole lookup.
            return self
                .parse_composite_glyph(glyph_data, min_x, min_y, max_x, max_y, depth)
                .or_else(bbox_only);
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

    /// Parses a composite glyph's component records (per the `glyf` spec:
    /// flags, referenced glyph index, x/y offset or point-matching
    /// indices, and an optional scale/2x2 transform) and concatenates each
    /// referenced glyph's transformed outline. `contour_ends` is offset by
    /// the running point count as components are appended, same as any
    /// outline concatenation.
    fn parse_composite_glyph(
        &self,
        glyph_data: &[u8],
        min_x: i16,
        min_y: i16,
        max_x: i16,
        max_y: i16,
        depth: u32,
    ) -> Option<GlyphOutline> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let mut points = Vec::new();
        let mut contour_ends = Vec::new();
        let mut cursor = 10usize; // past the 10-byte glyph header

        loop {
            let flags = u16_at(glyph_data, cursor)?;
            let component_glyph_id = u16_at(glyph_data, cursor + 2)?;
            cursor += 4;

            let (dx, dy) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                let a1 = i16_at(glyph_data, cursor)?;
                let a2 = i16_at(glyph_data, cursor + 2)?;
                cursor += 4;
                (a1, a2)
            } else {
                let a1 = *glyph_data.get(cursor)? as i8 as i16;
                let a2 = *glyph_data.get(cursor + 1)? as i8 as i16;
                cursor += 2;
                (a1, a2)
            };

            let (a, b, c, d) = if flags & WE_HAVE_A_SCALE != 0 {
                let s = f2dot14_at(glyph_data, cursor)?;
                cursor += 2;
                (s, 0.0, 0.0, s)
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                let sx = f2dot14_at(glyph_data, cursor)?;
                let sy = f2dot14_at(glyph_data, cursor + 2)?;
                cursor += 4;
                (sx, 0.0, 0.0, sy)
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                let a = f2dot14_at(glyph_data, cursor)?;
                let b = f2dot14_at(glyph_data, cursor + 2)?;
                let c = f2dot14_at(glyph_data, cursor + 4)?;
                let d = f2dot14_at(glyph_data, cursor + 6)?;
                cursor += 8;
                (a, b, c, d)
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };

            // Point-matching (ARGS_ARE_XY_VALUES unset) isn't supported --
            // skip assembling this one component rather than fabricating
            // a wrong position. The cursor has already advanced past its
            // record either way, so later components still parse.
            if flags & ARGS_ARE_XY_VALUES != 0 {
                let child = self.glyph_outline_at_depth(component_glyph_id, depth + 1)?;
                let point_offset = points.len();
                for p in &child.points {
                    points.push(Point::new(
                        a * p.x + c * p.y + dx as f32,
                        b * p.x + d * p.y + dy as f32,
                        p.on_curve,
                    ));
                }
                contour_ends.extend(child.contour_ends.iter().map(|&e| e + point_offset));
            }

            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }

        Some(GlyphOutline {
            points,
            contour_ends,
            min_x,
            min_y,
            max_x,
            max_y,
        })
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

    /// Builds a minimal but fully valid `sfnt` font from raw table bytes —
    /// used to test glyph assembly (composite glyphs) without depending
    /// on a real font file being present on the machine.
    fn build_sfnt(tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let header_len = 12 + 16 * tables.len();
        let mut offset = header_len;
        let mut offsets = Vec::with_capacity(tables.len());
        for (_, data) in tables {
            offsets.push(offset);
            offset += data.len();
        }

        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        for (i, (tag, data)) in tables.iter().enumerate() {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offsets[i] as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        }
        for (_, data) in tables {
            out.extend_from_slice(data);
        }
        out
    }

    /// Builds a single-contour simple glyph from absolute point
    /// coordinates, all points on-curve, each coordinate stored as a full
    /// `i16` delta (simplest possible encoding, not the compact one real
    /// fonts use).
    fn build_simple_glyph(points: &[(i16, i16)]) -> Vec<u8> {
        let min_x = points.iter().map(|p| p.0).min().unwrap();
        let max_x = points.iter().map(|p| p.0).max().unwrap();
        let min_y = points.iter().map(|p| p.1).min().unwrap();
        let max_y = points.iter().map(|p| p.1).max().unwrap();

        let mut data = Vec::new();
        data.extend_from_slice(&1i16.to_be_bytes()); // numberOfContours
        data.extend_from_slice(&min_x.to_be_bytes());
        data.extend_from_slice(&min_y.to_be_bytes());
        data.extend_from_slice(&max_x.to_be_bytes());
        data.extend_from_slice(&max_y.to_be_bytes());
        data.extend_from_slice(&((points.len() - 1) as u16).to_be_bytes()); // endPtsOfContours[0]
        data.extend_from_slice(&0u16.to_be_bytes()); // instructionLength
        // on-curve, explicit (non-repeated, non-short) deltas for every point
        data.resize(data.len() + points.len(), 0x01);
        let mut prev = 0i16;
        for &(x, _) in points {
            data.extend_from_slice(&(x - prev).to_be_bytes());
            prev = x;
        }
        let mut prev = 0i16;
        for &(_, y) in points {
            data.extend_from_slice(&(y - prev).to_be_bytes());
            prev = y;
        }
        data
    }

    /// Builds a composite glyph referencing `components` (glyph id, dx,
    /// dy), each using an identity transform and word-sized xy offsets.
    fn build_composite_glyph(
        components: &[(u16, i16, i16)],
        bbox: (i16, i16, i16, i16),
    ) -> Vec<u8> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const MORE_COMPONENTS: u16 = 0x0020;

        let mut data = Vec::new();
        data.extend_from_slice(&(-1i16).to_be_bytes()); // numberOfContours
        data.extend_from_slice(&bbox.0.to_be_bytes());
        data.extend_from_slice(&bbox.1.to_be_bytes());
        data.extend_from_slice(&bbox.2.to_be_bytes());
        data.extend_from_slice(&bbox.3.to_be_bytes());
        for (i, &(glyph_id, dx, dy)) in components.iter().enumerate() {
            let is_last = i == components.len() - 1;
            let mut flags = ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES;
            if !is_last {
                flags |= MORE_COMPONENTS;
            }
            data.extend_from_slice(&flags.to_be_bytes());
            data.extend_from_slice(&glyph_id.to_be_bytes());
            data.extend_from_slice(&dx.to_be_bytes());
            data.extend_from_slice(&dy.to_be_bytes());
        }
        data
    }

    /// Assembles a minimal font with an empty `.notdef` (glyph 0) followed
    /// by the given glyphs (already-encoded `glyf` bytes, in glyph-id
    /// order starting at 1) — just enough tables (`head`/`maxp`/`loca`/
    /// `glyf`/`hhea`/`hmtx`) for `Font::parse` and `glyph_outline` to work;
    /// no `cmap`, since these tests address glyphs directly by id.
    fn build_test_font(glyphs: &[Vec<u8>]) -> Vec<u8> {
        let num_glyphs = (glyphs.len() + 1) as u16;

        let mut glyf = Vec::new();
        // loca needs numGlyphs+1 entries: glyph 0 (.notdef) is empty, so
        // both its start (loca[0]) and end (loca[1]) are 0.
        let mut loca_offsets = alloc::vec![0u32, 0u32];
        for g in glyphs {
            glyf.extend_from_slice(g);
            loca_offsets.push(glyf.len() as u32);
        }

        let mut loca = Vec::new();
        for off in &loca_offsets {
            loca.extend_from_slice(&off.to_be_bytes());
        }

        let mut head = alloc::vec![0u8; 54];
        head[18..20].copy_from_slice(&2048u16.to_be_bytes()); // unitsPerEm
        head[50..52].copy_from_slice(&1i16.to_be_bytes()); // indexToLocFormat: long

        let mut maxp = alloc::vec![0u8; 6];
        maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());

        let mut hhea = alloc::vec![0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes()); // ascender
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes()); // descender
        hhea[8..10].copy_from_slice(&0i16.to_be_bytes()); // lineGap
        hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes()); // numberOfHMetrics

        let mut hmtx = Vec::new();
        for _ in 0..num_glyphs {
            hmtx.extend_from_slice(&500u16.to_be_bytes()); // advanceWidth
            hmtx.extend_from_slice(&0i16.to_be_bytes()); // lsb
        }

        build_sfnt(&[
            (b"head", &head),
            (b"maxp", &maxp),
            (b"loca", &loca),
            (b"glyf", &glyf),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
        ])
    }

    #[test]
    fn composite_glyph_assembles_both_components_with_offsets_applied() {
        // Glyph 1: a triangle base shape. Glyph 2: a small diacritic mark
        // offset above it. Glyph 3: a composite combining both, with
        // component 2 shifted by (100, 800) -- simulating an accented
        // Latin character built from two component glyphs.
        let base = build_simple_glyph(&[(0, 0), (400, 0), (200, 600)]);
        let mark = build_simple_glyph(&[(0, 0), (100, 0), (50, 100)]);
        let composite = build_composite_glyph(&[(1, 0, 0), (2, 100, 800)], (0, 0, 500, 1000));

        let bytes = build_test_font(&[base, mark, composite]);
        let font = Font::parse(&bytes).expect("synthetic font should parse");

        let outline = font
            .glyph_outline(3)
            .expect("composite glyph should assemble");

        assert_eq!(
            outline.contour_ends.len(),
            2,
            "one contour per component, not merged into one"
        );
        assert_eq!(
            outline.points.len(),
            6,
            "all 3 points from each of the 2 components should be present"
        );
        // First component: offset (0, 0), so its points pass through
        // unchanged.
        assert_eq!(outline.points[0], Point::new(0.0, 0.0, true));
        assert_eq!(outline.points[1], Point::new(400.0, 0.0, true));
        assert_eq!(outline.points[2], Point::new(200.0, 600.0, true));
        // Second component: offset (100, 800) applied to every point.
        assert_eq!(outline.points[3], Point::new(100.0, 800.0, true));
        assert_eq!(outline.points[4], Point::new(200.0, 800.0, true));
        assert_eq!(outline.points[5], Point::new(150.0, 900.0, true));
        // contour_ends offset by the running point count: component 1 is
        // points [0..=2], component 2 is [3..=5].
        assert_eq!(outline.contour_ends, alloc::vec![2, 5]);
    }

    #[test]
    fn composite_glyph_component_scale_transform_is_applied() {
        // A component with WE_HAVE_A_SCALE (0x0008) set: a single F2Dot14
        // scale factor of 1.5 (0x6000 in 2.14 fixed point -- F2Dot14 is
        // signed with 2 integer bits, so 2.0 itself isn't representable)
        // applied to both x and y before the (0, 0) offset.
        let base = build_simple_glyph(&[(0, 0), (100, 0), (50, 100)]);

        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        let mut composite = Vec::new();
        composite.extend_from_slice(&(-1i16).to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        composite.extend_from_slice(&200i16.to_be_bytes());
        composite.extend_from_slice(&200i16.to_be_bytes());
        let flags = ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES | WE_HAVE_A_SCALE;
        composite.extend_from_slice(&flags.to_be_bytes());
        composite.extend_from_slice(&1u16.to_be_bytes()); // component glyph id
        composite.extend_from_slice(&0i16.to_be_bytes()); // dx
        composite.extend_from_slice(&0i16.to_be_bytes()); // dy
        composite.extend_from_slice(&0x6000u16.to_be_bytes()); // scale = 1.5 in F2Dot14

        let bytes = build_test_font(&[base, composite]);
        let font = Font::parse(&bytes).expect("synthetic font should parse");
        let outline = font
            .glyph_outline(2)
            .expect("composite glyph should assemble");

        assert_eq!(outline.points[0], Point::new(0.0, 0.0, true));
        assert_eq!(outline.points[1], Point::new(150.0, 0.0, true));
        assert_eq!(outline.points[2], Point::new(75.0, 150.0, true));
    }

    #[test]
    fn composite_glyph_point_matching_component_is_skipped_not_fabricated() {
        // Component 1 uses ARGS_ARE_XY_VALUES (real offsets); component 2
        // omits it (point-matching, a documented unsupported mode). The
        // assembled outline should contain only component 1's points --
        // skipped, not a wrong/fabricated position for component 2.
        let base = build_simple_glyph(&[(0, 0), (100, 0), (50, 100)]);
        let other = build_simple_glyph(&[(0, 0), (10, 0), (5, 10)]);

        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const MORE_COMPONENTS: u16 = 0x0020;
        let mut composite = Vec::new();
        composite.extend_from_slice(&(-1i16).to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        composite.extend_from_slice(&100i16.to_be_bytes());
        composite.extend_from_slice(&100i16.to_be_bytes());
        // Component 1: real xy offsets, more components follow.
        let flags1 = ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES | MORE_COMPONENTS;
        composite.extend_from_slice(&flags1.to_be_bytes());
        composite.extend_from_slice(&1u16.to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes());
        // Component 2: point-matching (ARGS_ARE_XY_VALUES unset), last.
        let flags2 = ARG_1_AND_2_ARE_WORDS;
        composite.extend_from_slice(&flags2.to_be_bytes());
        composite.extend_from_slice(&2u16.to_be_bytes());
        composite.extend_from_slice(&0i16.to_be_bytes()); // point indices, not offsets
        composite.extend_from_slice(&0i16.to_be_bytes());

        let bytes = build_test_font(&[base, other, composite]);
        let font = Font::parse(&bytes).expect("synthetic font should parse");
        let outline = font
            .glyph_outline(3)
            .expect("composite glyph should still assemble the components it can");

        assert_eq!(
            outline.points.len(),
            3,
            "only component 1's 3 points should be present; component 2 is skipped"
        );
        assert_eq!(outline.contour_ends, alloc::vec![2]);
    }

    /// Builds a CFF INDEX using a fixed 1-byte offset size -- only valid
    /// while every item stays small enough that all offsets fit in a
    /// `u8`, which holds for these tests' tiny charstrings/dicts.
    fn build_cff_index(items: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(items.len() as u16).to_be_bytes());
        if items.is_empty() {
            return out;
        }
        out.push(1u8); // offSize
        let mut offset = 1usize; // offsets are 1-based
        out.push(offset as u8);
        for item in items {
            offset += item.len();
            assert!(offset <= 255, "test CFF INDEX offset overflowed a u8");
            out.push(offset as u8);
        }
        for item in items {
            out.extend_from_slice(item);
        }
        out
    }

    /// A DICT integer operand, always encoded via the 5-byte (marker 29 +
    /// `i32`) form regardless of magnitude -- fixed-width, which is what
    /// lets `build_cff_table` size the Top DICT before its real offset
    /// values (which depend on what comes after it) are known.
    fn dict_int(v: i32) -> Vec<u8> {
        let mut out = alloc::vec![29u8];
        out.extend_from_slice(&v.to_be_bytes());
        out
    }

    fn build_cff_top_dict(
        charstrings_offset: i32,
        private_size: i32,
        private_offset: i32,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(dict_int(charstrings_offset));
        out.push(17); // CharStrings
        out.extend(dict_int(private_size));
        out.extend(dict_int(private_offset));
        out.push(18); // Private
        out
    }

    /// Assembles a minimal, standalone `CFF ` table (Header, Name/Top
    /// DICT/String/Global Subr INDEXes, CharStrings INDEX, and -- only if
    /// `local_subrs_items` is non-empty -- a Private DICT + Local Subr
    /// INDEX) from raw Type 2 charstring/subroutine bytes.
    fn build_cff_table(
        charstrings_items: &[&[u8]],
        global_subrs_items: &[&[u8]],
        local_subrs_items: &[&[u8]],
    ) -> Vec<u8> {
        let header: &[u8] = &[1, 0, 4, 1]; // major, minor, hdrSize, offSize
        let name_index = build_cff_index(&[b"Test"]);
        // Top DICT's encoded length doesn't depend on the actual offset
        // values (dict_int is fixed-width), so size it now with
        // placeholders to learn where CharStrings will land.
        let topdict_placeholder = build_cff_top_dict(0, 0, 0);
        let topdict_index_len = build_cff_index(&[&topdict_placeholder]).len();
        let string_index = build_cff_index(&[]);
        let global_subrs_index = build_cff_index(global_subrs_items);

        let charstrings_offset = header.len()
            + name_index.len()
            + topdict_index_len
            + string_index.len()
            + global_subrs_index.len();
        let charstrings_index = build_cff_index(charstrings_items);

        // Subrs offset is relative to the Private DICT's own start; a
        // dict holding just `Subrs` is always dict_int (5 bytes) + the
        // operator byte (1 byte) = 6 bytes, so the Local Subr INDEX
        // (placed right after) is always at relative offset 6.
        let private_dict: Vec<u8> = if local_subrs_items.is_empty() {
            Vec::new()
        } else {
            let mut d = dict_int(6);
            d.push(19); // Subrs
            d
        };
        let private_offset = charstrings_offset + charstrings_index.len();
        let local_subrs_index = build_cff_index(local_subrs_items);

        let top_dict = build_cff_top_dict(
            charstrings_offset as i32,
            private_dict.len() as i32,
            private_offset as i32,
        );
        let top_dict_index = build_cff_index(&[&top_dict]);
        assert_eq!(
            top_dict_index.len(),
            topdict_index_len,
            "Top DICT INDEX size must not change once real offsets are substituted"
        );

        let mut out = Vec::new();
        out.extend_from_slice(header);
        out.extend_from_slice(&name_index);
        out.extend_from_slice(&top_dict_index);
        out.extend_from_slice(&string_index);
        out.extend_from_slice(&global_subrs_index);
        out.extend_from_slice(&charstrings_index);
        out.extend_from_slice(&private_dict);
        out.extend_from_slice(&local_subrs_index);
        out
    }

    /// Wraps a `CFF ` table in a minimal `OTTO`-tagged `sfnt` font --
    /// `head`/`maxp`/`hhea`/`hmtx` are required alongside `CFF ` even
    /// though outlines don't come from `loca`/`glyf`.
    fn build_otto_font(cff_table: &[u8], num_glyphs: u16) -> Vec<u8> {
        let head = alloc::vec![0u8; 54];
        let mut maxp = alloc::vec![0u8; 6];
        maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
        let hhea = alloc::vec![0u8; 36];
        let hmtx = alloc::vec![0u8; 4];

        let mut bytes = build_sfnt(&[
            (b"CFF ", cff_table),
            (b"head", &head),
            (b"maxp", &maxp),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
        ]);
        bytes[0..4].copy_from_slice(b"OTTO");
        bytes
    }

    #[test]
    fn cff_charstring_draws_a_triangle_via_moveto_and_lineto() {
        // 0 0 rmoveto; 100 0 -50 100 rlineto; endchar
        let charstring: &[u8] = &[139, 139, 21, 239, 139, 89, 239, 5, 14];
        let cff = build_cff_table(&[charstring], &[], &[]);
        let font =
            Font::parse(&build_otto_font(&cff, 1)).expect("synthetic OTTO font should parse");

        let outline = font.glyph_outline(0).expect("CFF glyph should assemble");
        assert_eq!(
            outline.points,
            alloc::vec![
                Point::new(0.0, 0.0, true),
                Point::new(100.0, 0.0, true),
                Point::new(50.0, 100.0, true),
            ]
        );
        assert_eq!(outline.contour_ends, alloc::vec![2]);
    }

    #[test]
    fn cff_charstring_curve_is_flattened_to_line_segments() {
        // 0 0 rmoveto; 100 0 100 100 0 100 rrcurveto; endchar
        let charstring: &[u8] = &[139, 139, 21, 239, 139, 239, 239, 139, 239, 8, 14];
        let cff = build_cff_table(&[charstring], &[], &[]);
        let font =
            Font::parse(&build_otto_font(&cff, 1)).expect("synthetic OTTO font should parse");

        let outline = font.glyph_outline(0).expect("CFF glyph should assemble");
        // The moveto's point, plus a fixed 8-segment flattening of the
        // one curve.
        assert_eq!(outline.points.len(), 9);
        assert_eq!(outline.points[0], Point::new(0.0, 0.0, true));
        // p0=(0,0) p1=(100,0) p2=(200,100) p3=(200,200) -- the curve's
        // exact endpoint, which the flattening must land on precisely at
        // t=1.
        assert_eq!(
            *outline.points.last().unwrap(),
            Point::new(200.0, 200.0, true)
        );
        assert_eq!(outline.contour_ends, alloc::vec![8]);
    }

    #[test]
    fn cff_charstring_calls_a_global_subroutine() {
        // Global subr 0: 100 0 rlineto; return
        let subr: &[u8] = &[239, 139, 5, 11];
        // Main: 0 0 rmoveto; -107 callgsubr; endchar
        // (bias for 1 global subr is 107, so index 0 encodes as 0-107=-107)
        let charstring: &[u8] = &[139, 139, 21, 32, 29, 14];
        let cff = build_cff_table(&[charstring], &[subr], &[]);
        let font =
            Font::parse(&build_otto_font(&cff, 1)).expect("synthetic OTTO font should parse");

        let outline = font.glyph_outline(0).expect("CFF glyph should assemble");
        assert_eq!(
            outline.points,
            alloc::vec![Point::new(0.0, 0.0, true), Point::new(100.0, 0.0, true)]
        );
        assert_eq!(outline.contour_ends, alloc::vec![1]);
    }

    #[test]
    fn cff_charstring_hintmask_bytes_are_skipped_without_desyncing_the_parser() {
        // 0 100 hstemhm (1 stem); hintmask <1 byte>; 10 10 rmoveto;
        // 5 5 rlineto; endchar
        let charstring: &[u8] = &[139, 239, 18, 19, 0x80, 149, 149, 21, 144, 144, 5, 14];
        let cff = build_cff_table(&[charstring], &[], &[]);
        let font =
            Font::parse(&build_otto_font(&cff, 1)).expect("synthetic OTTO font should parse");

        let outline = font.glyph_outline(0).expect("CFF glyph should assemble");
        assert_eq!(
            outline.points,
            alloc::vec![Point::new(10.0, 10.0, true), Point::new(15.0, 15.0, true)]
        );
        assert_eq!(outline.contour_ends, alloc::vec![1]);
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
        // `OTTO` (CFF-flavor OpenType) is supported; legacy Mac `true` is
        // not -- that's the real remaining unsupported version tag.
        let mut bytes = alloc::vec![0u8; 12];
        bytes[0..4].copy_from_slice(b"true");
        assert!(matches!(
            Font::parse(&bytes),
            Err(FontError::UnsupportedVersion)
        ));
    }

    #[test]
    fn otto_without_a_cff_table_is_missing_table_not_unsupported_version() {
        // OTTO itself is a supported version tag; a font claiming it
        // without shipping a `CFF ` table is malformed, not unsupported.
        let head = alloc::vec![0u8; 54];
        let mut maxp = alloc::vec![0u8; 6];
        maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
        let hhea = alloc::vec![0u8; 36];
        let hmtx = alloc::vec![0u8; 4];

        let mut bytes = build_sfnt(&[
            (b"head", &head),
            (b"maxp", &maxp),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
        ]);
        bytes[0..4].copy_from_slice(b"OTTO");

        assert!(matches!(
            Font::parse(&bytes),
            Err(FontError::MissingTable("CFF "))
        ));
    }
}
