//! Software Framebuffer memory buffer and presentation target.

use crate::color::Color;
use alloc::vec;
use alloc::vec::Vec;

/// A 2D pixel memory framebuffer.
pub struct Framebuffer {
    width: usize,
    height: usize,
    buffer: Vec<u32>,
}

impl Framebuffer {
    /// Creates a new Framebuffer with width and height.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            buffer: vec![0u32; width * height],
        }
    }

    /// Returns width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Clears the entire buffer to a solid color.
    pub fn clear(&mut self, color: Color) {
        let pixel = color.to_u32();
        self.buffer.fill(pixel);
    }

    /// Sets a pixel at (x, y) coordinates to color.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x] = color.to_u32();
        }
    }

    /// Presents/blits this framebuffer to an OS window.
    pub fn present(&self, _window: &rusty_gui::Window) {
        #[cfg(windows)]
        unsafe {
            rusty_win32::windowing::blit_pixel_buffer(
                _window.raw_handle(),
                self.width,
                self.height,
                &self.buffer,
            );
        }
    }

    /// Borrows the raw 32-bit pixel buffer slice.
    pub fn as_slice(&self) -> &[u32] {
        &self.buffer
    }
}
