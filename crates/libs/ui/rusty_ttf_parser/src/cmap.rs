//! `cmap`: character to glyph mapping (subtable formats 4, 6 and 12).

use crate::reader::{u16_at, u32_at};

/// A supported `cmap` subtable, chosen once at [`Face`](crate::Face) parse time.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Subtable<'a> {
    /// Segment mapping to delta values (BMP).
    Format4(&'a [u8]),
    /// Trimmed table mapping (BMP, dense).
    Format6(&'a [u8]),
    /// Segmented coverage (full Unicode).
    Format12(&'a [u8]),
}

/// Pick the best Unicode subtable in a `cmap` table, preferring full-repertoire
/// (format 12) encodings over BMP-only ones.
pub(crate) fn select(cmap: &[u8]) -> Option<Subtable<'_>> {
    let count = u16_at(cmap, 2)?;
    let mut best: Option<(u8, Subtable<'_>)> = None;
    for i in 0..usize::from(count) {
        let record = 4 + i * 8;
        let (platform, encoding) = (u16_at(cmap, record)?, u16_at(cmap, record + 2)?);
        let rank = match (platform, encoding) {
            (3, 10) => 5,
            (0, 4 | 6) => 4,
            (3, 1) => 3,
            (0, 0..=3) => 2,
            _ => continue,
        };
        if best.is_some_and(|(r, _)| r >= rank) {
            continue;
        }
        let offset = usize::try_from(u32_at(cmap, record + 4)?).ok()?;
        let Some(table) = cmap.get(offset..) else {
            continue;
        };
        let subtable = match u16_at(table, 0) {
            Some(4) => Subtable::Format4(table),
            Some(6) => Subtable::Format6(table),
            Some(12) => Subtable::Format12(table),
            _ => continue,
        };
        best = Some((rank, subtable));
    }
    best.map(|(_, subtable)| subtable)
}

impl Subtable<'_> {
    /// The glyph for a code point; `None` if unmapped (glyph 0 counts as unmapped).
    pub(crate) fn glyph_index(&self, code: u32) -> Option<u16> {
        let glyph = match *self {
            Subtable::Format4(t) => format4(t, code)?,
            Subtable::Format6(t) => format6(t, code)?,
            Subtable::Format12(t) => format12(t, code)?,
        };
        (glyph != 0).then_some(glyph)
    }
}

fn format4(t: &[u8], code: u32) -> Option<u16> {
    let code = u16::try_from(code).ok()?;
    let segments = usize::from(u16_at(t, 6)? / 2);
    let end_codes = 14;
    let start_codes = end_codes + segments * 2 + 2;
    let deltas = start_codes + segments * 2;
    let range_offsets = deltas + segments * 2;

    // First segment whose end code is >= `code`.
    let (mut lo, mut hi) = (0, segments);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if u16_at(t, end_codes + mid * 2)? < code {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == segments {
        return None;
    }
    let start = u16_at(t, start_codes + lo * 2)?;
    if code < start {
        return None;
    }
    let delta = u16_at(t, deltas + lo * 2)?;
    let range_offset = usize::from(u16_at(t, range_offsets + lo * 2)?);
    if range_offset == 0 {
        return Some(code.wrapping_add(delta));
    }
    let at = range_offsets + lo * 2 + range_offset + usize::from(code - start) * 2;
    match u16_at(t, at)? {
        0 => None,
        glyph => Some(glyph.wrapping_add(delta)),
    }
}

fn format6(t: &[u8], code: u32) -> Option<u16> {
    let first = u32::from(u16_at(t, 6)?);
    let count = u32::from(u16_at(t, 8)?);
    let index = code.checked_sub(first).filter(|i| *i < count)?;
    u16_at(t, 10 + usize::try_from(index).ok()? * 2)
}

fn format12(t: &[u8], code: u32) -> Option<u16> {
    let groups = usize::try_from(u32_at(t, 12)?).ok()?;
    let group = |i: usize| -> Option<(u32, u32, u32)> {
        let at = 16 + i.checked_mul(12)?;
        Some((u32_at(t, at)?, u32_at(t, at + 4)?, u32_at(t, at + 8)?))
    };
    let (mut lo, mut hi) = (0, groups);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if group(mid)?.1 < code {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == groups {
        return None; // past the last group; never read beyond the declared count
    }
    let (start, _end, start_glyph) = group(lo)?;
    if code < start {
        return None;
    }
    u16::try_from(start_glyph.checked_add(code - start)?).ok()
}
