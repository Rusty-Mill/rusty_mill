//! Headless frame driver for the `bench_frame_time` binary: the windowed
//! front-end's real CPU frame path ([`render::composite`]) painting one pane
//! into an offscreen buffer, with no window, surface or present. Like the
//! window on a backend that keeps its buffer, it repaints only the rows that
//! changed since the last frame, unless built [`full_repaint`].
//!
//! [`full_repaint`]: FrameBench::full_repaint
//!
//! [`render::composite`]: super::render::composite

use crate::core::FrameView;

use super::damage::{Damage, Frame, Plan};
use super::font::{self, FontCache, GlyphSource};
use super::render::{self, PaneFrame};

/// An offscreen frame target with a loaded font.
pub struct FrameBench {
    font: FontCache,
    buf: Vec<u32>,
    width: usize,
    height: usize,
    /// `None` repaints every frame in full.
    damage: Option<Damage>,
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
            damage: Some(Damage::default()),
        })
    }

    /// Repaint every frame in full, as before damage tracking.
    pub fn full_repaint(mut self) -> Self {
        self.damage = None;
        self
    }

    /// Paint one frame snapshot as the focused pane, with its command-gutter
    /// marks as the window builds them.
    pub fn draw(&mut self, grid: &FrameView) {
        let marks = grid
            .block_marks()
            .iter()
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
        let colors = [0x202020, 0x000000, 0x101010];
        let panes = [pane];
        let size = (self.width, self.height);
        let frame = Frame::capture(&panes, &[], &[], self.font.id(), size, colors, 0);
        // The buffer persists, so it always holds the last frame (age 1).
        let plan = self
            .damage
            .as_ref()
            .map_or(Plan::Full, |d| d.plan(&frame, 1));
        render::composite(
            &mut self.buf,
            &panes,
            &[],
            &[],
            &mut self.font,
            self.width,
            self.height,
            colors[0],
            colors[1],
            colors[2],
            0,
            plan.rows(),
        );
        if let Some(d) = self.damage.as_mut() {
            d.presented(frame);
        }
    }
}
