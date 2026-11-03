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
//! once with CR LF, as conhost's legacy path does; with wrap off, the
//! cursor goes back to the column the run of text started at instead,
//! and the rest of it overwrites the row from there.
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/host/getset.cpp`, `_stream.cpp`, `src/host/VtIo.cpp`,
//! `src/terminal/adapter/adaptDispatch.cpp`; see THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use platform::term::WinSize;
use rusty_term_core::{
    ATTR_BOLD, ATTR_OVERLINE, ATTR_REVERSE, AnsiParser, CellColors, ColorRef, Grid, WIDE_TRAILER,
    char_width,
};

use super::cells::{CharInfo, LEADING, LT, TRAILING, sanitize};
use super::{OutputQueue, PREAMBLE};

/// `ENABLE_*` output mode bits.
pub(super) const PROCESSED_OUTPUT: u32 = 0x1;
pub(super) const WRAP_AT_EOL_OUTPUT: u32 = 0x2;
pub(super) const VIRTUAL_TERMINAL_PROCESSING: u32 = 0x4;
pub(super) const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x8;

/// `COMMON_LVB_*` attribute bits.
const FOREGROUND_INTENSITY: u16 = 0x8;
const LVB_GRID_HORIZONTAL: u16 = 0x0400;
const LVB_REVERSE_VIDEO: u16 = 0x4000;
const LVB_UNDERSCORE: u16 = 0x8000;
/// The gridlines no SGR maps to: left, right and bottom (conhost's
/// `BottomGridline` is `COMMON_LVB_UNDERSCORE`, apart from SGR 4). The
/// top one is overline, SGR 53.
const LVB_GRID: u16 = 0x0800 | 0x1000 | LVB_UNDERSCORE;
/// conhost's default colour table ("Campbell"), as `0x00RRGGBB` in ANSI
/// order.
const CAMPBELL: [u32; 16] = [
    0x0C0C0C, 0xC50F1F, 0x13A10E, 0xC19C00, 0x0037DA, 0x881798, 0x3A96DD, 0xCCCCCC, //
    0x767676, 0xE74856, 0x16C60C, 0xF9F1A5, 0x3B78FF, 0xB4009E, 0x61D6D6, 0xF2F2F2,
];
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

/// The longest CSI prefix the host holds: the parameters rusty_term_core
/// stores ([`CSI_PARAM_MAX`]), `ESC [ ?` and the final byte. A longer
/// DEC private one is forwarded as it comes and decided at its end.
pub(super) const WATCHED_MAX: usize = CSI_PARAM_MAX + 4;
/// The longest CHA the host sends: `ESC [ 65535 G`.
const CHA_MAX: usize = 8;
/// The most bytes one client byte (or UTF-16 unit) can come to: RIS's
/// two add the 16 of [`PREAMBLE`]. A legacy glyph is at most 3; a wrap
/// adds 2, and without wrap a margin adds a pad and a CHA. Over a run
/// that is an average: the one glyph that costs more (pad, CHA, glyph,
/// CHA) follows a narrow one that met no margin.
pub(super) const MAX_EXPANSION: usize = {
    let reset = (2 + PREAMBLE.len()) / 2;
    let margin = 3 + 1 + CHA_MAX;
    if reset > margin { reset } else { margin }
};

/// The CP437 glyphs conhost shows for C0 controls it does not act on.
pub(super) const CP437_C0: [char; 32] = [
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
    /// Inside a DEC private CSI too long to hold, forwarded as it came:
    /// its final byte decides whether the input modes are reasserted.
    long: bool,
    /// The gridline bits of the last SetConsoleTextAttribute, which no
    /// SGR sets; they hold until the model applies a rendition reset
    /// ([`AnsiParser::take_rendition_reset`]).
    legacy_meta: u16,
    /// The column the current run of legacy text started at, where the
    /// cursor goes back to when the run meets the margin with wrap off
    /// (conhost `_writeCharsLegacyUnprocessed`). A run is a whole write
    /// without PROCESSED_OUTPUT, else the text between controls.
    run: Option<usize>,
    /// Legacy text met the margin at the end of what the write has
    /// emitted so far: the move it owes waits for the next slice, whose
    /// marks may still join the glyph, or for the write's end.
    owed: Option<Margin>,
    /// Bytes sent to the terminal so far: a cursor sample taken at one
    /// count is stale once more has gone out (#1375).
    emitted: u64,
}

/// Where the cursor goes when legacy text meets the margin.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Margin {
    /// With wrap: CR LF.
    Wrap,
    /// Without: back to the run's start column, by CHA.
    Back(usize),
}

impl Screen {
    pub(super) fn new(size: WinSize) -> Screen {
        let mut grid = Grid::new(usize::from(size.cols), usize::from(size.rows));
        grid.set_scrollback_max(0);
        let mut screen = Screen {
            parser: AnsiParser::new(),
            grid,
            held: Vec::new(),
            after_cr: false,
            c2: false,
            long: false,
            legacy_meta: 0,
            run: None,
            owed: None,
            emitted: 0,
        };
        // conhost's colour table, which GetConsoleScreenBufferInfoEx
        // reports and legacy attributes are matched against. Only the
        // model's palette changes; the terminal keeps its own.
        screen.feed(&campbell_osc4());
        screen
    }

    /// VT the host made itself (echo, cursor moves, modes): forwarded as
    /// it is. Held client bytes go first, so output keeps its order.
    pub(super) fn host(&mut self, vt: &[u8], out: &mut OutputQueue) {
        self.flush(out);
        self.after_cr = false;
        // Whatever the client's unfinished sequence was, this ends it.
        self.long = false;
        self.forward(vt, out);
    }

    /// Forward whatever client bytes are held, as they are: no more
    /// output follows, or host VT goes next.
    pub(super) fn flush(&mut self, out: &mut OutputQueue) {
        self.end_write(out);
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
            self.end_write(out);
            // conhost adds an LF's CR afresh in every write.
            self.after_cr = false;
            self.run = None;
        }
        if mode & VIRTUAL_TERMINAL_PROCESSING != 0 && mode & PROCESSED_OUTPUT != 0 {
            self.client_vt(text, mode, room, out, replies)
        } else {
            self.client_legacy(text, mode, room, out)
        }
    }

    /// Client VT: transformed while it fits in `room` together with the
    /// modes a watched reset adds after it, then routed.
    fn client_vt(
        &mut self,
        text: &[u8],
        mode: u32,
        room: usize,
        out: &mut OutputQueue,
        replies: &mut Vec<u8>,
    ) -> usize {
        let mut vt = std::mem::take(&mut self.held);
        let held = vt.len();
        // The ESC of the sequence the bytes are in, while it may be watched.
        let mut esc = (held > 0).then_some(0);
        let mut long = self.long;
        let mut added = 0;
        let mut consumed = 0;
        for &b in text {
            let step = transform(b, mode, self.after_cr, self.c2);
            let start = vt.len();
            vt.extend_from_slice(&step.bytes[..step.len]);
            if b == 0x1b {
                esc = Some(start);
            }
            let mut adds = 0;
            // A parameter byte only lengthens the prefix: the sequence is
            // matched once, at its start and at its final byte.
            let inside = csi_byte(b) == CsiByte::Inside;
            if long && !inside {
                long = false;
                if b == b'l' {
                    adds = PREAMBLE.len();
                }
            }
            let inside = inside && vt.len() - start == 1;
            if let Some(e) = esc.filter(|&e| !inside || vt.len() - e <= 3) {
                match watched_at(&vt[e..]) {
                    Watched::Prefix | Watched::Long => {}
                    Watched::Whole(_, Action::Reassert(modes)) => (adds, esc) = (modes.len(), None),
                    Watched::Whole(..) | Watched::No => esc = None,
                }
            }
            if vt.len() - held + added + adds > room {
                vt.truncate(start);
                break;
            }
            added += adds;
            self.after_cr = b == b'\r';
            self.c2 = step.c2;
            consumed += 1;
        }
        self.route_vt(&vt, out, replies);
        consumed
    }

    /// Legacy client text, forwarded a piece at a time so that what a
    /// margin adds counts against `room` too: CR LF with wrap, else a CHA
    /// back to the run's start.
    fn client_legacy(
        &mut self,
        text: &[u8],
        mode: u32,
        room: usize,
        out: &mut OutputQueue,
    ) -> usize {
        let wrap = mode & WRAP_AT_EOL_OUTPUT != 0;
        // conhost writes each control on its own, between runs of text.
        let control = |b: u8| mode & PROCESSED_OUTPUT != 0 && (b < 0x20 || b == 0x7F);
        let held = std::mem::take(&mut self.held);
        self.forward(&held, out);
        let mut emitted = 0;
        if self.owed.is_some() && !text.is_empty() && !continues(text) {
            if CHA_MAX > room {
                return 0;
            }
            emitted += self.end_write(out);
        }
        let mut consumed = 0;
        while consumed < text.len() {
            let before = consumed;
            if !wrap {
                if control(text[consumed]) {
                    self.run = None;
                }
                let last = self.grid.cols.saturating_sub(1);
                let start = *self.run.get_or_insert(self.grid.cursor.0.min(last));
                if let Some(len) = self.wide_at_margin(&text[consumed..]) {
                    if emitted + 1 + CHA_MAX > room {
                        break;
                    }
                    self.forward(b" ", out);
                    emitted += 1 + self.margin(Margin::Back(start), out);
                    if start == last {
                        // It can't fit at the run's start either, so
                        // conhost throws it away.
                        consumed += len;
                    }
                }
            }
            // Each byte moves the cursor at most two columns, so a piece
            // this long cannot pass the margin; only a single byte near it
            // can meet it, and needs room for what the margin adds.
            let piece = (self.grid.cols.saturating_sub(self.grid.cursor.0 + 1) / 2).max(1);
            let near = self.grid.cursor.0 + 2 >= self.grid.cols;
            let reserve = match (near, wrap) {
                (false, _) => 0,
                (true, true) => 2,
                (true, false) => CHA_MAX,
            };
            let mut vt = Vec::new();
            let mut ends_run = false;
            for &b in &text[consumed..] {
                if !wrap && control(b) && !vt.is_empty() {
                    break;
                }
                let step = transform(b, mode, self.after_cr, self.c2);
                let fits = emitted + vt.len() + step.len + reserve <= room;
                if !fits || (!vt.is_empty() && vt.len() + step.len > piece) {
                    break;
                }
                vt.extend_from_slice(&step.bytes[..step.len]);
                self.after_cr = b == b'\r';
                self.c2 = step.c2;
                consumed += 1;
                if !wrap && control(b) {
                    ends_run = true;
                    break;
                }
            }
            if consumed == before {
                break;
            }
            emitted += vt.len();
            self.forward(&vt, out);
            // conhost writes a glyph with the marks that follow it before
            // it meets the margin, so they join it, not the next row's;
            // the next slice may still hold some.
            // A control is written alone, so no mark joins it: its move
            // is made at once. Another's destination is fixed now, as the
            // run may end before it is made.
            let rest = &text[consumed..];
            if self.grid.cursor.0 >= self.grid.cols && !self.c2 {
                self.owed = Some(if wrap {
                    Margin::Wrap
                } else {
                    Margin::Back(self.run.unwrap_or(0))
                });
                let lone = control(text[consumed - 1]);
                if lone || (!rest.is_empty() && !continues(rest)) {
                    emitted += self.end_write(out);
                }
            }
            if ends_run {
                self.run = None;
            }
        }
        consumed
    }

    /// The write is done: make the margin move it owes. Returns the
    /// bytes sent.
    pub(super) fn end_write(&mut self, out: &mut OutputQueue) -> usize {
        match self.owed.take() {
            Some(margin) => self.margin(margin, out),
            None => 0,
        }
    }

    /// The cursor met the margin: send it where `margin` says. Returns
    /// the bytes sent.
    fn margin(&mut self, margin: Margin, out: &mut OutputQueue) -> usize {
        let vt = match margin {
            Margin::Wrap => b"\r\n".to_vec(),
            Margin::Back(start) => format!("\x1b[{}G", start + 1).into_bytes(),
        };
        self.forward(&vt, out);
        vt.len()
    }

    /// The length of a double-width glyph at `text`'s start when it
    /// would cross the margin: conhost then pads the last column with a
    /// space and writes the glyph at the run's start instead.
    fn wide_at_margin(&self, text: &[u8]) -> Option<usize> {
        let at_last = self.grid.cursor.0 + 1 == self.grid.cols;
        if self.c2 || !at_last {
            return None;
        }
        wide_len(text)
    }

    /// Forward `vt`, acting on the watched sequences in it.
    fn route_vt(&mut self, vt: &[u8], out: &mut OutputQueue, replies: &mut Vec<u8>) {
        if self.long {
            let Some(end) = vt.iter().position(|&b| csi_byte(b) != CsiByte::Inside) else {
                return self.forward(vt, out);
            };
            self.long = false;
            // An ESC ends it unfinished, and starts a sequence of its own.
            let fin = usize::from(vt[end] != 0x1b);
            self.forward(&vt[..end + fin], out);
            if vt[end] == b'l' {
                self.forward(PREAMBLE, out);
            }
            return self.route_vt(&vt[end + fin..], out, replies);
        }
        let mut start = 0;
        let mut at = 0;
        while let Some(off) = vt[at..].iter().position(|&b| b == 0x1b) {
            let esc = at + off;
            match watched_at(&vt[esc..]) {
                Watched::Whole(len, Action::Answer) => {
                    self.forward(&vt[start..esc], out);
                    replies.extend(self.feed(&vt[esc..esc + len]));
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
                Watched::Long => {
                    self.forward(&vt[start..], out);
                    self.long = true;
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

    /// Send `vt` to the terminal and the Grid; the Grid's own answers to
    /// queries in it are discarded, since the terminal answers them.
    fn forward(&mut self, vt: &[u8], out: &mut OutputQueue) {
        if vt.is_empty() {
            return;
        }
        out.push(vt);
        self.emitted += vt.len() as u64;
        self.feed(vt);
    }

    /// Advance the Grid over `vt` and return its answers. What it relays
    /// to its own host (OSC 52, BEL, input modes) is dropped: `vt` itself
    /// already goes to the terminal, so keeping it would only grow.
    fn feed(&mut self, vt: &[u8]) -> Vec<u8> {
        self.parser.advance(&mut self.grid, vt);
        if self.parser.take_rendition_reset() {
            self.legacy_meta = 0;
        }
        self.grid.take_host_out();
        self.parser.take_responses()
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

    /// Bytes sent to the terminal so far.
    pub(super) fn emitted(&self) -> u64 {
        self.emitted
    }

    /// Move the cursor to screen position (`x`, `y`), where the terminal
    /// reports its own (#1375); nothing is emitted. A position the cursor
    /// already reads as keeps its pending wrap.
    pub(super) fn place_cursor(&mut self, x: u16, y: u16) {
        if self.cursor() != (x, y) {
            self.grid.set_cursor(usize::from(x), usize::from(y));
        }
    }

    /// CUP to screen position (`x`, `y`), for the Grid as it is now. The
    /// Win32 position is absolute, so under the client's origin mode
    /// (DECOM) it is made relative to the region; a position outside the
    /// region can't be reached with DECOM on, so origin mode goes off
    /// first.
    pub(super) fn absolute_cup(&self, x: u16, y: u16) -> Vec<u8> {
        let (x, y) = (usize::from(x), usize::from(y));
        let cup = |col: usize, row: usize| format!("\x1b[{};{}H", row + 1, col + 1);
        match self.grid.origin_region() {
            None => cup(x, y),
            Some((l, t, r, b)) if (l..=r).contains(&x) && (t..=b).contains(&y) => cup(x - l, y - t),
            Some(_) => format!("\x1b[?6l{}", cup(x, y)),
        }
        .into_bytes()
    }

    pub(super) fn cursor_visible(&self) -> bool {
        self.grid.cursor_visible
    }

    /// The pen as a legacy attribute (§5.2).
    pub(super) fn attributes(&self) -> u16 {
        legacy(self.parser.pen_colors(), self.parser.palette()) | self.legacy_meta
    }

    /// SetConsoleTextAttribute's pen, as SGR the host sends.
    pub(super) fn set_attributes(&mut self, attr: u16, out: &mut OutputQueue) {
        self.host(&sgr(attr), out);
        // After the SGR, whose own reset would clear them.
        self.legacy_meta = attr & LVB_GRID;
    }

    /// Cells `x0..x1` of screen row `y` as conhost's `AsCharInfo` reports
    /// them: a wide character's two cells both carry it, flagged leading
    /// and trailing, and a cell that is more than one UTF-16 unit reads
    /// as U+FFFD (§5.3). The caller keeps the range on the screen.
    pub(super) fn cells(&self, y: usize, x0: usize, x1: usize) -> Vec<CharInfo> {
        let row = self.grid.row(y);
        let palette = self.parser.palette();
        let trailer = |x: usize| row.get(x).is_some_and(|c| c.flags & WIDE_TRAILER != 0);
        (x0..x1.min(row.len()))
            .map(|x| {
                let (lead, flag) = match (trailer(x), trailer(x + 1)) {
                    (true, _) if x > 0 => (x - 1, TRAILING),
                    (false, true) => (x, LEADING),
                    _ => (x, 0),
                };
                let cell = row[lead];
                let ch = match cell.ch {
                    '\0' => u16::from(b' '),
                    _ if cell.cluster != 0 => 0xFFFD,
                    c => u16::try_from(u32::from(c)).unwrap_or(0xFFFD),
                };
                let attr = self
                    .grid
                    .cell_colors(x, y)
                    .map_or(0, |c| legacy(c, palette));
                CharInfo {
                    ch,
                    attr: attr | flag,
                }
            })
            .collect()
    }

    /// Paint `cells` from (`x`, `y`) as conhost's `VtIo::Writer`
    /// (`WriteInfos`) does: CUP, then SGR wherever the attribute changes,
    /// and each character once, a wide one from its leading cell. A
    /// leading cell at the end or a trailing one at the start can't hold
    /// its half and shows a space. All of it sits between DECSC and DECRC,
    /// as conhost's `BackupCursor` puts it, so the cursor, pen (with its
    /// gridlines) and character sets stay. Inside, ASCII is selected so
    /// each glyph prints as itself, origin mode goes off so the CUP is
    /// absolute, and insert mode goes off so cells are replaced, not
    /// shifted; the modes come back after.
    pub(super) fn paint(&mut self, x: usize, y: usize, cells: &[CharInfo], out: &mut OutputQueue) {
        let Some(last) = cells.len().checked_sub(1) else {
            return;
        };
        let origin = self.grid.origin_region().is_some();
        let insert = self.grid.insert_mode();
        let mut vt = Vec::with_capacity(40 + cells.len() * 4);
        // ASCII in G0, invoked into GL: DECRC restores the client's sets.
        vt.extend_from_slice(b"\x1b7\x1b(B\x0f");
        if origin {
            vt.extend_from_slice(b"\x1b[?6l");
        }
        if insert {
            vt.extend_from_slice(b"\x1b[4l");
        }
        vt.extend(format!("\x1b[{};{}H", y + 1, x + 1).bytes());
        let mut pen = None;
        for (i, c) in cells.iter().enumerate() {
            let mut wide = c.attr & LT != 0;
            let mut ch = c.ch;
            if c.attr & LEADING != 0 && i == last || c.attr & TRAILING != 0 && i == 0 {
                (ch, wide) = (u16::from(b' '), false);
            } else if c.attr & TRAILING != 0 {
                continue;
            }
            let attr = c.attr & !LT;
            if pen != Some(attr) {
                pen = Some(attr);
                vt.extend(sgr(attr));
            }
            // A control or lone surrogate is a narrow glyph: a wide cell
            // shows it twice, as conhost does.
            let narrow = (0xD800..0xE000).contains(&ch) || ch < 0x20 || (0x7F..0xA0).contains(&ch);
            let glyph = sanitize(ch);
            let mut buf = [0; 4];
            let glyph = glyph.encode_utf8(&mut buf).as_bytes();
            vt.extend_from_slice(glyph);
            if wide && narrow {
                vt.extend_from_slice(glyph);
            }
        }
        if insert {
            vt.extend_from_slice(b"\x1b[4h");
        }
        if origin {
            vt.extend_from_slice(b"\x1b[?6h");
        }
        vt.extend_from_slice(b"\x1b8");
        // Held client bytes go first and may reset the pen themselves; the
        // paint's own SGR 0 is not the client's, and DECRC can't restore
        // the gridlines, so they come back by hand.
        self.flush(out);
        let meta = self.legacy_meta;
        self.host(&vt, out);
        self.legacy_meta = meta;
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
    /// A DEC private CSI's parameters, too long to hold: forward them and
    /// decide at the final byte.
    Long,
    No,
}

/// Match `rest`, which starts at an ESC, against the watched sequences:
/// RIS; DSR 5, CPR and DECXCPR; a DEC private mode reset (DECRST) whose
/// parameters include win32-input-mode (9001) or focus reports (1004),
/// however many others it has.
fn watched_at(rest: &[u8]) -> Watched {
    match rest.get(1) {
        None => Watched::Prefix,
        Some(b'c') => Watched::Whole(2, Action::Reassert(PREAMBLE)),
        Some(b'[') => watched_csi(rest),
        Some(_) => Watched::No,
    }
}

fn watched_csi(rest: &[u8]) -> Watched {
    let private = rest.get(2) == Some(&b'?');
    let first = if private { 3 } else { 2 };
    // The parameters as rusty_term_core keeps them, without the controls
    // it runs in place.
    let mut stored = Vec::new();
    let mut plain = true;
    let mut end = None;
    for (i, &b) in rest.iter().enumerate().skip(first) {
        match csi_byte(b) {
            CsiByte::Inside if is_param(b) => {
                if stored.len() < CSI_PARAM_MAX {
                    stored.push(b);
                }
            }
            CsiByte::Inside => plain = false,
            CsiByte::Final => {
                end = Some(i);
                break;
            }
            CsiByte::Ends => return Watched::No,
        }
    }
    let Some(end) = end else {
        return match (rest.len() < WATCHED_MAX, private) {
            (true, _) => Watched::Prefix,
            (false, true) => Watched::Long,
            (false, false) => Watched::No,
        };
    };
    if end >= WATCHED_MAX {
        // Past rusty_term_core's cap, and the terminal's may differ: either
        // mode may be named in what it applies, so reassert both.
        return match (private, rest[end]) {
            (true, b'l') => Watched::Whole(end + 1, Action::Reassert(PREAMBLE)),
            _ => Watched::No,
        };
    }
    let params: Vec<u32> = stored
        .split(|&b| b == b';')
        .map(|p| {
            std::str::from_utf8(p)
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(0)
        })
        .collect();
    let action = match (private, rest[end], params.as_slice()) {
        (false, b'n', [5 | 6]) | (true, b'n', [6]) if plain => Action::Answer,
        (true, b'l', _) => match (params.contains(&9001), params.contains(&1004)) {
            (true, true) => Action::Reassert(PREAMBLE),
            (true, false) => Action::Reassert(b"\x1b[?9001h"),
            (false, true) => Action::Reassert(b"\x1b[?1004h"),
            (false, false) => return Watched::No,
        },
        _ => return Watched::No,
    };
    Watched::Whole(end + 1, action)
}

/// rusty_term_core's `CSI_PARAM_MAX`: the parameter bytes it keeps.
const CSI_PARAM_MAX: usize = 256;

/// What a byte does inside a CSI, as rusty_term_core's parser reads it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CsiByte {
    /// The CSI goes on: a parameter, marker or intermediate byte, DEL, or
    /// a C0 control the parser runs in place.
    Inside,
    Final,
    /// CAN, SUB, ESC or a byte past 0x7F: the CSI ends without dispatch.
    Ends,
}

fn csi_byte(b: u8) -> CsiByte {
    match b {
        0x18 | 0x1a | 0x1b | 0x80.. => CsiByte::Ends,
        0x40..=0x7e => CsiByte::Final,
        _ => CsiByte::Inside,
    }
}

/// A CSI parameter byte, as the parser stores them.
fn is_param(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b';' | b':')
}

/// Whether `text` goes on with the glyph before it: the rest of a UTF-8
/// sequence, or a mark that joins it.
fn continues(text: &[u8]) -> bool {
    match text.first() {
        Some(0x80..=0xBF) => true,
        Some(_) => std::str::from_utf8(&text[..text.len().min(4)])
            .or_else(|e| std::str::from_utf8(&text[..e.valid_up_to()]))
            .ok()
            .and_then(|s| s.chars().next())
            .is_some_and(joins),
        None => false,
    }
}

/// Whether `ch` joins the glyph on its left (rusty_term_core's default
/// widths: a zero-width scalar), not counting C1 controls.
fn joins(ch: char) -> bool {
    ch >= '\u{a0}' && char_width(ch) == 0
}

/// The UTF-8 length of a double-width glyph at `text`'s start. Those
/// start at U+1100, so they take 3 or 4 bytes.
fn wide_len(text: &[u8]) -> Option<usize> {
    let n = match text.first()? {
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let ch = std::str::from_utf8(text.get(..n)?).ok()?.chars().next()?;
    (char_width(ch) == 2).then_some(n)
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

/// OSC 4 setting the first 16 palette entries to [`CAMPBELL`].
fn campbell_osc4() -> Vec<u8> {
    let mut vt = Vec::new();
    for (i, rgb) in CAMPBELL.iter().enumerate() {
        let (r, g, b) = (rgb >> 16, rgb >> 8 & 0xFF, rgb & 0xFF);
        vt.extend(format!("\x1b]4;{i};rgb:{r:02x}/{g:02x}/{b:02x}\x1b\\").bytes());
    }
    vt
}

/// `colors` as a legacy attribute: default reads as 7 on 0 (§5.2). As
/// conhost's `GetLegacyAttributes`: bold brightens a default or 16-colour
/// foreground, reverse and overline are meta bits, underline is none.
fn legacy(colors: CellColors, palette: &[u32; 256]) -> u16 {
    let mut attr = nibble(colors.fg, 7, palette) | nibble(colors.bg, 0, palette) << 4;
    let brightens = matches!(colors.fg, ColorRef::Default | ColorRef::Palette(0..16));
    if colors.flags & ATTR_BOLD != 0 && brightens {
        attr |= FOREGROUND_INTENSITY;
    }
    if colors.flags & ATTR_REVERSE != 0 {
        attr |= LVB_REVERSE_VIDEO;
    }
    if colors.flags & ATTR_OVERLINE != 0 {
        attr |= LVB_GRID_HORIZONTAL;
    }
    attr
}

/// SGR for a legacy attribute (§5.2): it replaces the whole rendition, so
/// it starts with a reset; 7 on 0 is the default colours.
fn sgr(attr: u16) -> Vec<u8> {
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
    if attr & LVB_GRID_HORIZONTAL != 0 {
        params.push("53".into());
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
        s.end_write(&mut out);
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
    fn the_grids_own_relays_are_dropped() {
        let mut s = screen();
        for _ in 0..1000 {
            client(&mut s, b"\x07\x1b[?2004h\x1b]52;c;aGk=\x07", VT);
        }
        assert!(s.grid.take_host_out().is_empty(), "nothing retained");
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
    fn compound_mode_resets_are_watched_too() {
        let mut s = screen();
        let (vt, _) = client(&mut s, b"\x1b[?1004;9001l", VT);
        assert_eq!(vt, [&b"\x1b[?1004;9001l"[..], PREAMBLE].concat());
        // The other parameters still take effect.
        let (vt, _) = client(&mut s, b"\x1b[?25;1004l", VT);
        assert_eq!(vt, b"\x1b[?25;1004l\x1b[?1004h");
        assert!(!s.cursor_visible());
        assert_eq!(client(&mut s, b"\x1b[?25;10", VT).0, b"");
        assert_eq!(client(&mut s, b"04lz", VT).0, b"\x1b[?25;1004l\x1b[?1004hz");
        // Resets without them, and sets, are not.
        assert_eq!(
            client(&mut s, b"\x1b[?25l\x1b[?1004h", VT).0,
            b"\x1b[?25l\x1b[?1004h"
        );
        assert_eq!(client(&mut s, b"\x1b[?10041l", VT).0, b"\x1b[?10041l");
    }

    #[test]
    fn long_compound_resets_are_watched_up_to_the_parsers_cap() {
        let mut s = screen();
        let long = [&b"\x1b[?"[..], &b"25;".repeat(10), b"1004l"].concat();
        let (vt, _) = client(&mut s, &long, VT);
        assert_eq!(vt, [&long[..], b"\x1b[?1004h"].concat());
        let max = [&b"\x1b[?"[..], &b"0;".repeat(125), b"1004l"].concat();
        assert!(max.len() <= WATCHED_MAX);
        let (vt, _) = client(&mut s, &max, VT);
        assert!(vt.ends_with(b"1004l\x1b[?1004h"), "{} bytes", max.len());
        // Past the parser's cap it still applies the parameters it kept,
        // wherever 1004 is, so both modes are reasserted.
        for past in [
            [&b"\x1b[?"[..], &b"0;".repeat(130), b"1004l"].concat(),
            [&b"\x1b[?1004;"[..], &b"0;".repeat(130), b"l"].concat(),
        ] {
            assert!(past.len() > WATCHED_MAX);
            assert_eq!(client(&mut s, &past, VT).0, [&past[..], PREAMBLE].concat());
        }
        // Split past the cap: forwarded as it comes, decided at its end.
        let head = [&b"\x1b[?1004;"[..], &b"0;".repeat(130)].concat();
        assert_eq!(client(&mut s, &head, VT).0, head, "not held");
        assert_eq!(client(&mut s, b"0;0", VT).0, b"0;0");
        assert_eq!(
            client(&mut s, b"lz", VT).0,
            [&b"l"[..], PREAMBLE, b"z"].concat()
        );
        // A long set, or a sequence an ESC cuts short, reasserts nothing.
        let set = [&b"\x1b[?1004;"[..], &b"0;".repeat(130)].concat();
        client(&mut s, &set, VT);
        assert_eq!(client(&mut s, b"h", VT).0, b"h");
        client(&mut s, &set, VT);
        assert_eq!(
            client(&mut s, b"\x1b[?1004l", VT).0,
            [&b"\x1b[?1004l"[..], b"\x1b[?1004h"].concat()
        );
    }

    #[test]
    fn controls_run_inside_a_csi_do_not_end_its_watch() {
        let mut s = screen();
        // Held within the cap: BEL runs in place, the reset still counts.
        assert_eq!(client(&mut s, b"\x1b[?10\x07", VT).0, b"");
        assert_eq!(client(&mut s, b"04l", VT).0, b"\x1b[?10\x0704l\x1b[?1004h");
        // Past the cap, across writes.
        let head = [&b"\x1b[?1004;"[..], &b"0;".repeat(130)].concat();
        assert_eq!(client(&mut s, &head, VT).0, head);
        assert_eq!(client(&mut s, b"\x07", VT).0, b"\x07");
        assert_eq!(client(&mut s, b"l", VT).0, [&b"l"[..], PREAMBLE].concat());
        // CAN cancels it, so its `l` is text.
        client(&mut s, &head, VT);
        assert_eq!(client(&mut s, b"\x18l", VT).0, b"\x18l");
        assert_eq!(client(&mut s, b"\x1b[?1004\x18l", VT).0, b"\x1b[?1004\x18l");
    }

    #[test]
    fn room_counts_the_modes_a_reset_adds() {
        let mut s = screen();
        let mut out = OutputQueue::default();
        let mut replies = Vec::new();
        let n = s.client(
            b"\x1bc\x1bc",
            VT,
            2 + PREAMBLE.len() + 1,
            true,
            &mut out,
            &mut replies,
        );
        assert_eq!(n, 3, "the second RIS's c and its modes would not fit");
        assert_eq!(
            out.take(),
            [&b"\x1bc"[..], PREAMBLE].concat(),
            "its ESC is held"
        );
    }

    #[test]
    fn room_counts_the_cr_lf_a_legacy_wrap_adds() {
        let mut s = Screen::new(WinSize { rows: 4, cols: 1 });
        let mut out = OutputQueue::default();
        let mut replies = Vec::new();
        let legacy = PROCESSED_OUTPUT | WRAP_AT_EOL_OUTPUT;
        let n = s.client(b"abc", legacy, 8, true, &mut out, &mut replies);
        assert_eq!(n, 2, "a third glyph and its CR LF would not fit");
        assert_eq!(out.take(), b"a\r\nb\r\n");
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
        // VT output keeps the delayed wrap, and WRAP off goes back to the
        // run's start.
        let (vt, _) = client(&mut s, b"0123456789", VT);
        assert_eq!((vt.as_slice(), s.cursor()), (&b"0123456789"[..], (9, 2)));
        let (vt, _) = client(&mut s, b"\r0123456789ab", PROCESSED_OUTPUT);
        assert_eq!(vt, b"\r0123456789\x1b[1Gab");
        assert_eq!((row(&s, 2), s.cursor()), ("ab23456789".into(), (2, 2)));
    }

    #[test]
    fn wrap_off_overwrites_from_the_runs_start() {
        let mut s = screen();
        client(&mut s, b"\x1b[?7l\x1b[4G", VT);
        let (vt, _) = client(&mut s, b"abcdefghij", PROCESSED_OUTPUT);
        assert_eq!(vt, b"abcdefg\x1b[4Ghij");
        assert_eq!((row(&s, 0), s.cursor()), ("   hijdefg".into(), (6, 0)));
        // Text that just fills the row goes back too.
        let (vt, _) = client(&mut s, b"\r0123456789", PROCESSED_OUTPUT);
        assert_eq!(
            (vt.as_slice(), s.cursor()),
            (&b"\r0123456789\x1b[1G"[..], (0, 0))
        );
    }

    #[test]
    fn wrap_off_runs_end_at_processed_controls() {
        let mut s = screen();
        // ab, the glyph, then a run from column 3.
        client(&mut s, b"ab\x01cdefghijk", PROCESSED_OUTPUT);
        assert_eq!((row(&s, 0), s.cursor()), ("ab☺jkefghi".into(), (5, 0)));
        // Unprocessed, the whole write is one run.
        client(&mut s, b"\x1b[2H", VT);
        client(&mut s, b"ab\x01cdefghijk", 0);
        assert_eq!((row(&s, 1), s.cursor()), ("jk☺cdefghi".into(), (2, 1)));
    }

    #[test]
    fn wrap_off_moves_a_wide_glyph_at_the_margin_to_the_runs_start() {
        let mut s = screen();
        let (vt, _) = client(&mut s, "abcdefghi漢".as_bytes(), PROCESSED_OUTPUT);
        assert_eq!(vt, "abcdefghi \x1b[1G漢".as_bytes());
        assert_eq!(s.cursor(), (2, 0));
        assert_eq!(row(&s, 0).chars().next(), Some('漢'));
        assert_eq!(s.grid().row(0)[9].ch, ' ', "the last column is padded");
        // A run starting in the last column can't hold it: it's dropped.
        let (vt, _) = client(&mut s, "\r\nabcdefghi".as_bytes(), PROCESSED_OUTPUT);
        assert_eq!(vt, b"\r\nabcdefghi");
        let (vt, _) = client(&mut s, "漢x".as_bytes(), PROCESSED_OUTPUT);
        assert_eq!(vt, b" \x1b[10Gx\x1b[10G");
        assert_eq!((row(&s, 1), s.cursor()), ("abcdefghix".into(), (9, 1)));
    }

    #[test]
    fn marks_join_the_glyph_at_the_margin_before_the_cursor_moves() {
        let mut s = screen();
        let (vt, _) = client(&mut s, "abcdefghie\u{301}x".as_bytes(), PROCESSED_OUTPUT);
        assert_eq!(vt, "abcdefghie\u{301}\x1b[1Gx".as_bytes());
        assert_ne!(s.grid().row(0)[9].cluster, 0, "the accent is on the e");
        assert_eq!((row(&s, 0), s.cursor()), ("xbcdefghie".into(), (1, 0)));
        let legacy = PROCESSED_OUTPUT | WRAP_AT_EOL_OUTPUT;
        let (vt, _) = client(&mut s, "\r\n0123456789\u{301}x".as_bytes(), legacy);
        assert_eq!(vt, "\r\n0123456789\u{301}\r\nx".as_bytes());
        assert_ne!(s.grid().row(1)[9].cluster, 0, "and on the 9");
    }

    #[test]
    fn wrap_off_margin_counts_against_room() {
        let mut s = Screen::new(WinSize { rows: 4, cols: 2 });
        let mut out = OutputQueue::default();
        let mut replies = Vec::new();
        let n = s.client(b"abc", PROCESSED_OUTPUT, 10, true, &mut out, &mut replies);
        assert_eq!(n, 2, "b and its CHA would not fit with c");
        assert_eq!(out.take(), b"ab\x1b[1G");
    }

    #[test]
    fn wrap_off_margins_stay_within_max_expansion() {
        // Five-digit CHAs, the longest; runs from the columns where every
        // glyph meets the margin.
        let cols = 20_000;
        let cases: [(usize, &str); 4] = [
            (cols - 1, "a"),
            (cols - 1, "\u{1}"),
            (cols - 2, "€漢"),
            (cols - 3, "漢"),
        ];
        for ((start, unit), mode) in cases
            .into_iter()
            .flat_map(|c| [(c, 0), (c, PROCESSED_OUTPUT)])
        {
            let mut s = Screen::new(WinSize {
                rows: 2,
                cols: cols as u16,
            });
            client(
                &mut s,
                format!("\x1b[?7l\x1b[{}G", start + 1).as_bytes(),
                VT,
            );
            let text = unit.repeat(64);
            let units = text.encode_utf16().count();
            let room = units * MAX_EXPANSION + PREAMBLE.len();
            let mut out = OutputQueue::default();
            let n = s.client(text.as_bytes(), mode, room, true, &mut out, &mut Vec::new());
            assert_eq!(n, text.len(), "{unit:?} from {start}, mode {mode}");
            assert!(out.take().len() <= room);
        }
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
        // SGR 4 is no legacy bit: COMMON_LVB_UNDERSCORE is conhost's
        // bottom gridline (the parity suite's `print 7 \e[4m`).
        client(&mut s, b"\x1b[91;44;4;7m", VT);
        assert_eq!(s.attributes(), 0x0C | 0x10 | LVB_REVERSE_VIDEO);
        // Bold brightens a default or 16-colour foreground; overline is
        // the top gridline.
        client(&mut s, b"\x1b[0;1;53m", VT);
        assert_eq!(s.attributes(), 0x0F | LVB_GRID_HORIZONTAL);
        client(&mut s, b"\x1b[0;1;38;5;100m", VT);
        assert_eq!(s.attributes() & FOREGROUND_INTENSITY, 0, "not 256-colour");
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
            0x07 | LVB_GRID_HORIZONTAL | 0x0800 | 0x1000,
        ] {
            let mut s = screen();
            let mut out = OutputQueue::default();
            s.set_attributes(attr, &mut out);
            assert_eq!(out.take(), sgr(attr));
            assert_eq!(s.attributes(), attr, "{attr:#06x}");
        }
        assert_eq!(sgr(0x07), b"\x1b[0m");
        assert_eq!(sgr(0x4E), b"\x1b[0;93;41m");
        // conhost's own order: VtIoTests::SetConsoleTextAttribute.
        assert_eq!(sgr(0x2D | LVB_REVERSE_VIDEO), b"\x1b[0;7;95;42m");
    }

    #[test]
    fn gridlines_hold_until_a_rendition_reset() {
        let mut s = screen();
        let mut out = OutputQueue::default();
        s.set_attributes(0x1E | LVB_UNDERSCORE, &mut out);
        client(&mut s, b"text", VT);
        client(&mut s, b"\x1b[24;31m", VT);
        assert_eq!(
            s.attributes() & LVB_GRID,
            LVB_UNDERSCORE,
            "only a reset clears them"
        );
        client(&mut s, b"\x1b[0m", VT);
        assert_eq!(s.attributes(), 0x07, "SGR 0 resets them");
        client(&mut s, b"\x1b[4m", VT);
        assert_eq!(s.attributes(), 0x07, "and SGR 4 is not one");
    }

    #[test]
    fn default_coloured_gridlines_are_cleared_by_sgr_0() {
        // The pen after 0x0807 is the default one, the same as after
        // SGR 0: only the reset itself can tell them apart.
        let mut s = screen();
        let mut out = OutputQueue::default();
        s.set_attributes(0x0807, &mut out);
        assert_eq!(s.attributes(), 0x0807);
        client(&mut s, b"plain text keeps them", 0x3);
        assert_eq!(s.attributes(), 0x0807);
        client(&mut s, b"\x1b[0m", VT);
        assert_eq!(s.attributes(), 0x07);
    }

    #[test]
    fn a_reset_then_the_same_pen_does_not_bring_them_back() {
        let mut s = screen();
        let mut out = OutputQueue::default();
        s.set_attributes(0x1E | 0x1000, &mut out);
        client(&mut s, b"\x1b[0m\x1b[93;44m", VT);
        assert_eq!(s.attributes(), 0x1E);
    }

    #[test]
    fn gridlines_follow_the_models_sgr_parsing() {
        let mut out = OutputQueue::default();
        // A zero-padded 256-colour selector: index 0, not a reset.
        let mut s = screen();
        s.set_attributes(0x07 | LVB_UNDERSCORE, &mut out);
        client(&mut s, b"\x1b[38;05;0m", VT);
        assert_eq!(s.attributes() & LVB_GRID, LVB_UNDERSCORE);
        // A 67-byte list ending in 0: the model applies it, so a reset,
        // whole or split across writes.
        let long = format!("\x1b[{}0m", "1;".repeat(33));
        for split in [long.len(), 3, 40, long.len() - 1] {
            s.set_attributes(0x07 | LVB_UNDERSCORE, &mut out);
            let (head, tail) = long.as_bytes().split_at(split);
            client(&mut s, head, VT);
            client(&mut s, tail, VT);
            assert_eq!(s.attributes() & LVB_GRID, 0, "split at {split}");
        }
        // The extended forms keep them, split or not.
        for vt in [
            &b"\x1b[38;5;0m"[..],
            b"\x1b[48;2;0;0;0m",
            b"\x1b[38:2:0:0:0m",
        ] {
            s.set_attributes(0x07 | LVB_UNDERSCORE, &mut out);
            let (head, tail) = vt.split_at(vt.len() - 2);
            client(&mut s, head, VT);
            client(&mut s, tail, VT);
            assert_eq!(s.attributes() & LVB_GRID, LVB_UNDERSCORE, "{vt:?}");
        }
    }

    #[test]
    fn color_table_starts_as_conhosts() {
        let s = screen();
        // Win32 order: blue (ANSI 4) is entry 1; COLORREF is 0x00BBGGRR.
        assert_eq!(s.color_table()[0], 0x000C_0C0C);
        assert_eq!(s.color_table()[1], 0x00DA_3700);
        assert_eq!(s.color_table()[15], 0x00F2_F2F2);
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
