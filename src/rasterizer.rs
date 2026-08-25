//! A real scanline glyph rasterizer: flattens TrueType's on/off-curve
//! quadratic Bézier contours into line segments, then fills them with the
//! non-zero winding rule (the rule TrueType itself specifies — needed for
//! glyphs with a "hole," like `O`, whose counter must stay unfilled).

use crate::glyph::GlyphOutline;
use alloc::vec;
use alloc::vec::Vec;

/// Glyph scanline rasterizer engine.
pub struct Rasterizer {
    width: usize,
    height: usize,
}

/// A single scanline-fillable edge between two points, direction-tagged
/// (+1 if it goes downward in y, -1 upward) for the non-zero winding rule.
/// Horizontal edges contribute no crossings and are never constructed.
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Edge {
    /// The x-coordinate and winding direction where this edge crosses
    /// horizontal line `y`, or `None` if `y` isn't within this edge's span
    /// (half-open at the top so a shared vertex is never double-counted).
    fn intersect(&self, y: f32) -> Option<(f32, i32)> {
        let (y_min, y_max, dir) = if self.y0 < self.y1 { (self.y0, self.y1, 1) } else { (self.y1, self.y0, -1) };
        if y < y_min || y >= y_max {
            return None;
        }
        let t = (y - self.y0) / (self.y1 - self.y0);
        Some((self.x0 + t * (self.x1 - self.x0), dir))
    }
}

/// Samples a quadratic Bézier curve (`p0` on-curve, `p1` the off-curve
/// control point, `p2` on-curve) into `segments` line segments, pushing
/// every point after `p0` (the caller already has `p0` from the previous
/// point in the contour).
fn flatten_quad(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), segments: usize, out: &mut Vec<(f32, f32)>) {
    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        let mt = 1.0 - t;
        let x = mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0;
        let y = mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1;
        out.push((x, y));
    }
}

/// Walks one contour's on/off-curve points (TrueType's encoding: two
/// consecutive off-curve points imply an on-curve point at their
/// midpoint) into a flattened polyline of `(x, y)` pixel-space points,
/// closing back to the start.
fn flatten_contour(points: &[crate::glyph::Point], scale: f32) -> Vec<(f32, f32)> {
    let n = points.len();
    if n == 0 {
        return Vec::new();
    }
    let pt = |i: usize| {
        let p = &points[i % n];
        (p.x * scale, p.y * scale)
    };
    let on = |i: usize| points[i % n].on_curve;

    let start_idx = (0..n).find(|&i| on(i));
    let (start_point, mut i) = match start_idx {
        Some(idx) => (pt(idx), idx),
        // All-off-curve contour (rare, but legal -- e.g. some rounded
        // glyphs): synthesize a start at the midpoint of the first two.
        None => {
            let a = pt(0);
            let b = pt(1);
            (((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), 0)
        }
    };

    let mut result = Vec::with_capacity(n * 2);
    result.push(start_point);
    let mut current = start_point;
    let mut steps_taken = 0usize;

    while steps_taken < n {
        i += 1;
        steps_taken += 1;
        let idx = i % n;
        if on(idx) {
            let p = pt(idx);
            result.push(p);
            current = p;
        } else {
            let control = pt(idx);
            let next_idx = i + 1;
            let end = if on(next_idx % n) {
                i += 1;
                steps_taken += 1;
                pt(next_idx % n)
            } else {
                let nxt = pt(next_idx % n);
                ((control.0 + nxt.0) / 2.0, (control.1 + nxt.1) / 2.0)
            };
            flatten_quad(current, control, end, 8, &mut result);
            current = end;
        }
    }
    result
}

fn build_edges(outline: &GlyphOutline, scale: f32) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut start = 0usize;
    for &end in &outline.contour_ends {
        let contour_points = &outline.points[start..=end.min(outline.points.len().saturating_sub(1))];
        let polyline = flatten_contour(contour_points, scale);
        for w in 0..polyline.len() {
            let (x0, y0) = polyline[w];
            let (x1, y1) = polyline[(w + 1) % polyline.len()];
            if y0 != y1 {
                edges.push(Edge { x0, y0, x1, y1 });
            }
        }
        start = end + 1;
    }
    edges
}

impl Rasterizer {
    /// Creates a rasterizer with target pixel dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Rasterizes a glyph outline into a 1-byte-per-pixel antialiased
    /// coverage map, scaling from font units to pixels by `scale`
    /// (typically `pixel_size / units_per_em`). A real scanline fill
    /// (non-zero winding), not a fixed test pattern — glyphs with a
    /// counter (`O`, `A`, ...) come out with a real hole, verified in this
    /// module's tests against real system fonts.
    ///
    /// Antialiasing is `Y_SUBSAMPLES` sub-scanlines per pixel row, each
    /// with exact fractional pixel coverage in X (no sub-sampling error
    /// along the scan direction — a span's start/end pixel gets exactly
    /// its overlapped fraction, not a rounded in/out decision). A pixel
    /// fully inside every sub-scanline's span is `255`; a boundary pixel
    /// is graded, not a hard edge.
    pub fn rasterize(&self, outline: &GlyphOutline, scale: f32) -> Vec<u8> {
        const Y_SUBSAMPLES: usize = 4;
        let edges = build_edges(outline, scale);
        let mut coverage = vec![0f32; self.width * self.height];
        if edges.is_empty() {
            return vec![0u8; self.width * self.height];
        }

        let weight = 1.0 / Y_SUBSAMPLES as f32;
        for y in 0..self.height {
            let row = &mut coverage[y * self.width..(y + 1) * self.width];
            for s in 0..Y_SUBSAMPLES {
                let scan_y = y as f32 + (s as f32 + 0.5) / Y_SUBSAMPLES as f32;
                let mut crossings: Vec<(f32, i32)> =
                    edges.iter().filter_map(|e| e.intersect(scan_y)).collect();
                crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));

                let mut winding = 0i32;
                let mut span_start: Option<f32> = None;
                for (x, dir) in crossings {
                    let was_inside = winding != 0;
                    winding += dir;
                    let is_inside = winding != 0;
                    if !was_inside && is_inside {
                        span_start = Some(x);
                    } else if was_inside && !is_inside {
                        if let Some(sx) = span_start.take() {
                            accumulate_span(row, self.width, sx, x, weight);
                        }
                    }
                }
            }
        }
        coverage
            .iter()
            .map(|&c| round_nonneg(c.clamp(0.0, 1.0) * 255.0).min(255) as u8)
            .collect()
    }
}

/// Rounds a non-negative `f32` to the nearest integer without `f32::round`
/// (unavailable in `core` — needs `libm`): adding `0.5` then truncating via
/// `as u32` is round-half-up, which is what's wanted here anyway.
fn round_nonneg(x: f32) -> u32 {
    (x.max(0.0) + 0.5) as u32
}

/// Ceiling of a non-negative `f32` without `f32::ceil` (unavailable in
/// `core`): truncation already gives the floor, so bump by one when the
/// value wasn't already an exact integer.
fn ceil_nonneg(x: f32) -> usize {
    let floor = x as usize;
    if floor as f32 == x { floor } else { floor + 1 }
}

/// Adds `weight` coverage to `row` for the pixel span `[x0, x1)`: a pixel
/// fully inside the span gets the full `weight`; a pixel the span only
/// partially overlaps gets `weight` scaled by the overlapped fraction.
/// This is what makes glyph edges graded instead of a hard on/off cutoff.
fn accumulate_span(row: &mut [f32], width: usize, x0: f32, x1: f32, weight: f32) {
    let x0 = x0.max(0.0);
    let x1 = x1.min(width as f32);
    if x0 >= x1 {
        return;
    }
    let start_px = x0 as usize; // truncation == floor for x0 >= 0
    let end_px = ceil_nonneg(x1).min(width);
    if start_px >= width {
        return;
    }
    if start_px + 1 >= end_px {
        // The whole span lands within one pixel.
        row[start_px] += weight * (x1 - x0);
        return;
    }
    row[start_px] += weight * ((start_px + 1) as f32 - x0);
    for px in &mut row[start_px + 1..end_px - 1] {
        *px += weight;
    }
    let last = end_px - 1;
    row[last] += weight * (x1 - last as f32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ttf::Font;

    fn load_system_font(name: &str) -> Option<Vec<u8>> {
        std::fs::read(alloc::format!("C:\\Windows\\Fonts\\{name}")).ok()
    }

    fn rasterize_char(font: &Font, ch: char, px: usize) -> Vec<u8> {
        let glyph_id = font.glyph_index(ch).unwrap();
        let outline = font.glyph_outline(glyph_id).unwrap();
        let scale = px as f32 / font.units_per_em() as f32;
        Rasterizer::new(px, px).rasterize(&outline, scale)
    }

    /// Approximate filled-area in "full pixel" units: summing coverage
    /// bytes and dividing by 255 turns graded (antialiased) edge pixels
    /// into a fractional contribution, so this stays a meaningful area
    /// proxy under AA instead of only counting the now-rare exactly-255
    /// (fully covered, no edge) pixels.
    fn coverage(buffer: &[u8]) -> usize {
        buffer.iter().map(|&b| b as usize).sum::<usize>() / 255
    }

    #[test]
    fn rasterizing_a_letter_produces_a_real_partial_fill_not_a_fixed_test_pattern() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let buffer = rasterize_char(&font, 'I', 32);
        let filled = coverage(&buffer);
        assert!(filled > 0, "'I' should rasterize to some filled pixels");
        // The old stub always filled every pixel except a 2px border,
        // i.e. (32-4)*(32-4) = 784 out of 1024 -- a real glyph like 'I'
        // (a comparatively thin vertical bar) should cover far less.
        assert!(filled < 400, "a thin glyph like 'I' shouldn't cover most of the 32x32 box (got {filled})");
    }

    #[test]
    fn the_letter_o_has_a_real_unfilled_hole_in_its_center() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let size = 64;
        let buffer = rasterize_char(&font, 'O', size);

        // Center of the glyph box: 'O's counter should be unfilled --
        // proof the non-zero winding rule (and the outer+inner contour
        // structure it depends on) is genuinely working, not just "some
        // pixels are on".
        let center = (size / 2) * size + (size / 2);
        assert_eq!(buffer[center], 0, "the center of 'O' should be its unfilled counter (hole)");

        // But 'O' isn't empty overall -- the ring itself must be filled.
        assert!(coverage(&buffer) > 0, "'O' should have a filled ring");
    }

    #[test]
    fn space_glyph_rasterizes_to_a_fully_empty_buffer() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let buffer = rasterize_char(&font, ' ', 16);
        assert_eq!(coverage(&buffer), 0, "space has no outline, so nothing should be filled");
    }

    #[test]
    fn different_letters_rasterize_to_different_coverage() {
        let Some(bytes) = load_system_font("arial.ttf") else {
            eprintln!("skipping: arial.ttf not found on this machine");
            return;
        };
        let font = Font::parse(&bytes).unwrap();
        let i_coverage = coverage(&rasterize_char(&font, 'I', 32));
        let m_coverage = coverage(&rasterize_char(&font, 'M', 32));
        // 'M' has substantially more ink than 'I' at the same size.
        assert!(m_coverage > i_coverage * 2, "'M' ({m_coverage}px) should cover much more than 'I' ({i_coverage}px)");
    }
}
