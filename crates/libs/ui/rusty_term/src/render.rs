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

/// What an SGR sequence sets for a run of cells, normalized so equal
/// renditions compare equal: underline style bits count only while the
/// underline is on, and the underline color only while its flag is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rendition {
    fg: u32,
    bg: u32,
    attrs: u16,
    underline_color: u32,
}

impl Rendition {
    fn new(fg: u32, bg: u32, flags: u16, underline_color: u32) -> Self {
        let mut attrs = flags & ATTR_MASK;
        if attrs & ATTR_UNDERLINE == 0 {
            attrs = UnderlineStyle::Straight.pack_into(attrs);
        }
        let has_color = attrs & ATTR_UNDERLINE_COLOR != 0;
        Rendition {
            fg,
            bg,
            attrs,
            underline_color: if has_color { underline_color } else { 0 },
        }
    }

    fn underline(self) -> Option<UnderlineStyle> {
        (self.attrs & ATTR_UNDERLINE != 0).then(|| UnderlineStyle::from_attrs(self.attrs))
    }
}

/// On/off attributes and their SGR parameters, in emission order.
const SGR_FLAGS: [(u16, &str); 7] = [
    (ATTR_BOLD, "1"),
    (ATTR_DIM, "2"),
    (ATTR_ITALIC, "3"),
    (ATTR_BLINK, "5"),
    (ATTR_REVERSE, "7"),
    (ATTR_HIDDEN, "8"),
    (ATTR_STRIKE, "9"),
];

/// Append the SGR that takes the host from `prev` to `next`. With `prev`
/// unknown (`None`), or when `next` drops an attribute `prev` had, it starts
/// from a reset (`0`) and states everything; otherwise it states only what
/// `next` adds or changes. The underline style and color use the colon and
/// `58;2` forms, so a host that knows undercurl and colored underlines gets
/// the same rendition back.
fn push_sgr(out: &mut String, prev: Option<Rendition>, next: Rendition) {
    use std::fmt::Write as _;
    let base = prev.filter(|p| p.attrs & !next.attrs == 0);
    let start = out.len();
    out.push_str("\x1b[");
    if base.is_none() {
        out.push_str("0;");
    }
    let had = base.map_or(0, |p| p.attrs);
    let changed = |f: fn(&Rendition) -> u32| base.is_none_or(|p| f(&p) != f(&next));
    for (flag, param) in SGR_FLAGS {
        if next.attrs & flag != 0 && had & flag == 0 {
            out.push_str(param);
            out.push(';');
        }
    }
    if let Some(underline) = next.underline()
        && base.and_then(Rendition::underline) != Some(underline)
    {
        let style = match underline {
            UnderlineStyle::Straight => 1,
            UnderlineStyle::Double => 2,
            UnderlineStyle::Curly => 3,
            UnderlineStyle::Dotted => 4,
            UnderlineStyle::Dashed => 5,
        };
        let _ = write!(out, "4:{style};");
    }
    let rgb = |c: u32| ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    if changed(|r| r.fg) {
        let (r, g, b) = rgb(next.fg);
        let _ = write!(out, "38;2;{r};{g};{b};");
    }
    if changed(|r| r.bg) {
        let (r, g, b) = rgb(next.bg);
        let _ = write!(out, "48;2;{r};{g};{b};");
    }
    if next.attrs & ATTR_UNDERLINE_COLOR != 0
        && (had & ATTR_UNDERLINE_COLOR == 0 || changed(|r| r.underline_color))
    {
        let (r, g, b) = rgb(next.underline_color);
        let _ = write!(out, "58;2;{r};{g};{b};");
    }
    if out.ends_with(';') {
        out.pop();
        out.push('m');
    } else {
        out.truncate(start); // nothing changed
    }
}

/// Paint the dirty rows of `frame` to stdout in one write, then position the
/// hardware cursor where the shell's cursor is.
pub(crate) fn draw(frame: &DirtyFrame, position_cursor: bool) {
    let mut buf = String::new();
    encode_frame(frame, position_cursor, &mut buf);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(buf.as_bytes());
    let _ = out.flush();
}

/// Append the bytes that paint the dirty rows of `frame`, then, when
/// `position_cursor`, the escape that places the host cursor. SGR is emitted
/// only where the rendition changes, and only the part that changed, so a
/// run of same-styled cells costs one introducer. The rendition carries from
/// row to row, since only cursor moves and line sizes lie between them, and
/// one reset after the last row leaves the host's pen at its default.
fn encode_frame(frame: &DirtyFrame, position_cursor: bool, out: &mut String) {
    use std::fmt::Write as _;
    let cells_total: usize = frame.rows.iter().map(|(_, c)| c.len()).sum();
    out.reserve(cells_total * 2 + frame.rows.len() * 16);
    let mut pen = None;
    for row in 0..frame.rows.len() {
        pen = encode_row(frame, row, pen, out);
    }
    if !frame.rows.is_empty() {
        out.push_str("\x1b[0m");
    }

    // Place the visible cursor where the shell expects it (1-indexed). Skipped
    // while browsing scrollback, where the live cursor position is meaningless.
    if position_cursor {
        let (cx, cy) = frame.cursor;
        let _ = write!(out, "\x1b[{};{}H", cy + 1, cx + 1);
    }
}

/// Append dirty row `row` of `frame`: the move to its first column, its line
/// size, cells and hyperlinks. `last` is the host's rendition before the row;
/// returns the one after it.
fn encode_row(
    frame: &DirtyFrame,
    row: usize,
    mut last: Option<Rendition>,
    out: &mut String,
) -> Option<Rendition> {
    use std::fmt::Write as _;
    let (y, cells) = (frame.rows[row].0, &frame.rows[row].1);
    let _ = write!(out, "\x1b[{};1H", y + 1);
    // Relay this row's line size (DECDWL/DECDHL) to the host, which scales the
    // glyphs. A double-width/height row displays only the left half of the
    // columns, so cap emission to avoid spilling into the next line.
    let (dec_seq, max_cells) = match frame.line_attrs.get(y).copied().unwrap_or(LineAttr::Single) {
        LineAttr::Single => ("\x1b#5", cells.len()),
        LineAttr::DoubleWidth => ("\x1b#6", cells.len() / 2),
        LineAttr::DoubleTop => ("\x1b#3", cells.len() / 2),
        LineAttr::DoubleBottom => ("\x1b#4", cells.len() / 2),
    };
    out.push_str(dec_seq);
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
                    out.push_str("\x1b]8;;");
                    out.push_str(uri);
                    out.push_str("\x1b\\");
                }
                // link == 0, or an unknown id: close any open link.
                _ => out.push_str("\x1b]8;;\x1b\\"),
            }
            cur_link = cell.link;
        }
        let pen = Rendition::new(cell.fg, cell.bg, cell.flags, cell.underline_color);
        if last != Some(pen) {
            push_sgr(out, last, pen);
            last = Some(pen);
        }
        out.push(cell.ch);
        // Emit the grapheme continuation (combining marks, ZWJ joins, …) so
        // the full cluster renders as one glyph.
        if cell.cluster != 0
            && let Some(suffix) = frame.clusters.get((cell.cluster - 1) as usize)
        {
            out.push_str(suffix);
        }
    }
    // Close a still-open hyperlink before ending the row.
    if cur_link != 0 {
        out.push_str("\x1b]8;;\x1b\\");
    }
    last
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
    use crate::core::AnsiParser;

    /// A rendition from a few random bits: every on/off flag, an underline
    /// in any style, an underline color, and colors from a small palette so
    /// that consecutive draws often share them.
    fn random_rendition(seed: &mut u64) -> Rendition {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let r = *seed >> 16;
        let mut flags = SGR_FLAGS
            .iter()
            .enumerate()
            .filter(|&(i, _)| r >> i & 1 == 1)
            .fold(0, |acc, (_, &(flag, _))| acc | flag);
        if r >> 7 & 1 == 1 {
            let style = [
                UnderlineStyle::Straight,
                UnderlineStyle::Double,
                UnderlineStyle::Curly,
                UnderlineStyle::Dotted,
                UnderlineStyle::Dashed,
            ][(r >> 8) as usize % 5];
            flags = style.pack_into(flags | ATTR_UNDERLINE);
        }
        if r >> 11 & 1 == 1 {
            flags |= ATTR_UNDERLINE_COLOR;
        }
        let palette = [0x000000, 0xFFFFFF, 0x102030, 0xC0FFEE];
        let color = |shift: u64| palette[(r >> shift) as usize % palette.len()];
        Rendition::new(color(12), color(14), flags, color(16))
    }

    /// The renditions a real parser assigns to the cells printed after each
    /// SGR this module emits for `pens`, in order.
    fn parsed_renditions(pens: &[Rendition]) -> Vec<Rendition> {
        let mut bytes = String::new();
        let mut last = None;
        for &pen in pens {
            push_sgr(&mut bytes, last, pen);
            bytes.push('x');
            last = Some(pen);
        }
        let mut grid = Grid::new(pens.len(), 1);
        AnsiParser::new().advance(&mut grid, bytes.as_bytes());
        let frame = grid.snapshot_viewport();
        frame.rows[0]
            .1
            .iter()
            .map(|c| Rendition::new(c.fg, c.bg, c.flags, c.underline_color))
            .collect()
    }

    #[test]
    fn a_first_sgr_resets_and_states_everything() {
        let mut out = String::new();
        let pen = Rendition::new(0x010203, 0x0A0B0C, ATTR_BOLD | ATTR_UNDERLINE, 0);
        push_sgr(&mut out, None, pen);
        assert_eq!(out, "\x1b[0;1;4:1;38;2;1;2;3;48;2;10;11;12m");
    }

    #[test]
    fn an_sgr_states_only_what_changed() {
        let base = Rendition::new(0x010203, 0x0A0B0C, 0, 0);
        let cases = [
            (
                Rendition {
                    attrs: ATTR_BOLD,
                    ..base
                },
                "\x1b[1m",
            ),
            (
                Rendition {
                    fg: 0xFF0000,
                    ..base
                },
                "\x1b[38;2;255;0;0m",
            ),
            (base, ""),
        ];
        for (next, want) in cases {
            let mut out = String::new();
            push_sgr(&mut out, Some(base), next);
            assert_eq!(out, want, "{next:?}");
        }
    }

    #[test]
    fn dropping_an_attribute_resets_first() {
        let bold = Rendition::new(0x010203, 0x0A0B0C, ATTR_BOLD, 0);
        let mut out = String::new();
        push_sgr(&mut out, Some(bold), Rendition { attrs: 0, ..bold });
        assert_eq!(out, "\x1b[0;38;2;1;2;3;48;2;10;11;12m");
    }

    #[test]
    fn style_bits_of_a_hidden_underline_do_not_change_the_rendition() {
        let curly_off = UnderlineStyle::Curly.pack_into(0);
        assert_eq!(
            Rendition::new(1, 2, curly_off, 9),
            Rendition::new(1, 2, 0, 0)
        );
    }

    /// Any chain of SGR deltas leaves the parser on exactly the rendition
    /// each cell asked for.
    #[test]
    fn sgr_deltas_parse_back_to_the_renditions_they_encode() {
        let mut seed = 0x2545_F491_4F6C_DD1D;
        for round in 0..200 {
            let pens: Vec<Rendition> = (0..24).map(|_| random_rendition(&mut seed)).collect();
            assert_eq!(parsed_renditions(&pens), pens, "round {round}");
        }
    }

    /// Bytes and encode time of a full 120x40 repaint after each
    /// `bench/workloads` file, against the full SGR at every style change
    /// that this module used to emit. Run with `--ignored --nocapture`
    /// after `python3 bench/gen_workloads.py --size quick`.
    #[test]
    #[ignore = "measurement; needs generated bench workloads"]
    fn measure_tui_frame_bytes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/workloads");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .expect("workloads")
            .flatten()
            .collect();
        names.sort_by_key(|e| e.file_name());
        for entry in names {
            let mut grid = Grid::new(120, 40);
            AnsiParser::new().advance(&mut grid, &std::fs::read(entry.path()).expect("read"));
            let frame = grid.snapshot_viewport();
            // Before: each row started from a reset, stated the full SGR at
            // every change and ended with its own reset.
            let mut full = 0usize;
            for (_, cells) in &frame.rows {
                let mut last = None;
                for c in cells {
                    let pen = Rendition::new(c.fg, c.bg, c.flags, c.underline_color);
                    if last != Some(pen) {
                        let mut s = String::new();
                        push_sgr(&mut s, None, pen);
                        full += s.len();
                        last = Some(pen);
                    }
                }
            }
            let mut out = String::new();
            let t = Instant::now();
            for _ in 0..100 {
                out.clear();
                encode_frame(&frame, true, &mut out);
            }
            let us = t.elapsed().as_secs_f64() * 1e4;
            let mut sgr = 0usize;
            let mut last = None;
            for c in frame.rows.iter().flat_map(|(_, cells)| cells) {
                let pen = Rendition::new(c.fg, c.bg, c.flags, c.underline_color);
                if last != Some(pen) {
                    let mut s = String::new();
                    push_sgr(&mut s, last, pen);
                    sgr += s.len();
                    last = Some(pen);
                }
            }
            let resets = "\x1b[0m".len() * (frame.rows.len() - 1);
            let before = out.len() - sgr + full + resets;
            println!(
                "{:?}: {} -> {} bytes ({:.0}%), encode {us:.1} us",
                entry.file_name(),
                before,
                out.len(),
                100.0 * out.len() as f64 / before as f64
            );
        }
    }

    #[test]
    fn a_frame_carries_its_rendition_across_rows_and_resets_once() {
        let mut grid = Grid::new(6, 2);
        AnsiParser::new().advance(&mut grid, b"\x1b[1mab\x1b[31mcd\r\n\x1b[0mef");
        let mut out = String::new();
        encode_frame(&grid.snapshot_viewport(), true, &mut out);
        assert!(
            out.ends_with("\x1b[0m\x1b[2;3H"),
            "reset, then cursor: {out:?}"
        );
        assert!(
            out.contains("ab\x1b[38;2;"),
            "a color change adds no reset: {out:?}"
        );
        let row2 = &out[out.find("\x1b[2;1H").expect("row 2")..];
        assert!(
            row2.starts_with("\x1b[2;1H\x1b#5ef"),
            "row 1's blank tail already set row 2's pen: {row2:?}"
        );
        assert_eq!(
            out.matches("\x1b[0").count(),
            3,
            "first pen, bold dropped, final reset"
        );
    }

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
