//! Shared rendering support used by both runtimes: SGR/glyph output to the host
//! terminal, the one-shot `render_once` snapshot-and-paint step, the host
//! raw-mode guard, and the host-input-mode reset performed on exit.
//!
//! Keeping the paint path here means the runtime's wake mechanism (`AsyncFd` on
//! Unix, channel + poll bridge on Windows) never changes *what* gets drawn.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::sync::Mutex;

use crate::backend::Backend;
use crate::core::{
    ATTR_BLINK, ATTR_BOLD, ATTR_DIM, ATTR_HIDDEN, ATTR_ITALIC, ATTR_MASK, ATTR_REVERSE,
    ATTR_STRIKE, ATTR_UNDERLINE, ATTR_UNDERLINE_COLOR, DirtyFrame, Grid, LineAttr, UnderlineStyle,
    WIDE_TRAILER,
};

/// Minimum wall-clock spacing between repaints. Bursts of output coalesce into
/// at most one frame per budget (~60 Hz), so a flood (`cat bigfile`) repaints
/// smoothly instead of once per PTY read.
pub(crate) const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// Restores the host terminal out of raw mode when dropped, so an early return
/// or a panic can never leave the user's shell with echo/line-editing disabled.
pub(crate) struct RawModeGuard<'a> {
    backend: &'a dyn Backend,
}

impl<'a> RawModeGuard<'a> {
    pub(crate) fn enable(backend: &'a dyn Backend) -> Result<Self, std::io::Error> {
        backend.set_raw_mode(true)?;
        Ok(Self { backend })
    }
}

impl Drop for RawModeGuard<'_> {
    fn drop(&mut self) {
        let _ = self.backend.set_raw_mode(false);
    }
}

/// The bytes [`restore_host_modes`] writes, pulled out as a constant so its
/// contents are assertable without capturing stdout (see the `mod tests`
/// below). Resets: application cursor keys (`?1`), the mouse-tracking modes
/// and their SGR/urxvt/pixel encodings (`?1000`/`?1002`/`?1003`/`?1006`/
/// `?1015`/`?1016`), focus reporting (`?1004`), bracketed paste (`?2004`),
/// cursor visibility (`?25h`), the Kitty keyboard protocol stack (`<128u`
/// pop, generous enough to empty any realistic stack, then `=0;1u` to force
/// the resulting flags to 0 regardless of stack depth), xterm
/// `modifyOtherKeys` (`>4;0m`), and the cursor shape (`0 q`, DECSCUSR
/// default).
const RESTORE_HOST_MODES: &[u8] = b"\x1b[?1l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?1015l\x1b[?1016l\x1b[?2004l\x1b[?25h\
    \x1b[<128u\x1b[=0;1u\x1b[>4;0m\x1b[0 q";

/// Reset any host-terminal input modes a runtime may have relayed on the
/// child's behalf (mouse, focus, bracketed paste, the Kitty keyboard
/// protocol, xterm `modifyOtherKeys`, and the cursor shape) and ensure the
/// cursor is visible, so a child that exited without disabling them can't
/// leave the host stuck misinterpreting the next program's keys or clicks.
/// Called once on shutdown.
///
/// The Kitty/modifyOtherKeys/DECSCUSR resets matter as much as the
/// mouse/paste ones already here: a child that pushed the Kitty keyboard
/// protocol (`CSI > flags u`, e.g. neovim) and got killed rather than
/// exiting cleanly leaves the *host* terminal's key encoding in that mode —
/// the next thing typed at the host's own prompt (Enter, Esc, Backspace) can
/// come through as the Kitty `CSI u` encoding instead of the plain bytes the
/// host shell expects.
pub(crate) fn restore_host_modes() {
    let mut out = std::io::stdout();
    let _ = out.write_all(RESTORE_HOST_MODES);
    let _ = out.flush();
}

/// Build the combined SGR introducer for a foreground/background/attribute/
/// underline-color quadruple. Starts with a reset (`0`) so attributes left
/// active by the previous run are cleared, then re-states the active
/// attributes and truecolor pairs. The underline style/color re-emit as the
/// colon sub-parameter forms (`4:N`, `58;2;…`) so a host terminal that
/// understands undercurl/colored-underline gets the same rendition back.
fn sgr_for(fg: u32, bg: u32, attrs: u16, underline_color: u32) -> String {
    let mut s = String::from("\x1b[0");
    if attrs & ATTR_BOLD != 0 {
        s.push_str(";1");
    }
    if attrs & ATTR_DIM != 0 {
        s.push_str(";2");
    }
    if attrs & ATTR_ITALIC != 0 {
        s.push_str(";3");
    }
    if attrs & ATTR_UNDERLINE != 0 {
        let style = match UnderlineStyle::from_attrs(attrs) {
            UnderlineStyle::Straight => 1,
            UnderlineStyle::Double => 2,
            UnderlineStyle::Curly => 3,
            UnderlineStyle::Dotted => 4,
            UnderlineStyle::Dashed => 5,
        };
        use std::fmt::Write as _;
        let _ = write!(s, ";4:{style}");
    }
    if attrs & ATTR_BLINK != 0 {
        s.push_str(";5");
    }
    if attrs & ATTR_REVERSE != 0 {
        s.push_str(";7");
    }
    if attrs & ATTR_HIDDEN != 0 {
        s.push_str(";8");
    }
    if attrs & ATTR_STRIKE != 0 {
        s.push_str(";9");
    }
    let (fr, fg_, fb) = ((fg >> 16) & 0xFF, (fg >> 8) & 0xFF, fg & 0xFF);
    let (br, bg_, bb) = ((bg >> 16) & 0xFF, (bg >> 8) & 0xFF, bg & 0xFF);
    use std::fmt::Write as _;
    let _ = write!(s, ";38;2;{};{};{};48;2;{};{};{}", fr, fg_, fb, br, bg_, bb);
    if attrs & ATTR_UNDERLINE_COLOR != 0 {
        let (ur, ug, ub) = (
            (underline_color >> 16) & 0xFF,
            (underline_color >> 8) & 0xFF,
            underline_color & 0xFF,
        );
        let _ = write!(s, ";58;2;{ur};{ug};{ub}");
    }
    s.push('m');
    s
}

/// Paint the dirty rows of `frame` to stdout, then position the hardware cursor
/// where the shell's cursor is. SGR sequences are emitted only when the color
/// changes within a row, so a run of same-colored cells costs one introducer
/// instead of one per cell.
pub(crate) fn draw(frame: &DirtyFrame, position_cursor: bool) {
    let out = std::io::stdout();
    let mut out = out.lock();

    for (y, cells) in &frame.rows {
        // Move to column 1 (1-indexed) of this row.
        let _ = write!(out, "\x1b[{};1H", y + 1);

        // Relay this row's line size (DECDWL/DECDHL) to the host, which scales the
        // glyphs. A double-width/height row displays only the left half of the
        // columns, so cap emission to avoid spilling into the next line.
        let (dec_seq, max_cells) = match frame
            .line_attrs
            .get(*y)
            .copied()
            .unwrap_or(LineAttr::Single)
        {
            LineAttr::Single => ("\x1b#5", cells.len()),
            LineAttr::DoubleWidth => ("\x1b#6", cells.len() / 2),
            LineAttr::DoubleTop => ("\x1b#3", cells.len() / 2),
            LineAttr::DoubleBottom => ("\x1b#4", cells.len() / 2),
        };
        let mut line_buf = String::with_capacity(cells.len() + 32);
        line_buf.push_str(dec_seq);
        let mut last: Option<(u32, u32, u16, u32)> = None;
        // Active hyperlink id while painting this row; reset per row so a link
        // is reopened at the start of each line it covers and closed at row end.
        let mut cur_link: u16 = 0;
        for cell in cells.iter().take(max_cells) {
            // The trailing half of a wide glyph is not emitted; the glyph
            // itself already advances the host cursor by two columns.
            if cell.flags & WIDE_TRAILER != 0 {
                continue;
            }
            // Open/close an OSC 8 hyperlink when the cell's link changes.
            if cell.link != cur_link {
                match frame.links.get(cell.link.wrapping_sub(1) as usize) {
                    Some(uri) if cell.link != 0 => {
                        line_buf.push_str("\x1b]8;;");
                        line_buf.push_str(uri);
                        line_buf.push_str("\x1b\\");
                    }
                    // link == 0, or an unknown id: close any open link.
                    _ => line_buf.push_str("\x1b]8;;\x1b\\"),
                }
                cur_link = cell.link;
            }
            // Style key excludes the WIDE_TRAILER layout bit (trailers are
            // skipped above, so only rendition attributes reach here).
            let attrs = cell.flags & ATTR_MASK;
            let key = (cell.fg, cell.bg, attrs, cell.underline_color);
            if last != Some(key) {
                line_buf.push_str(&sgr_for(cell.fg, cell.bg, attrs, cell.underline_color));
                last = Some(key);
            }
            line_buf.push(cell.ch);
            // Emit the grapheme continuation (combining marks, ZWJ joins, …) so
            // the full cluster renders as one glyph.
            if cell.cluster != 0
                && let Some(suffix) = frame.clusters.get((cell.cluster - 1) as usize)
            {
                line_buf.push_str(suffix);
            }
        }
        // Close a still-open hyperlink before ending the row.
        if cur_link != 0 {
            line_buf.push_str("\x1b]8;;\x1b\\");
        }
        line_buf.push_str("\x1b[0m");
        let _ = write!(out, "{}", line_buf);
    }

    // Place the visible cursor where the shell expects it (1-indexed). Skipped
    // while browsing scrollback, where the live cursor position is meaningless.
    if position_cursor {
        let (cx, cy) = frame.cursor;
        let _ = write!(out, "\x1b[{};{}H", cy + 1, cx + 1);
    }
    let _ = out.flush();
}

/// Mutable per-frame renderer bookkeeping carried across repaints, so a paint
/// happens only when something actually changed (cells, cursor, title, or
/// cursor visibility).
pub(crate) struct RenderState {
    /// Last cursor position the host cursor was placed at; lets a cursor-only
    /// move (arrows, Ctrl-A/E, backspace) repaint even when no row is dirty.
    pub(crate) last_cursor: Option<(usize, usize)>,
    /// Last window title forwarded to the host, so OSC 0/2 updates pass through
    /// only when they actually change.
    pub(crate) last_title: Option<String>,
    /// Current visibility of the host cursor (starts shown).
    pub(crate) cursor_shown: bool,
    /// Timestamp of the last actual paint, so the frame budget spaces real
    /// repaints rather than no-op wakes.
    pub(crate) last_frame: Instant,
    /// The open synchronized update's timeout that a wakeup is already set
    /// for, so a burst of held frames sets one timer, not one each.
    sync_until: Option<Instant>,
    /// Set by [`resize_grid`]: the next frame clears the host screen and
    /// repaints every row, even mid synchronized update.
    clear_pending: bool,
}

impl RenderState {
    pub(crate) fn new() -> Self {
        Self {
            last_cursor: None,
            last_title: None,
            cursor_shown: true,
            last_frame: Instant::now(),
            sync_until: None,
            clear_pending: false,
        }
    }
}

/// Reflow `grid` to `cols`x`rows` when that changes its size, and have the
/// next [`render_once`] clear the host screen and repaint it in full. A
/// resize releases a held synchronized update for that one frame, as the
/// window does, so the clear and the reflowed grid always land together and
/// the screen is never left blank. `true` on a change.
pub(crate) fn resize_grid(
    grid: &Mutex<Grid>,
    st: &mut RenderState,
    cols: usize,
    rows: usize,
) -> bool {
    let mut g = grid.lock();
    if (g.cols, g.rows) == (cols, rows) {
        return false;
    }
    g.resize(cols, rows);
    st.clear_pending = true;
    true
}

/// Snapshot the grid and paint one frame if warranted, forwarding any clipboard
/// (OSC 52) bytes, title (OSC 0/2) changes, and cursor-visibility (DECTCEM)
/// changes to the host terminal. Shared verbatim by both runtimes.
///
/// While a synchronized update (DEC 2026) is open nothing is painted, so
/// input, a resize or a scroll can't show a half-drawn frame. The first held
/// frame returns the update's timeout, for the caller to wake and repaint
/// then; later ones for the same update return `None`. A frame after
/// [`resize_grid`] is the exception: it always paints.
#[must_use]
pub(crate) fn render_once(grid: &Mutex<Grid>, st: &mut RenderState) -> Option<Instant> {
    let (frame, title, viewing, dirty_any, host_out, app_cursor_visible) = {
        let mut g = grid.lock();
        if !st.clear_pending
            && let Some(until) = g.sync_output_until()
        {
            return (st.sync_until.replace(until) != Some(until)).then_some(until);
        }
        st.sync_until = None;
        if st.clear_pending {
            g.dirty.fill(true);
        }
        let viewing = g.view_offset > 0;
        let dirty_any = g.dirty.iter().any(|&d| d);
        // While scrolled back, composite history over the live grid; that
        // snapshot covers every row, so it's only worth painting on a change.
        let frame = if viewing {
            g.snapshot_viewport()
        } else {
            g.snapshot_dirty()
        };
        g.clear_dirty();
        (
            frame,
            g.title.clone(),
            viewing,
            dirty_any,
            g.take_host_out(),
            g.cursor_visible,
        )
    };

    // Forward any clipboard (OSC 52) bytes to the host terminal verbatim.
    if !host_out.is_empty() {
        let mut out = std::io::stdout();
        let _ = out.write_all(&host_out);
        let _ = out.flush();
    }

    // Forward a changed, non-empty window title to the host so its title bar
    // tracks what the child set via OSC 0/2.
    if !title.is_empty() && st.last_title.as_deref() != Some(title.as_str()) {
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]0;{}\x07", title);
        let _ = out.flush();
        st.last_title = Some(title);
    }

    // The host cursor is shown only in the live view and only when the child
    // wants it visible. Sync the host's state on any change.
    let want_cursor = !viewing && app_cursor_visible;
    if want_cursor != st.cursor_shown {
        let mut out = std::io::stdout();
        let _ = out.write_all(if want_cursor {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        });
        let _ = out.flush();
        st.cursor_shown = want_cursor;
    }

    if std::mem::take(&mut st.clear_pending) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[2J");
        st.last_cursor = None;
    }

    if viewing {
        // Repaint the whole viewport only when something changed (a scroll, or
        // new output arriving underneath).
        if dirty_any {
            draw(&frame, false);
            st.last_frame = Instant::now();
        }
        // Force a cursor reposition on the first live frame after we return.
        st.last_cursor = None;
    } else if !frame.rows.is_empty() || st.last_cursor != Some(frame.cursor) {
        // Draw when cells changed, or when only the cursor moved — `draw` emits
        // the final cursor-positioning escape, so a pure motion still needs it.
        st.last_cursor = Some(frame.cursor);
        draw(&frame, true);
        st.last_frame = Instant::now();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_synchronized_update_holds_paints_and_asks_for_one_wakeup() {
        use crate::core::AnsiParser;
        let grid = Mutex::new(Grid::new(20, 4));
        let mut p = AnsiParser::new();
        p.advance(&mut grid.lock(), b"\x1b[?2026hhalf a frame");
        let mut st = RenderState::new();
        let until = render_once(&grid, &mut st);
        assert!(until.is_some(), "held, with its timeout");
        assert!(grid.lock().dirty.iter().any(|&d| d), "nothing painted");
        assert_eq!(render_once(&grid, &mut st), None, "one wakeup per update");
        p.advance(&mut grid.lock(), b"\x1b[?2026l");
        assert_eq!(render_once(&grid, &mut st), None);
        assert!(!grid.lock().dirty.iter().any(|&d| d), "painted once closed");
    }

    #[test]
    fn a_resize_during_a_synchronized_update_repaints_the_reflowed_grid() {
        use crate::core::AnsiParser;
        let grid = Mutex::new(Grid::new(20, 4));
        let mut p = AnsiParser::new();
        p.advance(&mut grid.lock(), b"\x1b[?2026hhalf a frame");
        let mut st = RenderState::new();
        assert!(render_once(&grid, &mut st).is_some(), "held");
        assert!(!resize_grid(&grid, &mut st, 20, 4), "same size: no change");
        assert!(resize_grid(&grid, &mut st, 30, 6));
        assert_eq!(render_once(&grid, &mut st), None, "painted, not held");
        let mut g = grid.lock();
        assert_eq!((g.cols, g.rows), (30, 6));
        assert!(
            !g.dirty.iter().any(|&d| d),
            "every row painted with the clear"
        );
        assert!(g.sync_output_active(), "the update itself stays open");
        drop(g);
        assert!(!st.clear_pending);
        assert!(
            render_once(&grid, &mut st).is_some(),
            "later frames hold again"
        );
    }

    #[test]
    fn restore_host_modes_resets_kitty_modify_keys_and_cursor_shape() {
        // A child that pushed the Kitty keyboard protocol / modifyOtherKeys /
        // a custom cursor shape and then got killed (rather than exiting
        // cleanly and resetting them itself) used to leave the *host*
        // terminal stuck in that mode — the next keystroke at the host's own
        // prompt could come through Kitty-encoded instead of as plain bytes.
        let s = String::from_utf8_lossy(RESTORE_HOST_MODES);
        assert!(
            s.contains("\x1b[<128u"),
            "must pop the Kitty keyboard flag stack: {s:?}"
        );
        assert!(
            s.contains("\x1b[=0;1u"),
            "must force Kitty flags to 0 regardless of stack depth: {s:?}"
        );
        assert!(
            s.contains("\x1b[>4;0m"),
            "must reset xterm modifyOtherKeys: {s:?}"
        );
        assert!(
            s.contains("\x1b[0 q"),
            "must reset the cursor shape (DECSCUSR default): {s:?}"
        );
    }

    #[test]
    fn restore_host_modes_still_resets_mouse_focus_paste_and_cursor_visibility() {
        // Regression guard for the modes this already handled before adding
        // the Kitty/modifyOtherKeys/DECSCUSR resets above.
        let s = String::from_utf8_lossy(RESTORE_HOST_MODES);
        for mode in [
            "?1000l", "?1002l", "?1003l", "?1004l", "?1006l", "?1015l", "?1016l", "?2004l",
        ] {
            assert!(s.contains(mode), "must reset {mode}: {s:?}");
        }
        assert!(s.contains("?25h"), "must leave the cursor visible: {s:?}");
    }
}
