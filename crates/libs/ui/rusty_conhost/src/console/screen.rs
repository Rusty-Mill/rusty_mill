//! The host's screen (design §5): an [`AnsiParser`] over a [`Grid`] the
//! size of the window, with no scrollback. Every byte the host sends the
//! terminal goes through here, so the Grid is what the terminal was told
//! and the screen APIs read it.
//!
//! Client text first takes the output-mode table (§5.4). Under VT
//! processing, DSR 5 and CPR queries are answered from the Grid and
//! dropped from the forwarded VT (§5.6, V11); every other query goes to
//! the terminal and the parser's answer is discarded. A client turning
//! off win32-input-mode or focus reports, or resetting the terminal, has
//! them turned back on, as conhost does: the host needs both (§5.5).
//! Without VT processing, a write that fills the last column wraps at
//! once with CR LF, as conhost's legacy path does.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/host/getset.cpp`, `_stream.cpp`, `src/host/VtIo.cpp`,
//! `src/terminal/adapter/adaptDispatch.cpp`; see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use platform::term::WinSize;
use rusty_term_core::{ATTR_REVERSE, ATTR_UNDERLINE, AnsiParser, CellColors, ColorRef, Grid};

use super::{OutputQueue, PREAMBLE};

/// `ENABLE_*` output mode bits.
pub(super) const PROCESSED_OUTPUT: u32 = 0x1;
pub(super) const WRAP_AT_EOL_OUTPUT: u32 = 0x2;
pub(super) const VIRTUAL_TERMINAL_PROCESSING: u32 = 0x4;
pub(super) const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x8;

/// `COMMON_LVB_*` attribute bits.
const LVB_REVERSE_VIDEO: u16 = 0x4000;
const LVB_UNDERSCORE: u16 = 0x8000;
/// The bits SetConsoleTextAttribute accepts (conhost `FG_ATTRS`,
/// `BG_ATTRS` and `META_ATTRS`): all but 0x2000.
pub(super) const VALID_ATTRIBUTES: u16 = !0x2000;

/// What the host does with a sequence it watches for in client VT.
#[derive(Clone, Copy)]
enum Action {
    /// A query answered from the host's Grid and kept from the terminal
    /// (§5.6).
    Answer,
    /// Forwarded, then followed by this, so the modes the host relies on
    /// stay on (conhost `_stream.cpp`, `adaptDispatch.cpp`): a nested
    /// ConPTY turns both off when it exits.
    Reassert(&'static [u8]),
}

/// DSR 5, CPR and DECXCPR; win32-input-mode off, focus reports off, RIS.
const WATCHED: [(&[u8], Action); 6] = [
    (b"\x1b[5n", Action::Answer),
    (b"\x1b[6n", Action::Answer),
    (b"\x1b[?6n", Action::Answer),
    (b"\x1b[?9001l", Action::Reassert(b"\x1b[?9001h")),
    (b"\x1b[?1004l", Action::Reassert(b"\x1b[?1004h")),
    (b"\x1bc", Action::Reassert(PREAMBLE)),
];

/// The CP437 glyphs conhost shows for C0 controls it does not act on.
const CP437_C0: [char; 32] = [
    ' ', '☺', '☻', '♥', '♦', '♣', '♠', '•', '◘', '○', '◙', '♂', '♀', '♪', '♫', '☼', //
    '►', '◄', '↕', '‼', '¶', '§', '▬', '↨', '↑', '↓', '→', '←', '∟', '↔', '▲', '▼',
];

pub(super) struct Screen {
    parser: AnsiParser,
    grid: Grid,
    /// The start of a watched sequence at the end of the last client
    /// text, kept until the next bytes decide it; shorter than any of
    /// them.
    held: Vec<u8>,
    /// The last byte of this write was CR, so an LF after it adds none.
    after_cr: bool,
    /// A UTF-8 lead 0xC2 held without PROCESSED_OUTPUT: the next byte
    /// decides whether it starts a C1 control.
    c2: bool,
}

impl Screen {
    pub(super) fn new(size: WinSize) -> Screen {
        let mut grid = Grid::new(usize::from(size.cols), usize::from(size.rows));
        grid.set_scrollback_max(0);
        Screen {
            parser: AnsiParser::new(),
            grid,
            held: Vec::new(),
            after_cr: false,
            c2: false,
        }
    }

    /// VT the host made itself (echo, cursor moves, modes): forwarded as
    /// it is. Held client bytes go first, so output keeps its order.
    pub(super) fn host(&mut self, vt: &[u8], out: &mut OutputQueue) {
        self.flush(out);
        self.after_cr = false;
        self.forward(vt, out);
    }

    /// Forward whatever client bytes are held, as they are: no more
    /// output follows, or host VT goes next.
    pub(super) fn flush(&mut self, out: &mut OutputQueue) {
        if std::mem::take(&mut self.c2) {
            self.held.push(0xC2);
        }
        let held = std::mem::take(&mut self.held);
        self.forward(&held, out);
    }

    /// Client text under output `mode`, transformed (§5.4) while its
    /// result fits in `room` bytes; `first` when it starts a write.
    /// Returns the bytes consumed; answers to the host's own queries are
    /// added to `replies`.
    pub(super) fn client(
        &mut self,
        text: &[u8],
        mode: u32,
        room: usize,
        first: bool,
        out: &mut OutputQueue,
        replies: &mut Vec<u8>,
    ) -> usize {
        if first {
            // conhost adds an LF's CR afresh in every write.
            self.after_cr = false;
        }
        let mut vt = std::mem::take(&mut self.held);
        let held = vt.len();
        let mut consumed = 0;
        for &b in text {
            let step = transform(b, mode, self.after_cr, self.c2);
            if vt.len() - held + step.len > room {
                break;
            }
            vt.extend_from_slice(&step.bytes[..step.len]);
            self.after_cr = b == b'\r';
            self.c2 = step.c2;
            consumed += 1;
        }
        if mode & VIRTUAL_TERMINAL_PROCESSING != 0 && mode & PROCESSED_OUTPUT != 0 {
            self.route_vt(&vt, out, replies);
        } else {
            self.forward_legacy(&vt, mode & WRAP_AT_EOL_OUTPUT != 0, out);
        }
        consumed
    }

    /// Forward `vt`, acting on the watched sequences in it.
    fn route_vt(&mut self, vt: &[u8], out: &mut OutputQueue, replies: &mut Vec<u8>) {
        let mut start = 0;
        let mut at = 0;
        while let Some(off) = vt[at..].iter().position(|&b| b == 0x1b) {
            let esc = at + off;
            match watched_at(&vt[esc..]) {
                Watched::Whole(len, Action::Answer) => {
                    self.forward(&vt[start..esc], out);
                    self.parser.advance(&mut self.grid, &vt[esc..esc + len]);
                    replies.extend(self.parser.take_responses());
                    start = esc + len;
                }
                Watched::Whole(len, Action::Reassert(modes)) => {
                    self.forward(&vt[start..esc + len], out);
                    self.forward(modes, out);
                    start = esc + len;
                }
                Watched::Prefix => {
                    self.forward(&vt[start..esc], out);
                    self.held = vt[esc..].to_vec();
                    return;
                }
                Watched::No => {
                    at = esc + 1;
                    continue;
                }
            }
            at = start;
        }
        self.forward(&vt[start..], out);
    }

    /// Forward legacy output. Under WRAP_AT_EOL_OUTPUT a glyph in the last
    /// column wraps at once, with CR LF, rather than waiting for the next
    /// glyph (conhost `WriteCharsLegacy`). Each piece is short enough
    /// that it cannot pass the margin: a byte moves the cursor at most
    /// two columns.
    fn forward_legacy(&mut self, vt: &[u8], wrap: bool, out: &mut OutputQueue) {
        if !wrap {
            return self.forward(vt, out);
        }
        let mut rest = vt;
        while !rest.is_empty() {
            let room = self.grid.cols.saturating_sub(self.grid.cursor.0 + 1) / 2;
            let (piece, tail) = rest.split_at(room.clamp(1, rest.len()));
            self.forward(piece, out);
            if self.grid.cursor.0 >= self.grid.cols {
                self.forward(b"\r\n", out);
            }
            rest = tail;
        }
    }

    /// Send `vt` to the terminal and the Grid; the Grid's own answers to
    /// queries in it are discarded, since the terminal answers them.
    fn forward(&mut self, vt: &[u8], out: &mut OutputQueue) {
        if vt.is_empty() {
            return;
        }
        out.push(vt);
        self.parser.advance(&mut self.grid, vt);
        self.parser.take_responses();
    }

    /// The terminal's new size (§5.7): nothing goes to the terminal.
    pub(super) fn resize(&mut self, size: WinSize) {
        self.grid
            .resize(usize::from(size.cols), usize::from(size.rows));
    }

    pub(super) fn size(&self) -> WinSize {
        WinSize {
            rows: self.grid.rows as u16,
            cols: self.grid.cols as u16,
        }
    }

    /// The cursor as (column, row), on the screen: a pending wrap reads
    /// as the last column.
    pub(super) fn cursor(&self) -> (u16, u16) {
        let (x, y) = self.grid.cursor;
        let last = |n: usize| n.saturating_sub(1);
        (
            x.min(last(self.grid.cols)) as u16,
            y.min(last(self.grid.rows)) as u16,
        )
    }

    pub(super) fn cursor_visible(&self) -> bool {
        self.grid.cursor_visible
    }

    /// The pen as a legacy attribute (§5.2).
    pub(super) fn attributes(&self) -> u16 {
        legacy(self.parser.pen_colors(), self.parser.palette())
    }

    /// The first 16 palette entries as `COLORREF`s (`0x00BBGGRR`), in
    /// Win32 order.
    pub(super) fn color_table(&self) -> [u32; 16] {
        let palette = self.parser.palette();
        std::array::from_fn(|i| colorref(palette[usize::from(swap(i as u8))]))
    }

    #[cfg(test)]
    pub(super) fn grid(&self) -> &Grid {
        &self.grid
    }

    #[cfg(test)]
    pub(super) fn palette(&self) -> &[u32; 256] {
        self.parser.palette()
    }
}

/// How the bytes at an ESC relate to the watched sequences.
enum Watched {
    Whole(usize, Action),
    /// A strict prefix of one: the next bytes decide.
    Prefix,
    No,
}

fn watched_at(rest: &[u8]) -> Watched {
    for (seq, action) in WATCHED {
        if rest.starts_with(seq) {
            return Watched::Whole(seq.len(), action);
        }
        if seq.starts_with(rest) {
            return Watched::Prefix;
        }
    }
    Watched::No
}

/// What one client byte becomes.
struct Step {
    bytes: [u8; 4],
    len: usize,
    /// The byte is a 0xC2 held for the next one.
    c2: bool,
}

impl Step {
    fn of(bytes: &[u8]) -> Step {
        let mut step = Step {
            bytes: [0; 4],
            len: bytes.len(),
            c2: false,
        };
        step.bytes[..bytes.len()].copy_from_slice(bytes);
        step
    }
}

/// One client byte under output `mode` (§5.4), at most 4 bytes out.
/// Only C0, DEL and, without PROCESSED_OUTPUT, the C1 controls change
/// (conhost `SanitizeUCS2`), so UTF-8 passes through whole however it
/// is sliced. An LF gains a CR unless one came just before it in this
/// write (`after_cr`). `c2` is a held 0xC2 lead, which a C1 control
/// replaces and anything else follows.
fn transform(b: u8, mode: u32, after_cr: bool, c2: bool) -> Step {
    let processed = mode & PROCESSED_OUTPUT != 0;
    let vt = processed && mode & VIRTUAL_TERMINAL_PROCESSING != 0;
    if c2 && (0x80..0xA0).contains(&b) {
        return Step::of(b"?");
    }
    let mut step = match b {
        0xC2 if !processed => Step {
            c2: true,
            ..Step::of(&[])
        },
        0x7F if !vt => Step::of("⌂".as_bytes()),
        0x20.. => Step::of(&[b]),
        _ if !processed => glyph(b),
        b'\n' if mode & DISABLE_NEWLINE_AUTO_RETURN == 0 && !after_cr => Step::of(b"\r\n"),
        _ if vt => Step::of(&[b]),
        0x07 | 0x08 | b'\t' | b'\n' | b'\r' => Step::of(&[b]),
        _ => glyph(b),
    };
    if c2 {
        // The held lead goes first; the byte after it is never 0xC2's
        // partner, so it takes at most 3 bytes.
        step.bytes.copy_within(..3, 1);
        step.bytes[0] = 0xC2;
        step.len += 1;
    }
    step
}

/// The CP437 glyph of C0 control `b`.
fn glyph(b: u8) -> Step {
    let mut bytes = [0u8; 4];
    let len = CP437_C0[usize::from(b)].encode_utf8(&mut bytes).len();
    Step::of(&bytes[..len])
}

/// ANSI index `n` (bit 0 red, bit 2 blue) as a Win32 colour nibble (bit 0
/// blue, bit 2 red), or back: bits 0 and 2 swap.
fn swap(n: u8) -> u8 {
    (n & 0b1010) | (n & 1) << 2 | (n >> 2) & 1
}

/// `0x00RRGGBB` as a `COLORREF`.
fn colorref(rgb: u32) -> u32 {
    (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16) & 0xFF
}

/// A colour as a Win32 nibble: the 16 ANSI colours exactly, others the
/// nearest of the live palette's first 16.
fn nibble(c: ColorRef, default: u8, palette: &[u32; 256]) -> u16 {
    let ansi = match c {
        ColorRef::Default => return u16::from(default),
        ColorRef::Palette(n) if n < 16 => n,
        ColorRef::Palette(n) => nearest(palette[usize::from(n)], palette),
        ColorRef::Rgb(rgb) => nearest(rgb, palette),
    };
    u16::from(swap(ansi))
}

/// The ANSI index of the first-16 palette entry closest to `rgb`.
fn nearest(rgb: u32, palette: &[u32; 256]) -> u8 {
    let channels = |c: u32| [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF].map(i64::from);
    let want = channels(rgb);
    let distance = |c: u32| {
        channels(c)
            .iter()
            .zip(want)
            .map(|(a, b)| (a - b).pow(2))
            .sum::<i64>()
    };
    (0..16u8)
        .min_by_key(|&i| distance(palette[usize::from(i)]))
        .unwrap_or(0)
}

/// `colors` as a legacy attribute: default reads as 7 on 0 (§5.2).
fn legacy(colors: CellColors, palette: &[u32; 256]) -> u16 {
    let mut attr = nibble(colors.fg, 7, palette) | nibble(colors.bg, 0, palette) << 4;
    if colors.flags & ATTR_REVERSE != 0 {
        attr |= LVB_REVERSE_VIDEO;
    }
    if colors.flags & ATTR_UNDERLINE != 0 {
        attr |= LVB_UNDERSCORE;
    }
    attr
}

/// SGR for a legacy attribute (§5.2): it replaces the whole rendition, so
/// it starts with a reset; 7 on 0 is the default colours.
pub(super) fn sgr(attr: u16) -> Vec<u8> {
    let fg = swap((attr & 0xF) as u8);
    let bg = swap((attr >> 4 & 0xF) as u8);
    let mut params = vec!["0".to_string()];
    // conhost's order (`VtIo` `formatAttributes`): reverse, then colours.
    if attr & LVB_REVERSE_VIDEO != 0 {
        params.push("7".into());
    }
    if fg != 7 {
        params.push(color_param(fg, 30, 90));
    }
    if bg != 0 {
        params.push(color_param(bg, 40, 100));
    }
    if attr & LVB_UNDERSCORE != 0 {
        params.push("4".into());
    }
    format!("\x1b[{}m", params.join(";")).into_bytes()
}

fn color_param(ansi: u8, base: u8, bright: u8) -> String {
    if ansi < 8 {
        (base + ansi).to_string()
    } else {
        (bright + ansi - 8).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_term_core::Cell;

    const VT: u32 = PROCESSED_OUTPUT | WRAP_AT_EOL_OUTPUT | VIRTUAL_TERMINAL_PROCESSING;

    fn screen() -> Screen {
        Screen::new(WinSize { rows: 4, cols: 10 })
    }

    /// Client text as one call: what the terminal got and the replies.
    fn client(s: &mut Screen, text: &[u8], mode: u32) -> (Vec<u8>, Vec<u8>) {
        let mut out = OutputQueue::default();
        let mut replies = Vec::new();
        assert_eq!(
            s.client(text, mode, usize::MAX, true, &mut out, &mut replies),
            text.len()
        );
        (out.take(), replies)
    }

    fn row(s: &Screen, y: usize) -> String {
        s.grid().row(y).iter().map(|c: &Cell| c.ch).collect()
    }

    #[test]
    fn vt_mode_passes_bytes_and_adds_cr_to_lf() {
        let mut s = screen();
        let (vt, _) = client(&mut s, b"a\x1b[31mb\nc\r", VT);
        assert_eq!(vt, b"a\x1b[31mb\r\nc\r");
        let (vt, _) = client(&mut s, b"\n\r\n", VT);
        assert_eq!(vt, b"\r\n\r\n", "only a CR in the same write counts");
        client(&mut s, b"\x1b[2;1H", VT);
        assert_eq!(row(&s, 1), "c         ");
        let (vt, _) = client(&mut s, b"\n", VT | DISABLE_NEWLINE_AUTO_RETURN);
        assert_eq!(vt, b"\n");
        assert_eq!(s.cursor(), (0, 2));
    }

    #[test]
    fn processed_without_vt_shows_esc_and_keeps_five_controls() {
        let mut s = screen();
        let (vt, _) = client(&mut s, b"\x1b[1m\x07\x08\t\r\x01", PROCESSED_OUTPUT);
        assert_eq!(vt, "←[1m\x07\x08\t\r☺".as_bytes());
        assert_eq!(row(&s, 0), "☺[1m      ");
    }

    #[test]
    fn unprocessed_output_shows_every_control_as_a_glyph() {
        let mut s = screen();
        let (vt, _) = client(&mut s, b"\x1b\n\r\x00", VT & !PROCESSED_OUTPUT);
        assert_eq!(vt, "←◙♪ ".as_bytes());
        assert_eq!(s.cursor(), (4, 0));
    }

    #[test]
    fn utf8_passes_whole() {
        let mut s = screen();
        let (vt, _) = client(&mut s, "é漢".as_bytes(), PROCESSED_OUTPUT);
        assert_eq!(vt, "é漢".as_bytes());
    }

    #[test]
    fn transform_stops_before_overflowing_room() {
        let mut s = screen();
        let mut out = OutputQueue::default();
        let mut replies = Vec::new();
        let n = s.client(
            b"ab\x1bc",
            PROCESSED_OUTPUT,
            4,
            true,
            &mut out,
            &mut replies,
        );
        assert_eq!(n, 2, "the ESC's three bytes would not fit");
        assert_eq!(out.take(), b"ab");
    }

    #[test]
    fn dsr_and_cpr_are_answered_and_not_forwarded() {
        let mut s = screen();
        let (vt, replies) = client(&mut s, b"ab\x1b[6n\x1b[5n\x1b[c\x1b[?6n", VT);
        assert_eq!(vt, b"ab\x1b[c");
        assert_eq!(replies, b"\x1b[1;3R\x1b[0n\x1b[?1;3;1R");
    }

    #[test]
    fn a_query_split_across_calls_is_still_stripped() {
        let mut s = screen();
        let (vt, replies) = client(&mut s, b"x\x1b[", VT);
        assert_eq!((vt.as_slice(), replies.as_slice()), (&b"x"[..], &b""[..]));
        let (vt, replies) = client(&mut s, b"6ny", VT);
        assert_eq!(vt, b"y");
        assert_eq!(replies, b"\x1b[1;2R");
    }

    #[test]
    fn a_held_prefix_that_is_no_query_is_forwarded_in_order() {
        let mut s = screen();
        client(&mut s, b"\x1b[", VT);
        let (vt, _) = client(&mut s, b"2J", VT);
        assert_eq!(vt, b"\x1b[2J");
        client(&mut s, b"\x1b", VT);
        let mut out = OutputQueue::default();
        s.host(b"echo", &mut out);
        assert_eq!(out.take(), b"\x1becho");
    }

    #[test]
    fn queries_are_forwarded_without_vt_processing() {
        let mut s = screen();
        let (vt, replies) = client(&mut s, b"\x1b[6n", PROCESSED_OUTPUT);
        assert_eq!(vt, "←[6n".as_bytes());
        assert!(replies.is_empty());
    }

    #[test]
    fn other_parser_answers_are_discarded() {
        let mut s = screen();
        let mut out = OutputQueue::default();
        s.host(b"\x1b[c\x1b[6n", &mut out);
        assert_eq!(out.take(), b"\x1b[c\x1b[6n");
    }

    #[test]
    fn input_modes_the_host_needs_are_turned_back_on() {
        let mut s = screen();
        let (vt, _) = client(&mut s, b"\x1b[?9001l\x1b[?1004l", VT);
        assert_eq!(vt, b"\x1b[?9001l\x1b[?9001h\x1b[?1004l\x1b[?1004h");
        let (vt, _) = client(&mut s, b"x\x1bc", VT);
        assert_eq!(vt, [&b"x\x1bc"[..], PREAMBLE].concat());
        // Split across writes, it still counts.
        assert_eq!(client(&mut s, b"\x1b[?90", VT).0, b"");
        assert_eq!(client(&mut s, b"01l", VT).0, b"\x1b[?9001l\x1b[?9001h");
    }

    #[test]
    fn legacy_output_wraps_at_once_at_the_margin() {
        let mut s = screen();
        let legacy = PROCESSED_OUTPUT | WRAP_AT_EOL_OUTPUT;
        let (vt, _) = client(&mut s, b"0123456789ab", legacy);
        assert_eq!(vt, b"0123456789\r\nab");
        assert_eq!(s.cursor(), (2, 1));
        let (vt, _) = client(&mut s, "\r漢字漢字漢".as_bytes(), legacy);
        assert_eq!(vt, "\r漢字漢字漢\r\n".as_bytes());
        assert_eq!(s.cursor(), (0, 2));
        // VT output keeps the delayed wrap, and WRAP off never wraps.
        let (vt, _) = client(&mut s, b"0123456789", VT);
        assert_eq!((vt.as_slice(), s.cursor()), (&b"0123456789"[..], (9, 2)));
        let (vt, _) = client(&mut s, b"\r0123456789ab", PROCESSED_OUTPUT);
        assert_eq!(vt, b"\r0123456789ab");
    }

    #[test]
    fn del_and_c1_show_as_glyphs_without_processing() {
        let mut s = screen();
        let (vt, _) = client(&mut s, "\u{7f}\u{9b}é".as_bytes(), 0);
        assert_eq!(vt, "⌂?é".as_bytes());
        // A C1 control split across writes, and DEL in each mode.
        assert_eq!(client(&mut s, b"\xc2", 0).0, b"");
        assert_eq!(client(&mut s, b"\x9b", 0).0, b"?");
        assert_eq!(client(&mut s, b"\x7f", PROCESSED_OUTPUT).0, "⌂".as_bytes());
        assert_eq!(client(&mut s, b"\x7f", VT).0, b"\x7f");
        assert_eq!(
            client(&mut s, "\u{9b}".as_bytes(), VT).0,
            "\u{9b}".as_bytes()
        );
    }

    #[test]
    fn host_vt_and_flush_send_held_bytes_first() {
        let mut s = screen();
        client(&mut s, b"\xc2", 0);
        let mut out = OutputQueue::default();
        s.flush(&mut out);
        assert_eq!(out.take(), b"\xc2");
        client(&mut s, b"\x1b[", VT);
        s.flush(&mut out);
        assert_eq!(out.take(), b"\x1b[");
    }

    #[test]
    fn swap_is_its_own_inverse_for_all_16() {
        for n in 0..16 {
            assert_eq!(swap(swap(n)), n);
        }
        assert_eq!(swap(1), 4, "ANSI red is Win32 4");
        assert_eq!(swap(12), 9, "bright blue");
    }

    #[test]
    fn attributes_read_the_pen() {
        let mut s = screen();
        assert_eq!(s.attributes(), 0x07);
        client(&mut s, b"\x1b[91;44;4;7m", VT);
        assert_eq!(
            s.attributes(),
            0x0C | 0x10 | LVB_UNDERSCORE | LVB_REVERSE_VIDEO
        );
        // Off the 16, the nearest entry: just off bright red, and black.
        let red = s.parser.palette()[9] ^ 1;
        let (r, g, b) = (red >> 16, red >> 8 & 0xFF, red & 0xFF);
        client(
            &mut s,
            format!("\x1b[0;38;2;{r};{g};{b};48;5;16m").as_bytes(),
            VT,
        );
        assert_eq!(s.attributes(), 0x0C);
    }

    #[test]
    fn sgr_round_trips_through_the_parser() {
        for attr in [
            0x07,
            0x00,
            0x1F,
            0xC4 | LVB_UNDERSCORE,
            0x70 | LVB_REVERSE_VIDEO,
        ] {
            let mut s = screen();
            client(&mut s, &sgr(attr), VT);
            assert_eq!(s.attributes(), attr, "{attr:#06x}");
        }
        assert_eq!(sgr(0x07), b"\x1b[0m");
        assert_eq!(sgr(0x4E), b"\x1b[0;93;41m");
        // conhost's own order: VtIoTests::SetConsoleTextAttribute.
        assert_eq!(sgr(0x2D | LVB_REVERSE_VIDEO), b"\x1b[0;7;95;42m");
    }

    #[test]
    fn color_table_is_in_win32_order() {
        let mut s = screen();
        client(&mut s, b"\x1b]4;1;rgb:12/34/56\x1b\\", VT);
        assert_eq!(s.color_table()[4], 0x0056_3412);
    }

    #[test]
    fn resize_changes_the_reported_size_and_emits_nothing() {
        let mut s = screen();
        s.resize(WinSize { rows: 2, cols: 5 });
        assert_eq!(s.size(), WinSize { rows: 2, cols: 5 });
    }
}
