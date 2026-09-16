//! Vector rendering pipeline.

use crate::color::Color;
use crate::framebuffer::Framebuffer;

/// An axis-aligned clip rectangle constraining subsequent [`Pipeline`] draws
/// to a sub-region of the target framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipRect {
    /// Left edge, in pixels.
    pub x: usize,
    /// Top edge, in pixels.
    pub y: usize,
    /// Width, in pixels.
    pub w: usize,
    /// Height, in pixels.
    pub h: usize,
}

impl ClipRect {
    /// Creates a new clip rectangle.
    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }
}

/// 2D Vector rasterization rendering pipeline.
pub struct Pipeline {
    clip: Option<ClipRect>,
}

impl Pipeline {
    /// Creates a new Pipeline with no clip region set.
    pub fn new() -> Self {
        Self { clip: None }
    }

    /// Constrains subsequent draws to `rect`; pass `None` to clear the clip
    /// and allow drawing across the whole framebuffer again.
    pub fn set_clip(&mut self, rect: Option<ClipRect>) {
        self.clip = rect;
    }

    /// Returns the active clip region, if any.
    pub fn clip(&self) -> Option<ClipRect> {
        self.clip
    }

    /// Intersects a requested `(x, y, w, h)` draw region with the active
    /// clip (if any) and the target framebuffer's bounds, returning the
    /// half-open pixel bounds `(x0, y0, x1, y1)` actually safe to draw, or
    /// `None` if the intersection is empty.
    fn effective_bounds(
        &self,
        target: &Framebuffer,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0) = (x, y);
        let (mut x1, mut y1) = (x.saturating_add(w), y.saturating_add(h));

        if let Some(clip) = self.clip {
            x0 = x0.max(clip.x);
            y0 = y0.max(clip.y);
            x1 = x1.min(clip.x.saturating_add(clip.w));
            y1 = y1.min(clip.y.saturating_add(clip.h));
        }
        x1 = x1.min(target.width());
        y1 = y1.min(target.height());

        if x0 >= x1 || y0 >= y1 {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }

    /// Renders a filled rectangle onto target framebuffer, constrained to
    /// the active clip region (if any).
    pub fn draw_rect(
        &self,
        target: &mut Framebuffer,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
    ) {
        let Some((x0, y0, x1, y1)) = self.effective_bounds(target, x, y, w, h) else {
            return;
        };
        for py in y0..y1 {
            for px in x0..x1 {
                target.set_pixel(px, py, color);
            }
        }
    }

    /// Composites an 8-bit coverage mask — e.g. a rasterized glyph bitmap
    /// from `rusty_font::Rasterizer::rasterize` — at `region`'s position
    /// using `color` as the foreground. Each covered pixel is alpha-blended
    /// (source-over) against the framebuffer's existing contents rather
    /// than overwritten, combining the coverage byte with `color`'s own
    /// alpha channel. Constrained to the active clip region (if any).
    ///
    /// `coverage` must have exactly `region.w * region.h` bytes, row-major,
    /// one byte per pixel (`0` = fully transparent, `255` = fully covered).
    pub fn blit_coverage(
        &self,
        target: &mut Framebuffer,
        region: ClipRect,
        coverage: &[u8],
        color: Color,
    ) {
        let ClipRect { x, y, w, h } = region;
        assert_eq!(coverage.len(), w * h, "coverage buffer must be w * h bytes");
        let Some((x0, y0, x1, y1)) = self.effective_bounds(target, x, y, w, h) else {
            return;
        };
        for py in y0..y1 {
            let src_row = py - y;
            for px in x0..x1 {
                let src_col = px - x;
                let mask = coverage[src_row * w + src_col];
                if mask == 0 {
                    continue;
                }
                let alpha = ((mask as u32 * color.a as u32) / 255) as u8;
                if alpha == 0 {
                    continue;
                }
                let dst = target.get_pixel(px, py).unwrap_or_default();
                target.set_pixel(px, py, blend_over(color, alpha, dst));
            }
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Source-over alpha compositing of `src` (using `alpha` in place of
/// `src.a`) onto `dst`.
fn blend_over(src: Color, alpha: u8, dst: Color) -> Color {
    let a = alpha as u32;
    let inv_a = 255 - a;
    let blend = |s: u8, d: u8| ((s as u32 * a + d as u32 * inv_a) / 255) as u8;
    Color::rgba(
        blend(src.r, dst.r),
        blend(src.g, dst.g),
        blend(src.b, dst.b),
        (a + (dst.a as u32 * inv_a) / 255) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_rect_fills_the_requested_area() {
        let mut fb = Framebuffer::new(4, 4);
        let pipeline = Pipeline::new();
        pipeline.draw_rect(&mut fb, 1, 1, 2, 2, Color::rgb(255, 0, 0));
        assert_eq!(fb.get_pixel(1, 1), Some(Color::rgb(255, 0, 0)));
        assert_eq!(fb.get_pixel(2, 2), Some(Color::rgb(255, 0, 0)));
        assert_eq!(fb.get_pixel(0, 0), Some(Color::default()));
        assert_eq!(fb.get_pixel(3, 3), Some(Color::default()));
    }

    #[test]
    fn draw_rect_is_constrained_by_the_active_clip() {
        let mut fb = Framebuffer::new(4, 4);
        let mut pipeline = Pipeline::new();
        pipeline.set_clip(Some(ClipRect::new(2, 0, 2, 4)));
        pipeline.draw_rect(&mut fb, 0, 0, 4, 4, Color::rgb(255, 0, 0));
        // Left of the clip: untouched.
        assert_eq!(fb.get_pixel(0, 0), Some(Color::default()));
        assert_eq!(fb.get_pixel(1, 3), Some(Color::default()));
        // Inside the clip: filled.
        assert_eq!(fb.get_pixel(2, 0), Some(Color::rgb(255, 0, 0)));
        assert_eq!(fb.get_pixel(3, 3), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn blit_coverage_full_coverage_matches_opaque_color() {
        let mut fb = Framebuffer::new(2, 1);
        let pipeline = Pipeline::new();
        pipeline.blit_coverage(
            &mut fb,
            ClipRect::new(0, 0, 2, 1),
            &[255, 255],
            Color::rgb(10, 20, 30),
        );
        assert_eq!(fb.get_pixel(0, 0), Some(Color::rgb(10, 20, 30)));
        assert_eq!(fb.get_pixel(1, 0), Some(Color::rgb(10, 20, 30)));
    }

    #[test]
    fn blit_coverage_zero_coverage_leaves_destination_untouched() {
        let mut fb = Framebuffer::new(1, 1);
        fb.clear(Color::rgb(9, 9, 9));
        let pipeline = Pipeline::new();
        pipeline.blit_coverage(
            &mut fb,
            ClipRect::new(0, 0, 1, 1),
            &[0],
            Color::rgb(255, 0, 0),
        );
        assert_eq!(fb.get_pixel(0, 0), Some(Color::rgb(9, 9, 9)));
    }

    #[test]
    fn blit_coverage_partial_coverage_blends_toward_destination() {
        let mut fb = Framebuffer::new(1, 1);
        fb.clear(Color::rgb(0, 0, 0));
        let pipeline = Pipeline::new();
        // Half coverage of pure white onto black should land roughly in the middle.
        pipeline.blit_coverage(
            &mut fb,
            ClipRect::new(0, 0, 1, 1),
            &[128],
            Color::rgb(255, 255, 255),
        );
        let blended = fb.get_pixel(0, 0).unwrap_or_default();
        assert!(blended.r > 0 && blended.r < 255, "got r={}", blended.r);
    }

    #[test]
    fn blit_coverage_is_constrained_by_the_active_clip() {
        let mut fb = Framebuffer::new(2, 1);
        let mut pipeline = Pipeline::new();
        pipeline.set_clip(Some(ClipRect::new(0, 0, 1, 1)));
        pipeline.blit_coverage(
            &mut fb,
            ClipRect::new(0, 0, 2, 1),
            &[255, 255],
            Color::rgb(255, 0, 0),
        );
        assert_eq!(fb.get_pixel(0, 0), Some(Color::rgb(255, 0, 0)));
        assert_eq!(fb.get_pixel(1, 0), Some(Color::default()));
    }

    #[test]
    #[should_panic(expected = "coverage buffer must be w * h bytes")]
    fn blit_coverage_panics_on_mismatched_buffer_length() {
        let mut fb = Framebuffer::new(2, 2);
        let pipeline = Pipeline::new();
        pipeline.blit_coverage(
            &mut fb,
            ClipRect::new(0, 0, 2, 2),
            &[255],
            Color::rgb(255, 0, 0),
        );
    }
}
