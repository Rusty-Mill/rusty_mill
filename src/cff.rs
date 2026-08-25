//! CFF table parsing and a Type 2 charstring interpreter — the outline
//! format `OTTO`-tagged OpenType fonts use instead of TrueType's
//! `loca`/`glyf`. Scope: standard (non-CID-keyed) CFF, which covers the
//! common case of a professional OpenType text font; CID-keyed CFF (used
//! mainly for CJK, with per-glyph `FDArray`/`FDSelect` private dicts) is a
//! documented remaining gap, same spirit as `ttf.rs`'s composite
//! point-matching gap.
//!
//! Type 2 charstrings describe outlines as cubic Bézier curves, but
//! [`crate::glyph::GlyphOutline`]/[`crate::rasterizer::Rasterizer`] are
//! built around TrueType's on/off-curve quadratic point model. Rather than
//! extend that model for one outline format, each cubic curve is
//! flattened into a short run of on-curve line segments at parse time —
//! an approximation (not the exact curve), acceptable at the point counts
//! real glyphs render at.

use crate::glyph::{GlyphOutline, Point};
use alloc::vec::Vec;

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    u16_at(data, offset).map(|v| v as i16)
}

fn read_be_uint(data: &[u8], offset: usize, size: u8) -> Option<usize> {
    let bytes = data.get(offset..offset + size as usize)?;
    Some(bytes.iter().fold(0usize, |acc, &b| (acc << 8) | b as usize))
}

/// The tables this parser needs out of a `CFF ` table: byte ranges (as
/// absolute offsets into the font's own data, not relative to the `CFF `
/// table) for each glyph's charstring, and the global/local subroutine
/// INDEXes charstrings can call into.
pub(crate) struct CffTable {
    charstrings: Vec<(usize, usize)>,
    global_subrs: Vec<(usize, usize)>,
    local_subrs: Vec<(usize, usize)>,
}

/// Reads one CFF INDEX structure starting at `pos` within `cff`. Returns
/// the item byte ranges (absolute offsets within `cff`) and the position
/// immediately following the INDEX.
fn parse_cff_index(cff: &[u8], pos: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    let count = u16_at(cff, pos)? as usize;
    if count == 0 {
        return Some((Vec::new(), pos + 2));
    }
    let off_size = *cff.get(pos + 2)?;
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offsets_start = pos + 3;
    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..=count {
        offsets.push(read_be_uint(
            cff,
            offsets_start + i * off_size as usize,
            off_size,
        )?);
    }
    // Offsets are 1-based, relative to the byte preceding the object data.
    let base = offsets_start + (count + 1) * off_size as usize - 1;
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let start = base + offsets[i];
        let end = base + offsets[i + 1];
        if end < start || end > cff.len() {
            return None;
        }
        items.push((start, end));
    }
    Some((items, base + offsets[count]))
}

/// Parses a CFF DICT (Top DICT or Private DICT) into `(operator,
/// operands)` pairs. Two-byte operators (`12 <n>`) are folded into a
/// single `0x0c00 | n` key so callers can match on one `u16` space.
fn parse_cff_dict(dict: &[u8]) -> Option<Vec<(u16, Vec<f64>)>> {
    let mut result = Vec::new();
    let mut operands: Vec<f64> = Vec::new();
    let mut pos = 0usize;
    while pos < dict.len() {
        let b0 = dict[pos];
        match b0 {
            32..=246 => {
                operands.push((b0 as i32 - 139) as f64);
                pos += 1;
            }
            247..=250 => {
                let b1 = *dict.get(pos + 1)?;
                operands.push(((b0 as i32 - 247) * 256 + b1 as i32 + 108) as f64);
                pos += 2;
            }
            251..=254 => {
                let b1 = *dict.get(pos + 1)?;
                operands.push((-((b0 as i32 - 251) * 256) - b1 as i32 - 108) as f64);
                pos += 2;
            }
            28 => {
                operands.push(i16_at(dict, pos + 1)? as f64);
                pos += 3;
            }
            29 => {
                let b = dict.get(pos + 1..pos + 5)?;
                operands.push(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64);
                pos += 5;
            }
            30 => {
                // Real number: nibble-encoded, terminated by a nibble of
                // 0xf. The value itself is unused by anything this parser
                // reads (FontMatrix and similar), so just skip past it.
                pos += 1;
                loop {
                    let byte = *dict.get(pos)?;
                    pos += 1;
                    if byte >> 4 == 0xf || byte & 0x0f == 0xf {
                        break;
                    }
                }
                operands.push(0.0);
            }
            0..=21 => {
                let (op, adv) = if b0 == 12 {
                    (0x0c00 | *dict.get(pos + 1)? as u16, 2)
                } else {
                    (b0 as u16, 1)
                };
                pos += adv;
                result.push((op, core::mem::take(&mut operands)));
            }
            _ => return None,
        }
    }
    Some(result)
}

/// Parses a `CFF ` table (`offset`/`length` within `data`) into the pieces
/// [`glyph_outline`] needs. `None` on any malformed structure — the
/// caller treats that as `FontError::Malformed`, same as any other
/// required table.
pub(crate) fn parse_cff_table(data: &[u8], offset: usize, length: usize) -> Option<CffTable> {
    let cff = data.get(offset..offset + length)?;
    let hdr_size = *cff.get(2)? as usize;

    let (_name_index, pos) = parse_cff_index(cff, hdr_size)?;
    let (top_dicts, pos) = parse_cff_index(cff, pos)?;
    let (_string_index, pos) = parse_cff_index(cff, pos)?;
    let (global_subrs, _pos) = parse_cff_index(cff, pos)?;

    let &(top_dict_start, top_dict_end) = top_dicts.first()?;
    let top_dict = parse_cff_dict(cff.get(top_dict_start..top_dict_end)?)?;

    let charstrings_offset = top_dict
        .iter()
        .find(|(op, _)| *op == 17)
        .and_then(|(_, v)| v.first())
        .copied()? as usize;
    let (charstrings, _) = parse_cff_index(cff, charstrings_offset)?;

    let mut local_subrs = Vec::new();
    if let Some((_, priv_operands)) = top_dict.iter().find(|(op, _)| *op == 18) {
        if let [priv_size, priv_offset] = priv_operands[..] {
            let (priv_size, priv_offset) = (priv_size as usize, priv_offset as usize);
            if let Some(priv_dict) = cff.get(priv_offset..priv_offset + priv_size) {
                if let Some(priv_entries) = parse_cff_dict(priv_dict) {
                    if let Some(subrs_rel) = priv_entries
                        .iter()
                        .find(|(op, _)| *op == 19)
                        .and_then(|(_, v)| v.first())
                    {
                        if let Some((subrs, _)) =
                            parse_cff_index(cff, priv_offset + *subrs_rel as usize)
                        {
                            local_subrs = subrs;
                        }
                    }
                }
            }
        }
    }

    let to_absolute = |ranges: Vec<(usize, usize)>| -> Vec<(usize, usize)> {
        ranges
            .into_iter()
            .map(|(s, e)| (s + offset, e + offset))
            .collect()
    };
    Some(CffTable {
        charstrings: to_absolute(charstrings),
        global_subrs: to_absolute(global_subrs),
        local_subrs: to_absolute(local_subrs),
    })
}

/// The bias added to a `callsubr`/`callgsubr` index before indexing into
/// the local/global Subrs INDEX (Type 2 spec section 4.7) — a fixed
/// offset chosen so small, frequently-used subroutine numbers can be
/// encoded as small (often negative) charstring operands.
fn subr_bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

/// Rounds to the nearest integer without `f32::round` (unavailable in
/// `core` -- needs `libm`), unlike `rasterizer.rs`'s `round_nonneg` this
/// also handles negative values (glyph coordinates routinely are, e.g.
/// descenders below the baseline): round-half-away-from-zero via a 0.5
/// nudge before truncation.
fn round_f32(x: f32) -> i16 {
    (if x >= 0.0 { x + 0.5 } else { x - 0.5 }) as i16
}

fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    out: &mut Vec<Point>,
) {
    // A fixed subdivision count is a simplification (not adaptive to
    // curve flatness), but plenty for the point density real glyphs
    // render at, and keeps this from needing an error-bound estimator.
    const STEPS: u32 = 8;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let mt = 1.0 - t;
        let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        out.push(Point::new(
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
            true,
        ));
    }
}

struct Interpreter<'a> {
    data: &'a [u8],
    cff: &'a CffTable,
    stack: Vec<f32>,
    x: f32,
    y: f32,
    num_stems: usize,
    width_taken: bool,
    points: Vec<Point>,
    contour_ends: Vec<usize>,
    contour_open: bool,
    depth: u32,
}

impl<'a> Interpreter<'a> {
    /// Real fonts nest subroutine calls at most a few levels deep; this
    /// guards against a malformed or (`callsubr`-cycle) pathological
    /// charstring recursing unboundedly.
    const MAX_DEPTH: u32 = 10;

    fn close_contour(&mut self) {
        if self.contour_open {
            self.contour_ends.push(self.points.len() - 1);
            self.contour_open = false;
        }
    }

    fn start_contour(&mut self) {
        self.points.push(Point::new(self.x, self.y, true));
        self.contour_open = true;
    }

    fn line_to(&mut self) {
        self.points.push(Point::new(self.x, self.y, true));
    }

    fn curve_to(&mut self, dxa: f32, dya: f32, dxb: f32, dyb: f32, dxc: f32, dyc: f32) {
        let p0 = (self.x, self.y);
        let p1 = (p0.0 + dxa, p0.1 + dya);
        let p2 = (p1.0 + dxb, p1.1 + dyb);
        let p3 = (p2.0 + dxc, p2.1 + dyc);
        flatten_cubic(p0, p1, p2, p3, &mut self.points);
        self.x = p3.0;
        self.y = p3.1;
    }

    /// The (at most one) leading width value is only ever present on the
    /// first stack-clearing operator in a charstring, and only when that
    /// operator's argument count exceeds what it normally takes. Called
    /// once per such operator; a no-op after the first call.
    fn strip_width(&mut self, normal_exact_count: Option<usize>) {
        if self.width_taken {
            return;
        }
        self.width_taken = true;
        let has_width = match normal_exact_count {
            Some(exact) => self.stack.len() > exact,
            None => self.stack.len() % 2 == 1, // stem hints: args come in pairs
        };
        if has_width && !self.stack.is_empty() {
            self.stack.remove(0);
        }
    }

    fn call_subr(&mut self, index: f32, global: bool) -> Option<bool> {
        self.depth += 1;
        if self.depth > Self::MAX_DEPTH {
            return None;
        }
        let subrs = if global {
            &self.cff.global_subrs
        } else {
            &self.cff.local_subrs
        };
        let real_index = index as i32 + subr_bias(subrs.len());
        let stop = if real_index >= 0 {
            let &(start, end) = subrs.get(real_index as usize)?;
            let code = self.data.get(start..end)?;
            self.run(code)?
        } else {
            return None;
        };
        self.depth -= 1;
        Some(stop)
    }

    /// Runs one charstring (top-level or a subroutine). Returns `Some(true)`
    /// if `endchar` was reached (the whole glyph is finished — propagate
    /// up through any nested `callsubr`/`callgsubr`), `Some(false)` on an
    /// explicit `return` or falling off the end of `code` (continue the
    /// caller's own charstring), `None` on malformed data.
    fn run(&mut self, code: &[u8]) -> Option<bool> {
        let mut pos = 0usize;
        while pos < code.len() {
            let b0 = code[pos];
            if b0 >= 32 || b0 == 28 {
                let (value, adv): (f32, usize) = match b0 {
                    32..=246 => ((b0 as i32 - 139) as f32, 1),
                    247..=250 => {
                        let b1 = *code.get(pos + 1)?;
                        (((b0 as i32 - 247) * 256 + b1 as i32 + 108) as f32, 2)
                    }
                    251..=254 => {
                        let b1 = *code.get(pos + 1)?;
                        ((-((b0 as i32 - 251) * 256) - b1 as i32 - 108) as f32, 2)
                    }
                    28 => (i16_at(code, pos + 1)? as f32, 3),
                    255 => {
                        let b = code.get(pos + 1..pos + 5)?;
                        (
                            i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 65536.0,
                            5,
                        )
                    }
                    _ => unreachable!(),
                };
                self.stack.push(value);
                pos += adv;
                continue;
            }

            let (op, adv) = if b0 == 12 {
                (0x0c00 | *code.get(pos + 1)? as u16, 2)
            } else {
                (b0 as u16, 1)
            };
            pos += adv;

            match op {
                1 | 3 | 18 | 23 => {
                    // hstem, vstem, hstemhm, vstemhm
                    self.strip_width(None);
                    self.num_stems += self.stack.len() / 2;
                    self.stack.clear();
                }
                19 | 20 => {
                    // hintmask, cntrmask -- any pending args here are an
                    // implicit final vstemhm before the mask bytes.
                    self.strip_width(None);
                    self.num_stems += self.stack.len() / 2;
                    self.stack.clear();
                    pos += self.num_stems.div_ceil(8);
                }
                21 => {
                    // rmoveto
                    self.strip_width(Some(2));
                    self.close_contour();
                    self.x += *self.stack.first()?;
                    self.y += *self.stack.get(1)?;
                    self.start_contour();
                    self.stack.clear();
                }
                22 => {
                    // hmoveto
                    self.strip_width(Some(1));
                    self.close_contour();
                    self.x += *self.stack.first()?;
                    self.start_contour();
                    self.stack.clear();
                }
                4 => {
                    // vmoveto
                    self.strip_width(Some(1));
                    self.close_contour();
                    self.y += *self.stack.first()?;
                    self.start_contour();
                    self.stack.clear();
                }
                5 => {
                    // rlineto: {dxa dya}+
                    let args = core::mem::take(&mut self.stack);
                    let mut i = 0;
                    while i + 1 < args.len() {
                        self.x += args[i];
                        self.y += args[i + 1];
                        self.line_to();
                        i += 2;
                    }
                }
                6 => {
                    // hlineto: alternating horizontal, vertical, ...
                    let args = core::mem::take(&mut self.stack);
                    for (i, &v) in args.iter().enumerate() {
                        if i % 2 == 0 {
                            self.x += v;
                        } else {
                            self.y += v;
                        }
                        self.line_to();
                    }
                }
                7 => {
                    // vlineto: alternating vertical, horizontal, ...
                    let args = core::mem::take(&mut self.stack);
                    for (i, &v) in args.iter().enumerate() {
                        if i % 2 == 0 {
                            self.y += v;
                        } else {
                            self.x += v;
                        }
                        self.line_to();
                    }
                }
                8 => {
                    // rrcurveto: {dxa dya dxb dyb dxc dyc}+
                    let args = core::mem::take(&mut self.stack);
                    let mut i = 0;
                    while i + 5 < args.len() {
                        self.curve_to(
                            args[i],
                            args[i + 1],
                            args[i + 2],
                            args[i + 3],
                            args[i + 4],
                            args[i + 5],
                        );
                        i += 6;
                    }
                }
                24 => {
                    // rcurveline: {dxa dya dxb dyb dxc dyc}+ dxd dyd
                    let args = core::mem::take(&mut self.stack);
                    if args.len() < 8 {
                        return None;
                    }
                    let num_curve_args = ((args.len() - 2) / 6) * 6;
                    let mut i = 0;
                    while i < num_curve_args {
                        self.curve_to(
                            args[i],
                            args[i + 1],
                            args[i + 2],
                            args[i + 3],
                            args[i + 4],
                            args[i + 5],
                        );
                        i += 6;
                    }
                    self.x += args[i];
                    self.y += args[i + 1];
                    self.line_to();
                }
                25 => {
                    // rlinecurve: {dxa dya}+ dxb dyb dxc dyc dxd dyd
                    let args = core::mem::take(&mut self.stack);
                    if args.len() < 8 {
                        return None;
                    }
                    let num_line_args = ((args.len() - 6) / 2) * 2;
                    let mut i = 0;
                    while i < num_line_args {
                        self.x += args[i];
                        self.y += args[i + 1];
                        self.line_to();
                        i += 2;
                    }
                    self.curve_to(
                        args[i],
                        args[i + 1],
                        args[i + 2],
                        args[i + 3],
                        args[i + 4],
                        args[i + 5],
                    );
                }
                26 => {
                    // vvcurveto: dx1? {dya dxb dyb dyc}+
                    let args = core::mem::take(&mut self.stack);
                    let mut i = 0;
                    let mut dx1 = 0.0;
                    if args.len() % 4 == 1 {
                        dx1 = args[0];
                        i = 1;
                    }
                    let mut first = true;
                    while i + 3 < args.len() {
                        let dxa = if first { dx1 } else { 0.0 };
                        self.curve_to(dxa, args[i], args[i + 1], args[i + 2], 0.0, args[i + 3]);
                        first = false;
                        i += 4;
                    }
                }
                27 => {
                    // hhcurveto: dy1? {dxa dxb dyb dxc}+
                    let args = core::mem::take(&mut self.stack);
                    let mut i = 0;
                    let mut dy1 = 0.0;
                    if args.len() % 4 == 1 {
                        dy1 = args[0];
                        i = 1;
                    }
                    let mut first = true;
                    while i + 3 < args.len() {
                        let dya = if first { dy1 } else { 0.0 };
                        self.curve_to(args[i], dya, args[i + 1], args[i + 2], args[i + 3], 0.0);
                        first = false;
                        i += 4;
                    }
                }
                30 | 31 => {
                    // vhcurveto (30) / hvcurveto (31): alternating
                    // curves, first tangent vertical/horizontal
                    // respectively, with an optional trailing extra
                    // value on the final curve.
                    let args = core::mem::take(&mut self.stack);
                    let mut vertical = op == 30;
                    let mut i = 0;
                    while i + 3 < args.len() {
                        let extra = if args.len() - i == 5 {
                            args[i + 4]
                        } else {
                            0.0
                        };
                        if vertical {
                            self.curve_to(
                                0.0,
                                args[i],
                                args[i + 1],
                                args[i + 2],
                                args[i + 3],
                                extra,
                            );
                        } else {
                            self.curve_to(
                                args[i],
                                0.0,
                                args[i + 1],
                                args[i + 2],
                                extra,
                                args[i + 3],
                            );
                        }
                        vertical = !vertical;
                        i += 4;
                    }
                }
                10 => {
                    // callsubr
                    let idx = self.stack.pop()?;
                    if self.call_subr(idx, false)? {
                        return Some(true);
                    }
                }
                29 => {
                    // callgsubr
                    let idx = self.stack.pop()?;
                    if self.call_subr(idx, true)? {
                        return Some(true);
                    }
                }
                11 => return Some(false), // return
                14 => {
                    // endchar. The deprecated 4/5-arg `seac`-style accent
                    // composition isn't implemented -- a documented gap,
                    // same spirit as the TrueType composite point-matching
                    // one; any such args are dropped rather than acted on.
                    self.strip_width(Some(0));
                    self.stack.clear();
                    self.close_contour();
                    return Some(true);
                }
                0x0c22 => {
                    // hflex: dx1 dx2 dy2 dx3 dx4 dx5 dx6
                    let a = core::mem::take(&mut self.stack);
                    if a.len() < 7 {
                        return None;
                    }
                    self.curve_to(a[0], 0.0, a[1], a[2], a[3], 0.0);
                    self.curve_to(a[4], 0.0, a[5], -a[2], a[6], 0.0);
                }
                0x0c23 => {
                    // flex: dx1 dy1 dx2 dy2 dx3 dy3 dx4 dy4 dx5 dy5 dx6 dy6 fd
                    let a = core::mem::take(&mut self.stack);
                    if a.len() < 13 {
                        return None;
                    }
                    self.curve_to(a[0], a[1], a[2], a[3], a[4], a[5]);
                    self.curve_to(a[6], a[7], a[8], a[9], a[10], a[11]);
                }
                0x0c24 => {
                    // hflex1: dx1 dy1 dx2 dy2 dx3 dx4 dx5 dy5 dx6
                    let a = core::mem::take(&mut self.stack);
                    if a.len() < 9 {
                        return None;
                    }
                    self.curve_to(a[0], a[1], a[2], a[3], a[4], 0.0);
                    let dy6 = -(a[1] + a[3] + a[7]);
                    self.curve_to(a[5], 0.0, a[6], a[7], a[8], dy6);
                }
                0x0c25 => {
                    // flex1: dx1 dy1 dx2 dy2 dx3 dy3 dx4 dy4 dx5 dy5 d6
                    let a = core::mem::take(&mut self.stack);
                    if a.len() < 11 {
                        return None;
                    }
                    let dx: f32 = a[0] + a[2] + a[4] + a[6] + a[8];
                    let dy: f32 = a[1] + a[3] + a[5] + a[7] + a[9];
                    self.curve_to(a[0], a[1], a[2], a[3], a[4], a[5]);
                    if dx.abs() > dy.abs() {
                        self.curve_to(a[6], a[7], a[8], a[9], a[10], -dy);
                    } else {
                        self.curve_to(a[6], a[7], a[8], a[9], -dx, a[10]);
                    }
                }
                _ => {
                    // Arithmetic/storage/conditional operators (and, or,
                    // not, abs, add, ..., ifelse) have no effect on
                    // outline shape and are vanishingly rare in real
                    // fonts; drop whatever's on the stack and continue.
                    self.stack.clear();
                }
            }
        }
        Some(false)
    }

    fn finish(self) -> GlyphOutline {
        if self.points.is_empty() {
            return GlyphOutline::default();
        }
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for p in &self.points {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        GlyphOutline {
            points: self.points,
            contour_ends: self.contour_ends,
            min_x: round_f32(min_x),
            min_y: round_f32(min_y),
            max_x: round_f32(max_x),
            max_y: round_f32(max_y),
        }
    }
}

/// Interprets a glyph's Type 2 charstring into a flattened outline. `None`
/// for an out-of-range glyph id or malformed charstring/subroutine data.
pub(crate) fn glyph_outline(data: &[u8], cff: &CffTable, glyph_id: u16) -> Option<GlyphOutline> {
    let &(start, end) = cff.charstrings.get(glyph_id as usize)?;
    let charstring = data.get(start..end)?;
    let mut interp = Interpreter {
        data,
        cff,
        stack: Vec::new(),
        x: 0.0,
        y: 0.0,
        num_stems: 0,
        width_taken: false,
        points: Vec::new(),
        contour_ends: Vec::new(),
        contour_open: false,
        depth: 0,
    };
    interp.run(charstring)?;
    interp.close_contour();
    Some(interp.finish())
}
