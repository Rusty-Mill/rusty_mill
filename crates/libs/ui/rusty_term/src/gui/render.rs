//! Renderer abstraction: the windowed front-end paints through a [`Renderer`].
//!
//! The CPU implementation here presents via `softbuffer`; the GPU one
//! ([`super::gpu`], behind `gui-gpu`) presents via `wgpu`. Both consume the same
//! [`FontCache`] and each pane's [`FrameView`] snapshot, so the window code is backend-agnostic.

use std::num::NonZeroU32;
use std::sync::Arc;

use winit::window::Window;

use crate::core::{
    ATTR_DIM, ATTR_HIDDEN, ATTR_OVERLINE, ATTR_REVERSE, ATTR_STRIKE, ATTR_UNDERLINE, BidiRow, Cell,
    FrameView, char_width,
};

use super::cpu;
use super::damage::{Damage, Frame};
use super::font::{FontCache, GlyphSource};

/// Pixels of strip-colored band above the chrome bar's tabs, so a tab's top
/// edge is visibly distinct from the window's (the tabs otherwise run flush
/// into the frame and read as sliced off). Bounded by the window padding so
/// the pushed-down bar can never overlap the grid, which starts a full bar
/// height plus `pad` below the top.
pub(super) fn bar_inset(pad: usize, bar_h: usize) -> usize {
    pad.min(bar_h / 6)
}

/// Pixel row where the pane grid starts: below the padding and, when there
/// is one, the chrome bar (`bar_h` tall, the UI font's cell height) and its
/// [`bar_inset`]. Pane rows count from 0 at this edge.
pub(super) fn grid_top(chrome: &[Cell], pad: usize, bar_h: usize) -> usize {
    if chrome.is_empty() {
        pad
    } else {
        pad + bar_inset(pad, bar_h) + bar_h
    }
}

/// Search-match highlight, shared by both renderers: amber for a match, orange
/// for the active one, with a dark glyph so text stays legible on either.
pub(super) const SEARCH_BG: u32 = 0xFFD24A;
pub(super) const SEARCH_CUR_BG: u32 = 0xFF7A1A;
pub(super) const SEARCH_FG: u32 = 0x101010;

/// How much of a dim (SGR 2) cell's foreground survives its fade toward the
/// background: two thirds.
const DIM_KEEP: u8 = 170;

/// A grid cell as the window draws it, shared by both renderers. SGR 7 swaps
/// fg and bg, SGR 2 then fades the fg toward the bg, and SGR 8 drops the
/// cell's lines; its glyph is dropped by [`glyph_char`], so the cell keeps
/// its character and width (a concealed wide glyph still covers two
/// columns). The grid keeps the attributes (the TUI and the mux re-emit
/// them), so the window resolves them here; cursor, selection and search
/// colours apply on top.
pub(super) fn drawn(mut cell: Cell) -> Cell {
    if cell.flags & ATTR_REVERSE != 0 {
        std::mem::swap(&mut cell.fg, &mut cell.bg);
    }
    if cell.flags & ATTR_DIM != 0 {
        cell.fg = blend(cell.bg, cell.fg, DIM_KEEP);
    }
    if concealed(&cell) {
        cell.flags &= !(ATTR_UNDERLINE | ATTR_STRIKE | ATTR_OVERLINE);
    }
    cell
}

/// Whether `ch`, in a cell's grapheme continuation, is a mark drawn over the
/// base glyph: zero width, but not a joiner or variation selector (those
/// only steer which glyph a font picks and draw nothing themselves).
pub(super) fn is_overlay_mark(ch: char) -> bool {
    char_width(ch) == 0
        && ch != '\u{200d}'
        && !('\u{fe00}'..='\u{fe0f}').contains(&ch)
        && !('\u{e0100}'..='\u{e01ef}').contains(&ch)
}

/// Whether `cell` is concealed (SGR 8): drawn as its background only.
pub(super) fn concealed(cell: &Cell) -> bool {
    cell.flags & ATTR_HIDDEN != 0
}

/// The character the cell at logical column `col` draws: a space for a
/// concealed cell or a Kitty placeholder (which draws image content), else
/// its character after the row's Arabic contextual form (`bidi.shaped`) and
/// RTL mirroring. Concealment wins over shaping, which is computed from the
/// stored characters.
pub(super) fn glyph_char(cell: &Cell, bidi: Option<&BidiRow>, col: usize) -> char {
    if concealed(cell) || cell.ch == '\u{10EEEE}' {
        return ' ';
    }
    let Some(b) = bidi else { return cell.ch };
    let ch = b.shaped.as_ref().and_then(|s| s[col]).unwrap_or(cell.ch);
    if b.rtl[col] {
        crate::core::bidi_mirrored(ch).unwrap_or(ch)
    } else {
        ch
    }
}

/// `fg` over `bg` at coverage `a` (`0` is all `bg`, `255` all `fg`), per
/// channel of `0xRRGGBB`.
pub(super) fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    let a = a as u32;
    let inv = 255 - a;
    let chan = |shift: u32| {
        let f = (fg >> shift) & 0xff;
        let b = (bg >> shift) & 0xff;
        (f * a + b * inv) / 255
    };
    (chan(16) << 16) | (chan(8) << 8) | chan(0)
}

/// One pane to paint this frame: its `grid` at cell offset `(col0, row0)`,
/// sized to the grid's own `cols × rows`. `focused` gates the cursor/IME
/// preedit (only the focused pane shows them); `cursor_on` is the blink phase.
pub(crate) struct PaneFrame<'a> {
    pub grid: &'a FrameView,
    pub col0: usize,
    pub row0: usize,
    pub focused: bool,
    pub cursor_on: bool,
    /// Cursor-trail ghosts (G36): faded cursor blocks at these cells, alpha
    /// `0.0..=1.0`, blended in the cursor color. Empty when the trail is off
    /// or expired.
    pub trail: Vec<(usize, usize, f32)>,
    /// A Ctrl-hovered hyperlink's `(row, start_col, end_col)` (inclusive),
    /// underlined for the click affordance — `None` when nothing's hovered
    /// or Ctrl isn't held. Cell coordinates, in this pane's own frame.
    pub hover_link: Option<(usize, usize, usize)>,
    /// Command gutter marks: `(viewport row, color)` for rows inside an
    /// OSC 133 command block, painted as a thin stripe just left of the
    /// pane's text (green success / red failure / accent while running).
    /// Empty when the feature is off or nothing is marked.
    pub marks: Vec<(usize, u32)>,
}

/// A present target: paint one frame of the tab's `panes` at the given pixel
/// size. `chrome` is the window's own top bar (tabs + caption buttons) as one
/// pre-laid cell row at the top; `status` is the bottom status ribbon (cwd,
/// git branch, exit pill, …) as one pre-laid cell row flush with the window's
/// bottom edge (empty when disabled). Both bars draw in `ui`, the window's
/// UI font, which the Ctrl+=/Ctrl+- zoom leaves alone; panes draw in `font`; `divider` fills the gaps between panes;
/// `bg` fills everything else — the `pad`-pixel band inset around the pane
/// area (`[window] padding`) and any right/bottom slack, so content reads as
/// floating on the terminal background. Both bars stay flush.
pub(crate) trait Renderer {
    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        panes: &[PaneFrame],
        chrome: &[Cell],
        status: &[Cell],
        font: &mut FontCache,
        ui: &mut FontCache,
        width: u32,
        height: u32,
        divider: u32,
        bg: u32,
        bar_bg: u32,
        pad: usize,
    );

    /// Set the window background opacity (`[window] opacity`), `0.0`-`1.0`.
    /// A no-op by default: the CPU (`softbuffer`) presentation path has no
    /// alpha channel to composite through, so only the GPU renderer overrides
    /// this — see `GpuRenderer`'s impl and `GpuCore::alpha_mode`.
    fn set_opacity(&mut self, _opacity: f32) {}

    /// Whether [`set_opacity`](Self::set_opacity) actually does anything.
    /// The settings page uses this to render the opacity row as disabled
    /// (instead of silently inert) when the CPU renderer is presenting.
    fn supports_opacity(&self) -> bool {
        false
    }

    /// Whether presenting already waits for the display refresh (vsync), so
    /// the window need not pace output repaints itself (see `pace`).
    fn waits_for_vsync(&self) -> bool {
        false
    }
}

/// CPU compositor presented through `softbuffer`. It repaints only the pane
/// rows that changed since the frame whose pixels the buffer still holds.
pub(crate) struct CpuRenderer {
    // The context must outlive the surface, so it is held alongside it.
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    damage: Damage,
}

impl CpuRenderer {
    pub(crate) fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        let context = softbuffer::Context::new(window.clone())?;
        let surface = softbuffer::Surface::new(&context, window)?;
        Ok(CpuRenderer {
            _context: context,
            surface,
            damage: Damage::default(),
        })
    }
}

impl Renderer for CpuRenderer {
    fn render(
        &mut self,
        panes: &[PaneFrame],
        chrome: &[Cell],
        status: &[Cell],
        font: &mut FontCache,
        ui: &mut FontCache,
        width: u32,
        height: u32,
        divider: u32,
        bg: u32,
        bar_bg: u32,
        pad: usize,
    ) {
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        if self.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return;
        };
        let (w, h) = (width as usize, height as usize);
        let frame = Frame::capture(
            panes,
            chrome,
            status,
            [font.id(), ui.id()],
            (w, h),
            [divider, bg, bar_bg],
            pad,
        );
        let plan = self.damage.plan(&frame, buffer.age());
        composite(
            &mut buffer,
            panes,
            chrome,
            status,
            font,
            ui,
            w,
            h,
            divider,
            bg,
            bar_bg,
            pad,
            plan.rows(),
        );
        // Present every pixel, not just the repainted rows: the buffer holds
        // the whole frame either way, and the window may have lost pixels
        // the rows don't cover. Moving it off screen and back, or resizing
        // it, asks for a repaint with no row changed, and a damage-only
        // present then leaves that area white or stale (#1387).
        let presented = buffer.present();
        // A failed present leaves the buffer's age unknown to us.
        match presented {
            Ok(()) => self.damage.presented(frame),
            Err(_) => self.damage.forget(),
        }
    }
}

/// Paint one frame into `buffer` (`width × height` pixels): everything
/// [`Renderer::render`] does except presenting. Pure, so the frame-time
/// bench drives the real frame path without a window.
///
/// `repaint` (per pane, per row; `None` repaints all) paints only those pane
/// rows over a buffer that already holds a frame with the same layout: the
/// padding, dividers, bars and marks are left as they are.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite(
    buffer: &mut [u32],
    panes: &[PaneFrame],
    chrome: &[Cell],
    status: &[Cell],
    font: &mut dyn GlyphSource,
    ui: &mut dyn GlyphSource,
    w: usize,
    h: usize,
    divider: u32,
    bg: u32,
    bar_bg: u32,
    pad: usize,
    repaint: Option<&[Vec<bool>]>,
) {
    let (cw, ch) = font.cell_size();
    let (bw, bh) = ui.cell_size();
    // The chrome bar sits `inset` px below the window's top edge (the
    // strip band shows above it); the grid starts below the bar plus the
    // pad-sized gap — the bottom pad absorbs the difference.
    let oy = grid_top(chrome, pad, bh);
    if let Some(rows) = repaint {
        for (p, rows) in panes.iter().zip(rows) {
            cpu::draw_grid(
                buffer,
                w,
                h,
                p.grid,
                p.col0,
                p.row0,
                pad,
                oy,
                p.focused,
                p.cursor_on,
                p.hover_link,
                font,
                Some(rows),
            );
            let trail = Some(rows.as_slice());
            cpu::draw_trail(
                buffer, w, h, p.grid, p.col0, p.row0, pad, oy, &p.trail, font, trail,
            );
        }
        return;
    }
    buffer.fill(bg); // the padding band (and any slack) is background
    let inset = bar_inset(pad, bh);
    // Gaps between panes show the divider: fill the panes' bounding box
    // with it, then let the grids overpaint everything but the gaps.
    if let (Some(x1), Some(y1)) = (
        panes.iter().map(|p| p.col0 + p.grid.cols).max(),
        panes.iter().map(|p| p.row0 + p.grid.rows).max(),
    ) {
        let y0 = panes.iter().map(|p| p.row0).min().unwrap_or(0) * ch + oy;
        for y in y0..(y1 * ch + oy).min(h) {
            let base = y * w;
            for x in pad..(x1 * cw + pad).min(w) {
                buffer[base + x] = divider;
            }
        }
    }
    if !chrome.is_empty() {
        cpu::draw_chrome(buffer, w, h, chrome, ui, bw, bh, inset, bar_bg);
    }
    // The status ribbon sits flush with the window's bottom edge; the
    // grid was sized to stop above it, so it never paints under it.
    if !status.is_empty() && h >= bh {
        cpu::draw_bar(buffer, w, h, status, ui, bw, bh, h - bh);
    }
    for p in panes {
        cpu::draw_grid(
            buffer,
            w,
            h,
            p.grid,
            p.col0,
            p.row0,
            pad,
            oy,
            p.focused,
            p.cursor_on,
            p.hover_link,
            font,
            None,
        );
        cpu::draw_trail(
            buffer, w, h, p.grid, p.col0, p.row0, pad, oy, &p.trail, font, None,
        );
        cpu::draw_marks(buffer, w, h, p.col0, p.row0, pad, oy, &p.marks, font);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{AnsiParser, Grid};
    use std::rc::Rc;

    use super::super::font::{Glyph, Style};

    /// A font of inkless glyphs at a fixed cell size, so a frame shows
    /// only cell backgrounds.
    struct Blank((usize, usize));

    impl GlyphSource for Blank {
        fn cell_size(&self) -> (usize, usize) {
            self.0
        }
        fn baseline(&self) -> i32 {
            0
        }
        fn glyph(&mut self, _ch: char, _style: Style) -> Rc<Glyph> {
            Rc::new(Glyph {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                coverage: Vec::new(),
                color: None,
            })
        }
    }

    /// The bars lay out on the UI font's cell and the grid on its own, so a
    /// zoomed grid font leaves the chrome and status bars their size (#1374
    /// zoom scaled the whole window).
    #[test]
    fn bars_keep_the_ui_font_size_when_the_grid_font_differs() {
        const PAD: usize = 6;
        let (chrome_bg, status_bg, bar_bg, page_bg) = (0x111111, 0x222222, 0x333333, 0x444444);
        let bar_cell = |bg| Cell {
            bg,
            ..Cell::blank()
        };
        let chrome = vec![bar_cell(chrome_bg); 10];
        let status = vec![bar_cell(status_bg); 10];
        let mut g = Grid::new(2, 2);
        AnsiParser::new().advance(&mut g, b"\x1b[48;2;0;0;255m\x1b[2J");
        let view = g.frame_view();
        let pane = PaneFrame {
            grid: &view,
            col0: 0,
            row0: 0,
            focused: false,
            cursor_on: false,
            trail: Vec::new(),
            hover_link: None,
            marks: Vec::new(),
        };
        // A zoomed-in grid font (20×30) under a 6×12 UI font.
        let (mut font, mut ui) = (Blank((20, 30)), Blank((6, 12)));
        let (w, h) = (60, 120);
        let mut buf = vec![0; w * h];
        composite(
            &mut buf,
            &[pane],
            &chrome,
            &status,
            &mut font,
            &mut ui,
            w,
            h,
            0,
            page_bg,
            bar_bg,
            PAD,
            None,
        );
        let at = |x: usize, y: usize| buf[y * w + x];
        let inset = bar_inset(PAD, 12);
        assert_eq!(inset, 2);
        assert_eq!(at(0, 0), bar_bg, "strip band above the tabs");
        assert_eq!(at(0, inset), chrome_bg, "chrome starts below the band");
        assert_eq!(at(0, inset + 11), chrome_bg, "chrome is one UI cell tall");
        assert_eq!(at(PAD, inset + 12), page_bg, "padding gap below the bar");
        let top = grid_top(&chrome, PAD, 12);
        assert_eq!(top, PAD + inset + 12);
        assert_eq!(at(PAD, top), 0x0000FF, "the grid starts below the bar");
        assert_eq!(at(PAD, top - 1), page_bg);
        assert_eq!(at(0, h - 12), status_bg, "status is one UI cell tall");
        assert_eq!(at(0, h - 13), page_bg);
    }

    /// The first cell of a 1×1 grid after `bytes`, as the window draws it.
    fn drawn_after(bytes: &[u8]) -> Cell {
        let mut g = Grid::new(1, 1);
        AnsiParser::new().advance(&mut g, bytes);
        drawn(g.frame_view().cell(0, 0))
    }

    #[test]
    fn plain_cells_draw_as_stored() {
        let cell = drawn_after(b"\x1b[38;2;255;0;0;48;2;0;0;255mX");
        assert_eq!((cell.ch, cell.fg, cell.bg), ('X', 0xFF0000, 0x0000FF));
    }

    #[test]
    fn reverse_swaps_fg_and_bg() {
        let cell = drawn_after(b"\x1b[38;2;255;0;0;48;2;0;0;255;7mX");
        assert_eq!((cell.fg, cell.bg), (0x0000FF, 0xFF0000));
    }

    #[test]
    fn dim_fades_fg_toward_bg() {
        let cell = drawn_after(b"\x1b[38;2;255;255;255;48;2;0;0;0;2mX");
        assert_eq!(cell.bg, 0x000000);
        assert_eq!(cell.fg, 0xAAAAAA);
    }

    #[test]
    fn dim_reverse_fades_the_swapped_fg() {
        let cell = drawn_after(b"\x1b[38;2;255;255;255;48;2;0;0;0;2;7mX");
        assert_eq!((cell.fg, cell.bg), (0x555555, 0xFFFFFF));
    }

    #[test]
    fn hidden_keeps_its_character_but_draws_no_glyph_or_lines() {
        let cell = drawn_after(b"\x1b[48;2;0;0;255;8;4;9;53mX");
        assert_eq!(cell.ch, 'X', "layout unchanged");
        assert_eq!(glyph_char(&cell, None, 0), ' ');
        assert_eq!(
            cell.flags & (ATTR_UNDERLINE | ATTR_STRIKE | ATTR_OVERLINE),
            0
        );
        assert_eq!(cell.bg, 0x0000FF);
    }

    #[test]
    fn concealed_arabic_stays_blank_through_bidi_shaping() {
        let mut g = Grid::new(4, 1);
        g.bidi = true;
        AnsiParser::new().advance(&mut g, "\x1b[8m\u{628}\u{628}".as_bytes());
        let view = g.frame_view();
        let row = view.bidi_row(0).expect("an RTL row");
        assert!(row.shaped.is_some(), "the row is shaped");
        for col in 0..2 {
            assert_eq!(glyph_char(&drawn(view.cell(col, 0)), Some(row), col), ' ');
        }
        // Not concealed, the same row draws its shaped forms.
        let mut g = Grid::new(4, 1);
        g.bidi = true;
        AnsiParser::new().advance(&mut g, "\u{628}\u{628}".as_bytes());
        let view = g.frame_view();
        let row = view.bidi_row(0).expect("an RTL row");
        assert_ne!(glyph_char(&view.cell(0, 0), Some(row), 0), '\u{628}');
    }

    #[test]
    fn blend_endpoints() {
        assert_eq!(blend(0x000000, 0xFFFFFF, 255), 0xFFFFFF); // full coverage = fg
        assert_eq!(blend(0x0000FF, 0xFF0000, 0), 0x0000FF); // zero coverage = bg
        assert_eq!(blend(0x000000, 0xFFFFFF, 128), 0x808080); // half = mid-gray
    }
}
