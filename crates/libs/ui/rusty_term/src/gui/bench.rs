//! Headless frame driver for the `bench_frame_time` binary: the windowed
//! front-end's real CPU frame path ([`render::composite`]) painting one pane
//! into an offscreen buffer, with no window, surface or present.
//!
//! [`render::composite`]: super::render::composite

use crate::core::Grid;

use super::font::{self, FontCache, GlyphSource};
use super::render::{self, PaneFrame};

/// An offscreen frame target with a loaded font.
pub struct FrameBench {
    font: FontCache,
    buf: Vec<u32>,
    width: usize,
    height: usize,
}

impl FrameBench {
    /// Load the default monospace font (`$RUSTY_TERM_FONT` or a system
    /// font, as the window does) at `px` pixels and size the buffer for a
    /// `cols`×`rows` pane. `None` when no usable font is found.
    pub fn new(cols: usize, rows: usize, px: f32) -> Option<Self> {
        let set = font::load_set(None, None, None, None, None)?;
        let font = FontCache::new(set, px, false)?;
        let (cw, ch) = font.cell_size();
        let (width, height) = (cols * cw, rows * ch);
        Some(FrameBench {
            font,
            buf: vec![0; width * height],
            width,
            height,
        })
    }

    /// Paint one frame of `grid` as the focused pane, computing its
    /// command-gutter marks the way the window does each frame.
    pub fn draw(&mut self, grid: &Grid) {
        let marks = grid
            .viewport_block_marks()
            .into_iter()
            .enumerate()
            .filter_map(|(row, m)| m.map(|_| (row, 0x00ff00)))
            .collect();
        let pane = PaneFrame {
            grid,
            col0: 0,
            row0: 0,
            focused: true,
            cursor_on: true,
            trail: Vec::new(),
            hover_link: None,
            marks,
        };
        render::composite(
            &mut self.buf,
            &[pane],
            &[],
            &[],
            &mut self.font,
            self.width,
            self.height,
            0x202020,
            0x000000,
            0x101010,
            0,
        );
    }
}
