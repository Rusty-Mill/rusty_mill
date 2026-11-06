//! A small antialiased painter for chrome drawn in pixels rather than cells:
//! rounded boxes, stroked icons and proportional text, into a [`Pixmap`]
//! both renderers present (the CPU one copies it, the GPU one uploads it).
//!
//! Shapes are covered by distance: a pixel's coverage is how far its centre
//! lies inside the shape's edge, clamped to one pixel. Icons are the pack's
//! 16 px stroke paths (`gui-design/assets/build_mux.py`), stroked with round
//! caps and joins.

use std::cell::Cell;

use super::text::{UiText, Weight};

/// An opaque `0xRRGGBB` image, row-major.
#[derive(Clone, Debug)]
pub(crate) struct Pixmap {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u32>,
    /// The fingerprint's hash once taken; any change clears it, so a
    /// window-sized pixmap is hashed once per paint, not once per frame.
    print: Cell<Option<u64>>,
}

impl PartialEq for Pixmap {
    fn eq(&self, other: &Self) -> bool {
        (self.width, self.height) == (other.width, other.height) && self.pixels == other.pixels
    }
}

impl Eq for Pixmap {}

/// Where a shape is known to cover nothing or everything, so painting it
/// skips the per-pixel distance there.
#[derive(Clone, Copy)]
enum Inner {
    None,
    /// Fully covered: set to the colour.
    Fill(Rect),
    /// Not covered: left alone.
    Skip(Rect),
}

/// A rectangle in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// This box shrunk by `by` on every side, never below nothing.
    pub(crate) fn inset(&self, by: f32) -> Rect {
        Rect {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2.0 * by).max(0.0),
            h: (self.h - 2.0 * by).max(0.0),
        }
    }
}

/// `src` over `dst` at coverage `a` (0..=1).
fn mix(dst: u32, src: u32, a: f32) -> u32 {
    if a <= 0.0 {
        return dst;
    }
    if a >= 1.0 {
        return src;
    }
    let ch = |shift: u32| {
        let (d, s) = (
            ((dst >> shift) & 0xff) as f32,
            ((src >> shift) & 0xff) as f32,
        );
        ((d + (s - d) * a).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// Signed distance from `(px, py)` to a rounded box's edge, negative inside.
fn box_distance(r: &Rect, radius: f32, px: f32, py: f32) -> f32 {
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    let radius = radius.min(hw).min(hh).max(0.0);
    let qx = (px - (r.x + hw)).abs() - (hw - radius);
    let qy = (py - (r.y + hh)).abs() - (hh - radius);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - radius
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

impl Pixmap {
    /// A `width × height` image filled with `color`.
    pub(crate) fn new(width: usize, height: usize, color: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; width * height],
            print: Cell::new(None),
        }
    }

    /// Its pixels, row-major.
    pub(crate) fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Its pixels to change; the fingerprint is taken again after.
    fn pixels_mut(&mut self) -> &mut [u32] {
        *self.print.get_mut() = None;
        &mut self.pixels
    }

    /// Its size and a hash of its pixels: equal pixmaps match, and a change
    /// to any pixel almost surely changes it. Hashed once after a change.
    pub(crate) fn fingerprint(&self) -> (usize, usize, u64) {
        let hash = self.print.get().unwrap_or_else(|| {
            // FNV-1a over whole pixels.
            let hash = self.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &px| {
                (h ^ u64::from(px)).wrapping_mul(0x0100_0000_01b3)
            });
            self.print.set(Some(hash));
            hash
        });
        (self.width, self.height, hash)
    }

    /// Calls `cover` for every pixel centre in `r` (clipped to `clip` and
    /// the image) and blends `color` at the coverage it returns; `inner`
    /// spares it where the answer is known.
    fn shade(
        &mut self,
        r: &Rect,
        clip: &Rect,
        color: u32,
        inner: Inner,
        cover: impl Fn(f32, f32) -> f32,
    ) {
        let (w, h) = (self.width, self.height);
        let span = |a: f32, b: f32, max: usize| {
            (
                (a.floor().max(0.0) as usize).min(max),
                (b.ceil().max(0.0) as usize).min(max),
            )
        };
        let (x0, x1) = span(r.x.max(clip.x), (r.x + r.w).min(clip.x + clip.w), w);
        let (y0, y1) = span(r.y.max(clip.y), (r.y + r.h).min(clip.y + clip.h), h);
        // Pixels whose centres lie in `inner`, per axis.
        let centred = |a: f32, len: f32| {
            let first = (a - 0.5).ceil().max(0.0) as usize;
            let end = (a + len - 0.5).ceil().max(0.0) as usize;
            (first, end.max(first))
        };
        let pixels = self.pixels_mut();
        for y in y0..y1 {
            let row = &mut pixels[y * w..(y + 1) * w];
            let (known, fill) = match inner {
                Inner::Fill(i) => (Some(i), true),
                Inner::Skip(i) => (Some(i), false),
                Inner::None => (None, false),
            };
            let (ix0, ix1) = match known {
                Some(i) if (i.y..i.y + i.h).contains(&(y as f32 + 0.5)) => {
                    let (a, b) = centred(i.x, i.w);
                    (a.clamp(x0, x1), b.clamp(x0, x1))
                }
                _ => (x1, x1),
            };
            for x in (x0..ix0).chain(ix1..x1) {
                let a = cover(x as f32 + 0.5, y as f32 + 0.5);
                row[x] = mix(row[x], color, a);
            }
            if fill {
                row[ix0..ix1].fill(color);
            }
        }
    }

    /// Fill a box with corners of `radius`.
    pub(crate) fn fill(&mut self, r: Rect, radius: f32, color: u32) {
        self.fill_part(r, radius, r, color);
    }

    /// Fill the part of a box with corners of `radius` that lies in `part`.
    pub(crate) fn fill_part(&mut self, r: Rect, radius: f32, part: Rect, color: u32) {
        let inner = Inner::Fill(r.inset(radius.max(0.0) + 1.0));
        self.shade(&r, &part, color, inner, |x, y| {
            (0.5 - box_distance(&r, radius, x, y)).clamp(0.0, 1.0)
        });
    }

    /// Draw a `width`-px border just inside a box with corners of `radius`.
    pub(crate) fn border(&mut self, r: Rect, radius: f32, width: f32, color: u32) {
        let inner = Inner::Skip(r.inset(radius.max(0.0) + width + 1.0));
        self.shade(&r, &r, color, inner, |x, y| {
            let d = box_distance(&r, radius, x, y);
            ((0.5 - d).clamp(0.0, 1.0) - (0.5 - d - width).clamp(0.0, 1.0)).max(0.0)
        });
    }

    /// Stroke the pack icon `path` (a 16-unit SVG path) at `(x, y)`, `size`
    /// px square, `width` px wide at that size, with round caps and joins.
    pub(crate) fn icon(&mut self, path: &str, x: f32, y: f32, size: f32, width: f32, color: u32) {
        let k = size / 16.0;
        let lines: Vec<Vec<(f32, f32)>> = flatten(path)
            .into_iter()
            .map(|l| {
                l.into_iter()
                    .map(|(px, py)| (x + px * k, y + py * k))
                    .collect()
            })
            .collect();
        let half = width / 2.0;
        let bounds = Rect {
            x: x - width,
            y: y - width,
            w: size + 2.0 * width,
            h: size + 2.0 * width,
        };
        self.shade(&bounds, &bounds, color, Inner::None, |px, py| {
            let d = lines
                .iter()
                .flat_map(|l| {
                    let single = (l.len() == 1).then(|| (l[0], l[0]));
                    l.windows(2).map(|w| (w[0], w[1])).chain(single)
                })
                .map(|(a, b)| segment_distance((px, py), a, b))
                .fold(f32::INFINITY, f32::min);
            (half + 0.5 - d).clamp(0.0, 1.0)
        });
    }

    /// Draw `s` with its baseline at `y`, starting at `x`; returns its width.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text(
        &mut self,
        font: &mut UiText,
        s: &str,
        weight: Weight,
        px: f32,
        x: f32,
        y: f32,
        color: u32,
    ) -> f32 {
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        let (width, height) = (self.width, self.height);
        let pixels = self.pixels_mut();
        for placed in font.layout(s, weight, px) {
            let g = &placed.glyph;
            for gy in 0..g.height {
                let ty = oy + g.top + gy as i32;
                if ty < 0 || ty as usize >= height {
                    continue;
                }
                for gx in 0..g.width {
                    let tx = ox + placed.x + g.left + gx as i32;
                    if tx < 0 || tx as usize >= width {
                        continue;
                    }
                    let a = f32::from(g.coverage[gy * g.width + gx]) / 255.0;
                    let i = ty as usize * width + tx as usize;
                    pixels[i] = mix(pixels[i], color, a);
                }
            }
        }
        font.width(s, weight, px)
    }
}

/// The pack's icon paths (`build_mux.py` `ICONS`), 16 units square.
pub(crate) mod icons {
    pub(crate) const CHEVRON_DOWN: &str = "M4 6l4 4 4-4";
    pub(crate) const PLUS: &str = "M8 3.5v9M3.5 8h9";
    pub(crate) const SERVER: &str = "M2.5 3.5h11v4h-11zM2.5 8.5h11v4h-11zM5 5.5h.1M5 10.5h.1";
    pub(crate) const CLIENTS: &str = "M6 7.5a2.3 2.3 0 1 0 0-.1M2 13c.4-2.4 2-3.6 4-3.6s3.6 1.2 4 3.6M10.5 4.6a2.2 2.2 0 1 1 .8 4.2M12 9.6c1.3.5 2 1.6 2.2 3.4";
    /// Caption buttons, drawn in the same 16-unit box.
    pub(crate) const MINIMIZE: &str = "M3.5 8h9";
    pub(crate) const MAXIMIZE: &str = "M3.5 3.5h9v9h-9z";
    pub(crate) const CLOSE: &str = "M4 4l8 8M12 4l-8 8";
    /// Block headers (gui-design D47, D48).
    pub(crate) const FOLDER: &str = "M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z";
    pub(crate) const BRANCH: &str =
        "M5 3v10M5 13a1.8 1.8 0 1 0 0 .1M11 3a1.8 1.8 0 1 0 0 .1M11 4.8c0 4-6 3-6 6.2";
    pub(crate) const EYE: &str = "M1.8 8s2.3-4.2 6.2-4.2S14.2 8 14.2 8s-2.3 4.2-6.2 4.2S1.8 8 1.8 8zM8 6.3a1.7 1.7 0 1 0 0 3.4 1.7 1.7 0 0 0 0-3.4z";
    pub(crate) const CHECK: &str = "M3.5 8.5l3 3 6-7";
    pub(crate) const CROSS: &str = "M4.5 4.5l7 7M11.5 4.5l-7 7";
    pub(crate) const PAUSE: &str = "M6 4v8M10 4v8";
    /// The spinner (`gui_lib.py` `spinner`): a ring in `line`, and the
    /// quarter arc on it in the accent.
    pub(crate) const SPINNER_RING: &str = "M3 8a5 5 0 1 0 10 0a5 5 0 1 0-10 0";
    pub(crate) const SPINNER_ARC: &str = "M8 3a5 5 0 0 1 5 5";
}

/// Segments per curve or arc when flattening.
const CURVE_STEPS: usize = 12;

/// The polylines an SVG path draws. Takes the commands the pack's icons
/// use: M L H V Z C S A, absolute and relative.
pub(crate) fn flatten(path: &str) -> Vec<Vec<(f32, f32)>> {
    let mut lines: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut toks = Tokens::new(path);
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = 'M';
    while let Some(next) = toks.command_or_repeat(cmd) {
        cmd = next;
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { (0.0, 0.0) };
        let mut ctrl = None;
        match cmd.to_ascii_uppercase() {
            'M' => {
                let Some(p) = toks.point(base) else { break };
                cur = p;
                start = p;
                lines.push(vec![p]);
                // Further pairs after a moveto are linetos.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let Some(p) = toks.point(base) else { break };
                cur = p;
                push(&mut lines, p);
            }
            'H' => {
                let Some(x) = toks.number() else { break };
                cur = (x + base.0, cur.1);
                push(&mut lines, cur);
            }
            'V' => {
                let Some(y) = toks.number() else { break };
                cur = (cur.0, y + base.1);
                push(&mut lines, cur);
            }
            'Z' => {
                push(&mut lines, start);
                cur = start;
            }
            'C' | 'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&'S') {
                    last_ctrl.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1))
                } else {
                    let Some(c) = toks.point(base) else { break };
                    c
                };
                let (Some(c2), Some(p)) = (toks.point(base), toks.point(base)) else {
                    break;
                };
                for i in 1..=CURVE_STEPS {
                    let t = i as f32 / CURVE_STEPS as f32;
                    let u = 1.0 - t;
                    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                    let x = w[0] * cur.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * p.0;
                    let y = w[0] * cur.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * p.1;
                    push(&mut lines, (x, y));
                }
                ctrl = Some(c2);
                cur = p;
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep)) = (
                    toks.number(),
                    toks.number(),
                    toks.number(),
                    toks.flag(),
                    toks.flag(),
                ) else {
                    break;
                };
                let Some(p) = toks.point(base) else { break };
                for q in arc(cur, p, rx, ry, rot, large, sweep) {
                    push(&mut lines, q);
                }
                cur = p;
            }
            _ => break,
        }
        last_ctrl = ctrl;
    }
    lines
}

/// Extend the current polyline, starting one if a path draws before moving.
fn push(lines: &mut Vec<Vec<(f32, f32)>>, p: (f32, f32)) {
    match lines.last_mut() {
        Some(l) => l.push(p),
        None => lines.push(vec![(0.0, 0.0), p]),
    }
}

/// Points along an SVG elliptical arc from `a` to `b` (SVG 1.1 F.6.5),
/// ending at `b`; a straight line when a radius is zero.
fn arc(
    a: (f32, f32),
    b: (f32, f32),
    rx: f32,
    ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
) -> Vec<(f32, f32)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || a == b {
        return vec![b];
    }
    let (sin, cos) = rot_deg.to_radians().sin_cos();
    let (dx, dy) = ((a.0 - b.0) / 2.0, (a.1 - b.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let scale = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if scale > 1.0 {
        rx *= scale.sqrt();
        ry *= scale.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (
        cos * cx1 - sin * cy1 + (a.0 + b.0) / 2.0,
        sin * cx1 + cos * cy1 + (a.1 + b.1) / 2.0,
    );
    let angle = |ux: f32, uy: f32| uy.atan2(ux);
    let t0 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - t0;
    let tau = std::f32::consts::TAU;
    if sweep && dt < 0.0 {
        dt += tau;
    } else if !sweep && dt > 0.0 {
        dt -= tau;
    }
    let steps = (CURVE_STEPS as f32 * (dt.abs() / tau * 4.0).max(1.0)).ceil() as usize;
    (1..=steps)
        .map(|i| {
            if i == steps {
                return b;
            }
            let t = t0 + dt * i as f32 / steps as f32;
            let (ex, ey) = (rx * t.cos(), ry * t.sin());
            (cos * ex - sin * ey + cx, sin * ex + cos * ey + cy)
        })
        .collect()
}

/// An SVG path's commands and numbers.
struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Tokens<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t') {
            self.i += 1;
        }
    }

    /// The next command letter, or `last` again when a number follows (an
    /// implicit repeat); `None` at the end.
    fn command_or_repeat(&mut self, last: char) -> Option<char> {
        self.skip_separators();
        let &c = self.s.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            return Some(c as char);
        }
        Some(last)
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.i;
        let mut seen_dot = false;
        let mut seen_e = false;
        while let Some(&c) = self.s.get(self.i) {
            let ok = match c {
                b'0'..=b'9' => true,
                b'-' | b'+' => self.i == start || matches!(self.s[self.i - 1], b'e' | b'E'),
                b'.' if !seen_dot && !seen_e => {
                    seen_dot = true;
                    true
                }
                b'e' | b'E' if !seen_e && self.i > start => {
                    seen_e = true;
                    true
                }
                _ => false,
            };
            if !ok {
                break;
            }
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }

    /// An arc flag: a single `0` or `1`, which may run into the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn point(&mut self, base: (f32, f32)) -> Option<(f32, f32)> {
        Some((self.number()? + base.0, self.number()? + base.1))
    }
}

#[cfg(test)]
mod tests {
    use super::{Inner, Pixmap, Rect, box_distance, flatten, icons};

    const BG: u32 = 0x000000;
    const FG: u32 = 0xFFFFFF;

    fn at(p: &Pixmap, x: usize, y: usize) -> u32 {
        p.pixels()[y * p.width + x]
    }

    #[test]
    fn paths_flatten_relative_moves_and_lines() {
        let lines = flatten("M8 3.5v9M3.5 8h9");
        assert_eq!(
            lines,
            vec![vec![(8.0, 3.5), (8.0, 12.5)], vec![(3.5, 8.0), (12.5, 8.0)]]
        );
        let closed = flatten("M2.5 3.5h11v4h-11z");
        assert_eq!(
            closed[0].first(),
            closed[0].last(),
            "z returns to the start"
        );
        assert_eq!(
            flatten("M4 6l4 4 4-4")[0],
            vec![(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]
        );
    }

    #[test]
    fn numbers_may_run_together() {
        assert_eq!(flatten("M5 5.5h.1")[0], vec![(5.0, 5.5), (5.1, 5.5)]);
        assert_eq!(flatten("M1-2l.5.5")[0], vec![(1.0, -2.0), (1.5, -1.5)]);
    }

    #[test]
    fn a_near_full_arc_rings_its_centre() {
        // The clients icon's head: a 0.1 chord from (6, 7.5) up, so the
        // circle of radius 2.3 is centred beside it, at (8.3, 7.45).
        let ring = &flatten(icons::CLIENTS)[0];
        assert!(ring.len() > 20);
        for &(x, y) in ring {
            let r = (x - 8.3).hypot(y - 7.45);
            assert!((r - 2.3).abs() < 0.01, "({x}, {y}) is {r} from the centre");
        }
    }

    #[test]
    fn curves_end_where_they_say() {
        let l = &flatten("M2 13c.4-2.4 2-3.6 4-3.6s3.6 1.2 4 3.6")[0];
        let end = *l.last().expect("points");
        assert!(
            (end.0 - 10.0).abs() < 1e-4 && (end.1 - 13.0).abs() < 1e-4,
            "{end:?}"
        );
    }

    #[test]
    fn every_icon_flattens() {
        for path in [
            icons::CHEVRON_DOWN,
            icons::PLUS,
            icons::SERVER,
            icons::CLIENTS,
            icons::MINIMIZE,
            icons::MAXIMIZE,
            icons::CLOSE,
            icons::FOLDER,
            icons::BRANCH,
            icons::EYE,
            icons::CHECK,
            icons::CROSS,
            icons::PAUSE,
            icons::SPINNER_RING,
            icons::SPINNER_ARC,
        ] {
            let lines = flatten(path);
            assert!(!lines.is_empty(), "{path}");
            assert!(
                lines
                    .iter()
                    .flatten()
                    .all(|&(x, y)| (0.0..=16.0).contains(&x) && (0.0..=16.0).contains(&y))
            );
        }
    }

    #[test]
    fn fill_covers_inside_and_rounds_corners() {
        let mut p = Pixmap::new(20, 20, BG);
        p.fill(
            Rect {
                x: 2.0,
                y: 2.0,
                w: 16.0,
                h: 16.0,
            },
            6.0,
            FG,
        );
        assert_eq!(at(&p, 10, 10), FG);
        assert_eq!(at(&p, 0, 0), BG);
        assert_eq!(at(&p, 2, 2), BG, "the corner is cut by the radius");
        assert_eq!(at(&p, 2, 10), FG, "an edge pixel inside is covered");
    }

    #[test]
    fn border_draws_only_the_edge() {
        let mut p = Pixmap::new(20, 20, BG);
        p.border(
            Rect {
                x: 2.0,
                y: 2.0,
                w: 16.0,
                h: 16.0,
            },
            0.0,
            1.0,
            FG,
        );
        assert_eq!(at(&p, 2, 10), FG);
        assert_eq!(at(&p, 10, 10), BG);
        assert_eq!(at(&p, 17, 10), FG);
    }

    #[test]
    fn icons_stroke_their_lines() {
        let mut p = Pixmap::new(16, 16, BG);
        p.icon(icons::PLUS, 0.0, 0.0, 16.0, 1.5, FG);
        assert_ne!(at(&p, 8, 8), BG, "the cross's centre");
        assert_ne!(
            at(&p, 7, 7),
            BG,
            "a line one px wide and a half covers both sides"
        );
        assert_eq!(at(&p, 1, 1), BG);
    }

    #[test]
    fn a_large_fill_matches_the_per_pixel_shape() {
        let r = Rect {
            x: 3.3,
            y: 2.6,
            w: 40.0,
            h: 30.0,
        };
        let mut fast = Pixmap::new(48, 40, BG);
        fast.fill(r, 9.0, FG);
        fast.border(r, 9.0, 1.5, 0x808080);
        let mut slow = Pixmap::new(48, 40, BG);
        let all = Rect {
            x: 0.0,
            y: 0.0,
            w: 48.0,
            h: 40.0,
        };
        slow.shade(&r, &all, FG, Inner::None, |x, y| {
            (0.5 - box_distance(&r, 9.0, x, y)).clamp(0.0, 1.0)
        });
        slow.shade(&r, &all, 0x808080, Inner::None, |x, y| {
            let d = box_distance(&r, 9.0, x, y);
            ((0.5 - d).clamp(0.0, 1.0) - (0.5 - d - 1.5).clamp(0.0, 1.0)).max(0.0)
        });
        assert_eq!(fast, slow);
    }

    #[test]
    fn fill_part_paints_only_its_part() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        let mut p = Pixmap::new(20, 20, BG);
        p.fill_part(
            r,
            6.0,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 20.0,
                h: 8.0,
            },
            FG,
        );
        assert_eq!(at(&p, 10, 4), FG);
        assert_eq!(at(&p, 10, 12), BG);
        assert_eq!(at(&p, 0, 0), BG, "its corner is still round");
    }

    #[test]
    fn the_fingerprint_follows_changes() {
        let mut p = Pixmap::new(4, 4, BG);
        let before = p.fingerprint();
        assert_eq!(p.fingerprint(), before, "taken again, unchanged");
        p.fill(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 2.0,
                h: 2.0,
            },
            0.0,
            FG,
        );
        assert_ne!(p.fingerprint(), before);
    }

    #[test]
    fn rect_contains_its_area_only() {
        let r = Rect {
            x: 1.0,
            y: 1.0,
            w: 2.0,
            h: 2.0,
        };
        assert!(r.contains(1.0, 2.9));
        assert!(!r.contains(3.0, 1.0));
    }
}
