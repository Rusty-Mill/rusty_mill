//! Vector rendering pipeline.

use crate::color::Color;
use crate::framebuffer::Framebuffer;

/// 2D Vector rasterization rendering pipeline.
pub struct Pipeline;

impl Pipeline {
    /// Creates a new Pipeline.
    pub fn new() -> Self {
        Self
    }

    /// Renders a filled rectangle onto target framebuffer.
    pub fn draw_rect(
        &self,
        target: &mut Framebuffer,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
    ) {
        for dy in 0..h {
            for dx in 0..w {
                target.set_pixel(x + dx, y + dy, color);
            }
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}
