//! The cell APIs over the host's Grid (design §5.1–§5.3, host PRs 6 and
//! 7): FillConsoleOutput, ScrollConsoleScreenBuffer, ReadConsoleOutput,
//! WriteConsoleOutput and their `String` forms.
//!
//! Reads take cells from the Grid as conhost's `AsCharInfo` does. Writes
//! go to the terminal and the Grid alike as VT, the way conhost under
//! ConPTY paints them (`VtIo::Writer::WriteInfos`): runs of cells, each
//! a CUP, SGR where the attribute changes and the characters, between
//! DECSC and DECRC. A write is a [`Paint`] in the write queue, so it keeps
//! its order with the text around it, and it reads the cells it keeps
//! (attributes under a character fill, the source of a scroll) a row at
//! a time as it paints, as conhost does. A step paints at most [`CHUNK`]
//! cells, which keeps its VT within one emission slice (§7.4).
//!
//! Behaviour follows microsoft/terminal's conhost (commit 09beddc6,
//! `src/server/ApiDispatchers.cpp`, `src/host/_output.cpp`,
//! `directio.cpp`, `getset.cpp`, `output.cpp`, `VtIo.cpp`; see
//! THIRD_PARTY_NOTICES.md):
//!
//! Copyright (c) Microsoft Corporation. Licensed under the MIT license.

use std::collections::VecDeque;

use platform::term::WinSize;
use rusty_term_core::char_width;

use super::output::reply;
use super::screen::{CP437_C0, Screen, WATCHED_MAX};
use super::{Console, Effect, Origin, OutputQueue, Packet, Reply, SLICE, Text};
use crate::codepage::{self, CodePage, Codec};
use crate::msg::{self, ApiMsg};

/// `COMMON_LVB_LEADING_BYTE` and `COMMON_LVB_TRAILING_BYTE`.
pub(super) const LEADING: u16 = 0x0100;
pub(super) const TRAILING: u16 = 0x0200;
pub(super) const LT: u16 = LEADING | TRAILING;

/// `CONSOLE_ASCII`, `CONSOLE_REAL_UNICODE`, `CONSOLE_ATTRIBUTE` and
/// `CONSOLE_FALSE_UNICODE`: what a `String` call or a fill carries.
const ASCII: u32 = 1;
const REAL_UNICODE: u32 = 2;
const ATTRIBUTE: u32 = 3;
const FALSE_UNICODE: u32 = 4;

/// `CONSOLE_FILLCONSOLEOUTPUT_MSG { COORD; ElementType; Element; Length }`.
const FILL_LEN: usize = 16;
const FILL_LENGTH: usize = 12;
/// `CONSOLE_{READ,WRITE}CONSOLEOUTPUTSTRING_MSG { COORD; StringType;
/// NumRecords }`.
const STRING_LEN: usize = 12;
const STRING_RECORDS: usize = 8;
/// `CONSOLE_{READ,WRITE}CONSOLEOUTPUT_MSG { SMALL_RECT; Unicode }`.
const BLOCK_LEN: usize = 10;
/// `CONSOLE_SCROLLSCREENBUFFER_MSG { ScrollRectangle; ClipRectangle;
/// Clip; Unicode; DestinationOrigin; Fill }`.
const SCROLL_LEN: usize = 26;

/// The most VT one cell adds: the longest SGR [`Screen::paint`] makes
/// (`ESC [0;7;97;107;53;4m`, 18 bytes) and a glyph of up to three bytes,
/// twice for a wide control.
const MAX_CELL_VT: usize = 24;
/// The most VT one run adds besides its cells: DECSC, ASCII into G0 and
/// SI, origin mode off, insert mode off, a CUP, insert mode on, origin
/// mode on, DECRC.
const MAX_RUN_VT: usize = 2 + 3 + 1 + 5 + 4 + 14 + 4 + 5 + 2;
/// Cells painted per emission step. Each could be a run of its own, and
/// the step's first paint flushes the client bytes the screen holds
/// (shorter than [`WATCHED_MAX`]) ahead of it.
pub(super) const CHUNK: usize = (SLICE - WATCHED_MAX) / (MAX_CELL_VT + MAX_RUN_VT);
const _: () = assert!(WATCHED_MAX + CHUNK * (MAX_CELL_VT + MAX_RUN_VT) <= SLICE);

/// One cell as the classic console API sees it (`CHAR_INFO`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CharInfo {
    pub ch: u16,
    pub attr: u16,
}

impl CharInfo {
    const BYTES: usize = 4;

    fn read(bytes: &[u8; 4]) -> CharInfo {
        CharInfo {
            ch: u16::from_le_bytes([bytes[0], bytes[1]]),
            attr: u16::from_le_bytes([bytes[2], bytes[3]]),
        }
    }

    fn put(self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.ch.to_le_bytes());
        out[2..4].copy_from_slice(&self.attr.to_le_bytes());
    }
}

/// conhost `VtIo::SanitizeUCS2`: what the terminal is sent for `unit`.
/// C0 controls and DEL show their CP437 glyphs, C1 controls `?`, and a
/// surrogate, alone in its cell, U+FFFD.
pub(super) fn sanitize(unit: u16) -> char {
    match unit {
        0..0x20 => CP437_C0[usize::from(unit)],
        0x7F => '⌂',
        0x80..0xA0 => '?',
        _ => char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER),
    }
}

/// Whether `unit` takes two cells (conhost: `ch >= 0x80 &&
/// IsGlyphFullWidth(ch)`), by rusty_term_core's width rules (§5.3).
fn is_wide(unit: u16) -> bool {
    unit >= 0x80 && char::from_u32(u32::from(unit)).is_some_and(|c| char_width(c) == 2)
}

/// An inclusive rectangle (`SMALL_RECT`, conhost's `Viewport`). It is
/// empty when its right or bottom edge comes before its left or top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn screen(size: WinSize) -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: i32::from(size.cols) - 1,
            bottom: i32::from(size.rows) - 1,
        }
    }

    /// The `SMALL_RECT` at `at` of a fixed struct.
    fn read(fixed: &[u8], at: usize) -> Option<Rect> {
        let edge = |i: usize| msg::u16_at(fixed, at + 2 * i).map(|v| i32::from(v as i16));
        Some(Rect {
            left: edge(0)?,
            top: edge(1)?,
            right: edge(2)?,
            bottom: edge(3)?,
        })
    }

    /// Write the rectangle at `at`. Every rectangle written back lies on
    /// the screen or is the empty one at a request's origin, so it fits
    /// a `SMALL_RECT`.
    fn put(self, fixed: &mut [u8], at: usize) {
        for (i, v) in [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .enumerate()
        {
            msg::put_u16(fixed, at + 2 * i, v as i16 as u16);
        }
    }

    /// The empty rectangle at this one's origin, which conhost reports
    /// for a call that did nothing.
    fn none(self) -> Rect {
        Rect {
            right: self.left - 1,
            bottom: self.top - 1,
            ..self
        }
    }

    fn is_empty(self) -> bool {
        self.right < self.left || self.bottom < self.top
    }

    fn width(self) -> usize {
        usize::try_from(self.right - self.left + 1).unwrap_or(0)
    }

    fn height(self) -> usize {
        usize::try_from(self.bottom - self.top + 1).unwrap_or(0)
    }

    fn intersect(self, o: Rect) -> Rect {
        Rect {
            left: self.left.max(o.left),
            top: self.top.max(o.top),
            right: self.right.min(o.right),
            bottom: self.bottom.min(o.bottom),
        }
    }

    fn offset(self, dx: i32, dy: i32) -> Rect {
        Rect {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }

    fn rows(self) -> std::ops::RangeInclusive<i32> {
        self.top..=self.bottom
    }
}

/// A row of cells to paint from (`x`, `y`).
#[derive(Debug)]
struct Run {
    x: usize,
    y: usize,
    cells: Vec<CharInfo>,
}

/// What a fill or `String` write puts in the cells it passes over.
#[derive(Debug)]
enum Stream {
    FillChar(u16),
    FillAttr(u16),
    Chars(Vec<u16>),
    Attrs(Vec<u16>),
    /// An `A` string write, kept in its code page so a parked write
    /// retains no more than it was sent; each step decodes it afresh, as
    /// the write queue's own `A` text does.
    Coded {
        bytes: Vec<u8>,
        cp: u32,
    },
}

impl Stream {
    fn sets_attrs(&self) -> bool {
        matches!(self, Stream::FillAttr(_) | Stream::Attrs(_))
    }

    /// Element `i`; `decoded` is a `Coded` stream's text.
    fn at(&self, i: usize, decoded: &[u16]) -> u16 {
        match self {
            Stream::FillChar(v) | Stream::FillAttr(v) => *v,
            Stream::Chars(v) | Stream::Attrs(v) => v[i],
            Stream::Coded { .. } => decoded[i],
        }
    }

    /// A `Coded` stream's text, as UTF-16.
    fn decode(&self, codec: &Codec) -> Vec<u16> {
        match self {
            Stream::Coded { bytes, cp } => {
                codepage::decode(codec, *cp, bytes).encode_utf16().collect()
            }
            _ => Vec::new(),
        }
    }
}

/// A ScrollConsoleScreenBuffer as asked, laid out when it paints.
#[derive(Debug)]
struct ScrollArgs {
    source: Rect,
    clip: Option<Rect>,
    dest: (i32, i32),
    fill: CharInfo,
}

/// A ScrollConsoleScreenBuffer laid out on the screen (conhost
/// `ScrollConsoleScreenBufferWImpl`): `fill` is the source within the
/// clip, `from` what of the source can move, and it lands at `from`
/// moved by `delta`. Rows go so that a source row is read before it is
/// written: up the screen when the text moves down, else down it.
#[derive(Debug)]
struct ScrollPlan {
    fill: Rect,
    from: Rect,
    delta: (i32, i32),
    cell: CharInfo,
    rows: Vec<i32>,
}

#[derive(Debug)]
enum Op {
    /// From (`x`, `y`) through the rows, `len` elements of `what`; `pos`
    /// of them are done.
    Stream {
        what: Stream,
        len: usize,
        x: usize,
        y: usize,
        pos: usize,
    },
    /// WriteConsoleOutput's cells, clipped to the screen when it was
    /// called: `width` wide from (`left`, `top`), row `row` next.
    Block {
        left: usize,
        top: usize,
        width: usize,
        cells: Vec<CharInfo>,
        row: usize,
    },
    Scroll(ScrollArgs, Option<ScrollPlan>),
}

impl Op {
    /// The runs of the next row, reading the Grid now; `None` when done.
    fn next_row(&mut self, screen: &Screen, decoded: &[u16]) -> Option<Vec<Run>> {
        let size = screen.size();
        match self {
            Op::Stream {
                what,
                len,
                x,
                y,
                pos,
            } => {
                let (w, h) = (usize::from(size.cols), usize::from(size.rows));
                if *pos >= *len || *y >= h || *x >= w {
                    return None;
                }
                let mut cells = screen.cells(*y, *x, w);
                let used = lay_out(what, decoded, *len, pos, &mut cells);
                cells.truncate(used);
                let run = Run {
                    x: std::mem::take(x),
                    y: *y,
                    cells,
                };
                *y += 1;
                Some(vec![run])
            }
            Op::Block {
                left,
                top,
                width,
                cells,
                row,
            } => {
                let start = *row * *width;
                let y = *top + *row;
                if start >= cells.len() || y >= usize::from(size.rows) {
                    return None;
                }
                *row += 1;
                let fits = usize::from(size.cols).saturating_sub(*left).min(*width);
                let run = Run {
                    x: *left,
                    y,
                    cells: cells[start..start + fits].to_vec(),
                };
                Some(vec![run])
            }
            Op::Scroll(args, plan) => {
                let plan = plan.get_or_insert_with(|| plan_scroll(args, size, screen.attributes()));
                let y = plan.rows.pop()?;
                Some(scroll_row(plan, y, screen))
            }
        }
    }
}

/// Lay `what` out over `cells`, one row from the write's column, as
/// conhost's `FillConsoleImpl` does; returns the cells used. Attributes
/// keep each cell's leading and trailing flags. A wide character takes
/// two cells; one that doesn't fit in the row's last cell leaves a
/// space there and goes no further.
fn lay_out(
    what: &Stream,
    decoded: &[u16],
    len: usize,
    pos: &mut usize,
    cells: &mut [CharInfo],
) -> usize {
    let avail = cells.len();
    let mut used = 0;
    while used < avail && *pos < len {
        let v = what.at(*pos, decoded);
        if what.sets_attrs() {
            let cell = &mut cells[used];
            cell.attr = v & !LT | cell.attr & LT;
            used += 1;
        } else if is_wide(v) {
            if used + 1 >= avail {
                let cell = &mut cells[used];
                *cell = CharInfo {
                    ch: u16::from(b' '),
                    attr: cell.attr & !LT,
                };
                return used + 1;
            }
            let (lead, trail) = (cells[used].attr, cells[used + 1].attr);
            cells[used] = CharInfo {
                ch: v,
                attr: lead & !TRAILING | LEADING,
            };
            cells[used + 1] = CharInfo {
                ch: v,
                attr: trail & !LEADING | TRAILING,
            };
            used += 2;
        } else {
            let cell = &mut cells[used];
            *cell = CharInfo {
                ch: v,
                attr: cell.attr & !LT,
            };
            used += 1;
        }
        *pos += 1;
    }
    used
}

fn plan_scroll(args: &ScrollArgs, size: WinSize, pen: u16) -> ScrollPlan {
    let screen = Rect::screen(size);
    let clip = args.clip.map_or(screen, |c| c.intersect(screen));
    let delta = (
        args.dest.0 - args.source.left,
        args.dest.1 - args.source.top,
    );
    let fill = args.source.intersect(clip);
    let from = args
        .source
        .intersect(screen)
        .intersect(clip.offset(-delta.0, -delta.1));
    let to = from.offset(delta.0, delta.1);
    let mut rows: Vec<i32> = Vec::new();
    for r in [fill, to].into_iter().filter(|r| !r.is_empty()) {
        rows.extend(r.rows());
    }
    // Popped from the end: moving down, the bottom row goes first.
    rows.sort_unstable_by(|a, b| if delta.1 > 0 { a.cmp(b) } else { b.cmp(a) });
    rows.dedup();
    // Applications pass 0 and 0 for the current pen's blank.
    let attr = if args.fill == (CharInfo { ch: 0, attr: 0 }) {
        pen
    } else {
        args.fill.attr & !LT
    };
    let ch = u16::try_from(u32::from(sanitize(args.fill.ch))).unwrap_or(0xFFFD);
    ScrollPlan {
        fill,
        from,
        delta,
        cell: CharInfo { ch, attr },
        rows,
    }
}

/// Row `y` of a scroll: its source read first, then the fill outside
/// where the text lands, then the moved text.
fn scroll_row(plan: &ScrollPlan, y: i32, screen: &Screen) -> Vec<Run> {
    let to = plan.from.offset(plan.delta.0, plan.delta.1);
    let lands = !to.is_empty() && to.rows().contains(&y);
    let moved = lands.then(|| {
        let src = (y - plan.delta.1) as usize;
        screen.cells(src, plan.from.left as usize, plan.from.right as usize + 1)
    });
    let mut runs = Vec::new();
    if !plan.fill.is_empty() && plan.fill.rows().contains(&y) {
        let (l, r) = (plan.fill.left, plan.fill.right);
        let spans = if lands {
            vec![(l, r.min(to.left - 1)), (l.max(to.right + 1), r)]
        } else {
            vec![(l, r)]
        };
        for (a, b) in spans.into_iter().filter(|(a, b)| a <= b) {
            runs.push(Run {
                x: a as usize,
                y: y as usize,
                cells: fill_span(plan.cell, (b - a + 1) as usize),
            });
        }
    }
    if let Some(cells) = moved {
        runs.push(Run {
            x: to.left as usize,
            y: y as usize,
            cells,
        });
    }
    runs
}

/// `n` cells of a scroll's fill. A wide character fills them in leading
/// and trailing pairs, as conhost's `OutputCellIterator` does; an odd
/// cell left at the span's end can't hold a half and takes a space.
fn fill_span(cell: CharInfo, n: usize) -> Vec<CharInfo> {
    if !is_wide(cell.ch) {
        return vec![cell; n];
    }
    let mut cells: Vec<CharInfo> = (0..n)
        .map(|i| CharInfo {
            ch: cell.ch,
            attr: cell.attr | if i % 2 == 0 { LEADING } else { TRAILING },
        })
        .collect();
    if n % 2 == 1 {
        cells[n - 1] = CharInfo {
            ch: u16::from(b' '),
            attr: cell.attr,
        };
    }
    cells
}

/// A write of cells in the write queue: it paints a step at a time and
/// knows its count once done.
#[derive(Debug)]
pub(super) struct Paint {
    op: Op,
    pending: VecDeque<Run>,
    /// Where the count goes in the reply's struct, and the code page an
    /// `A` write's count is measured in.
    count: Option<(usize, Option<u32>)>,
}

impl Paint {
    fn new(op: Op, count: Option<(usize, Option<u32>)>) -> Paint {
        Paint {
            op,
            pending: VecDeque::new(),
            count,
        }
    }

    /// Payload bytes the write retains while parked.
    pub(super) fn retained(&self) -> usize {
        match &self.op {
            Op::Stream {
                what: Stream::Chars(v) | Stream::Attrs(v),
                ..
            } => v.len() * 2,
            Op::Stream {
                what: Stream::Coded { bytes, .. },
                ..
            } => bytes.len(),
            Op::Block { cells, .. } => cells.len() * CharInfo::BYTES,
            _ => 0,
        }
    }

    /// Paint up to [`CHUNK`] cells; whether all are painted.
    pub(super) fn step(
        &mut self,
        screen: &mut Screen,
        out: &mut OutputQueue,
        codec: &Codec,
    ) -> bool {
        let mut budget = CHUNK;
        let decoded = match &self.op {
            Op::Stream { what, .. } => what.decode(codec),
            _ => Vec::new(),
        };
        loop {
            let Some(mut run) = self.pending.pop_front() else {
                match self.op.next_row(screen, &decoded) {
                    Some(runs) => self.pending.extend(runs),
                    None => return true,
                }
                continue;
            };
            // A resize since the row was laid out may have cut it short.
            let size = screen.size();
            let room = usize::from(size.cols).saturating_sub(run.x);
            run.cells.truncate(room);
            if run.cells.is_empty() || run.y >= usize::from(size.rows) {
                continue;
            }
            let mut cut = run.cells.len().min(budget);
            // Keep a wide character's halves together.
            if cut < run.cells.len() && cut > 0 && run.cells[cut - 1].attr & LEADING != 0 {
                cut -= 1;
            }
            if cut == 0 {
                self.pending.push_front(run);
                return false;
            }
            let rest = run.cells.split_off(cut);
            if !rest.is_empty() {
                self.pending.push_front(Run {
                    x: run.x + cut,
                    y: run.y,
                    cells: rest,
                });
            }
            screen.paint(run.x, run.y, &run.cells, out);
            budget -= cut;
        }
    }

    /// Put the count of what was done in `fixed`: elements read from a
    /// fill or `String` write, which for an `A` write are its bytes.
    pub(super) fn complete(&self, fixed: &mut [u8], codec: &Codec) {
        let (Some((at, cp)), Op::Stream { what, pos, .. }) = (self.count, &self.op) else {
            return;
        };
        let n = match (cp, what) {
            (Some(cp), Stream::Coded { .. }) => {
                let text = String::from_utf16_lossy(&what.decode(codec)[..*pos]);
                codepage::encode(codec, cp, &text).len()
            }
            _ => *pos,
        };
        if fixed.len() >= at + 4 {
            msg::put_u32(fixed, at, n as u32);
        }
    }
}

impl Console {
    /// `CONSOLE_FILLCONSOLEOUTPUT_MSG`: FillConsoleOutputCharacter and
    /// FillConsoleOutputAttribute. From a position on the screen through
    /// the rows, up to the end of the screen; Length comes back as the
    /// elements used.
    pub(super) fn fill_output(
        &mut self,
        p: &Packet<'_>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        if fixed.len() < FILL_LEN {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        }
        let (Some(x), Some(y), Some(kind), Some(element), Some(len)) = (
            msg::u16_at(&fixed, 0),
            msg::u16_at(&fixed, 2),
            msg::u32_at(&fixed, 4),
            msg::u16_at(&fixed, 8),
            msg::u32_at(&fixed, FILL_LENGTH),
        ) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        msg::put_u32(&mut fixed, FILL_LENGTH, 0);
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        let what = match kind {
            ATTRIBUTE => Stream::FillAttr(element),
            REAL_UNICODE | FALSE_UNICODE => Stream::FillChar(element),
            ASCII => Stream::FillChar(self.ansi_unit(element as u8)),
            _ => return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx),
        };
        self.stream(
            p,
            fixed,
            (x, y),
            what,
            len as usize,
            (FILL_LENGTH, None),
            out,
            fx,
        );
    }

    /// `CONSOLE_WRITECONSOLEOUTPUTSTRING_MSG`: WriteConsoleOutputCharacter
    /// and WriteConsoleOutputAttribute, the elements in the input.
    pub(super) fn write_output_string(
        &mut self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(x), Some(y), Some(kind), true) = (
            msg::u16_at(&fixed, 0),
            msg::u16_at(&fixed, 2),
            msg::u32_at(&fixed, 4),
            fixed.len() >= STRING_LEN,
        ) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        msg::put_u32(&mut fixed, STRING_RECORDS, 0);
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        let (what, ansi) = match kind {
            ATTRIBUTE => (Stream::Attrs(msg::utf16_units(m.variable)), None),
            REAL_UNICODE | FALSE_UNICODE => (Stream::Chars(msg::utf16_units(m.variable)), None),
            ASCII => {
                let cp = self.output_cp.cp;
                let what = Stream::Coded {
                    bytes: m.variable.to_vec(),
                    cp,
                };
                (what, Some(cp))
            }
            _ => return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx),
        };
        let len = match &what {
            Stream::Chars(v) | Stream::Attrs(v) => v.len(),
            coded @ Stream::Coded { .. } => coded.decode(&self.codec).len(),
            _ => 0,
        };
        self.stream(p, fixed, (x, y), what, len, (STRING_RECORDS, ansi), out, fx);
    }

    /// Queue a fill or `String` write. One that starts off the screen or
    /// has nothing to write does nothing and succeeds.
    #[allow(clippy::too_many_arguments)]
    fn stream(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        (x, y): (u16, u16),
        what: Stream,
        len: usize,
        count: (usize, Option<u32>),
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let size = self.screen.size();
        // COORD halves are signed: a negative one is past any size.
        if len == 0 || x >= size.cols || y >= size.rows {
            return reply(p, msg::STATUS_SUCCESS, fixed, fx);
        }
        let op = Op::Stream {
            what,
            len,
            x: usize::from(x),
            y: usize::from(y),
            pos: 0,
        };
        self.paint(p, fixed, Paint::new(op, Some(count)), out, fx);
    }

    /// `CONSOLE_SCROLLSCREENBUFFER_MSG`: move a rectangle, within the clip
    /// rectangle if any, and fill what it leaves with the fill cell.
    pub(super) fn scroll_screen_buffer(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(source), Some(clip), Some(dx), Some(dy), Some(ch), Some(attr), true) = (
            Rect::read(&fixed, 0),
            Rect::read(&fixed, 8),
            msg::u16_at(&fixed, 18),
            msg::u16_at(&fixed, 20),
            msg::u16_at(&fixed, 22),
            msg::u16_at(&fixed, 24),
            fixed.len() >= SCROLL_LEN,
        ) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        let dest = (i32::from(dx as i16), i32::from(dy as i16));
        // A request that moves nothing, or an empty source, does nothing.
        if (source.left, source.top) == dest || source.is_empty() {
            return reply(p, msg::STATUS_SUCCESS, fixed, fx);
        }
        let ch = if fixed[17] != 0 {
            ch
        } else {
            self.ansi_unit(ch as u8)
        };
        let args = ScrollArgs {
            source,
            clip: (fixed[16] != 0).then_some(clip),
            dest,
            fill: CharInfo { ch, attr },
        };
        self.paint(p, fixed, Paint::new(Op::Scroll(args, None), None), out, fx);
    }

    /// `CONSOLE_WRITECONSOLEOUTPUT_MSG`: a rectangle of `CHAR_INFO`s from
    /// the input, clipped to the screen. CharRegion comes back as the
    /// part written.
    pub(super) fn write_output(
        &mut self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        mut fixed: Vec<u8>,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let (Some(region), Some(unicode), true) = (
            Rect::read(&fixed, 0),
            msg::bool_at(&fixed, 8),
            fixed.len() >= BLOCK_LEN,
        ) else {
            return reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed, fx);
        };
        region.none().put(&mut fixed, 0);
        if let Err(status) = self.output_object(p.object, true) {
            return reply(p, status, fixed, fx);
        }
        let (width, area) = (region.width(), region.width() * region.height());
        if m.variable.len() < area * CharInfo::BYTES {
            return reply(p, msg::STATUS_FROM_INVALIDARG, fixed, fx);
        }
        let screen = Rect::screen(self.screen.size());
        let clipped = region.intersect(screen);
        if area == 0 || clipped.is_empty() {
            return reply(p, msg::STATUS_SUCCESS, fixed, fx);
        }
        let mut given: Vec<CharInfo> = m.variable[..area * CharInfo::BYTES]
            .as_chunks::<{ CharInfo::BYTES }>()
            .0
            .iter()
            .map(CharInfo::read)
            .collect();
        if !unicode {
            for row in given.chunks_mut(width) {
                cells_from_ansi(row, self.output_cp, &self.codec);
            }
        }
        let (dx, dy) = (clipped.left - region.left, clipped.top - region.top);
        let mut cells = Vec::with_capacity(clipped.width() * clipped.height());
        for r in 0..clipped.height() {
            let at = (dy as usize + r) * width + dx as usize;
            cells.extend_from_slice(&given[at..at + clipped.width()]);
        }
        clipped.put(&mut fixed, 0);
        let op = Op::Block {
            left: clipped.left as usize,
            top: clipped.top as usize,
            width: clipped.width(),
            cells,
            row: 0,
        };
        self.paint(p, fixed, Paint::new(op, None), out, fx);
    }

    /// `CONSOLE_READCONSOLEOUTPUT_MSG`: a rectangle of the screen as
    /// `CHAR_INFO`s, laid out as the whole request in the output buffer.
    /// CharRegion comes back as the part read. A reply larger than
    /// [`READ_REPLY_CAP`](super::read::READ_REPLY_CAP) is refused (§7.4).
    pub(super) fn read_output(&self, p: &Packet<'_>, m: ApiMsg<'_>, mut fixed: Vec<u8>) -> Reply {
        let (Some(region), Some(unicode), true) = (
            Rect::read(&fixed, 0),
            msg::bool_at(&fixed, 8),
            fixed.len() >= BLOCK_LEN,
        ) else {
            return status_reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed);
        };
        region.none().put(&mut fixed, 0);
        if let Err(status) = self.output_object(p.object, false) {
            return status_reply(p, status, fixed);
        }
        let at = m.fixed.len();
        let room = (p.output_size as usize).saturating_sub(at);
        let (width, area) = (region.width(), region.width() * region.height());
        let bytes = area * CharInfo::BYTES;
        if bytes > room {
            return status_reply(p, msg::STATUS_FROM_INVALIDARG, fixed);
        }
        if bytes > super::read::READ_REPLY_CAP {
            return status_reply(p, msg::STATUS_INVALID_PARAMETER, fixed);
        }
        let clipped = region.intersect(Rect::screen(self.screen.size()));
        if area == 0 || clipped.is_empty() {
            return status_reply(p, msg::STATUS_SUCCESS, fixed);
        }
        let mut output = vec![0; bytes];
        let (dx, dy) = (clipped.left - region.left, clipped.top - region.top);
        for r in 0..clipped.height() {
            let y = (clipped.top as usize) + r;
            let mut row = self
                .screen
                .cells(y, clipped.left as usize, clipped.right as usize + 1);
            if !unicode {
                cells_to_ansi(&mut row, self.output_cp.cp, &self.codec);
            }
            let start = ((dy as usize + r) * width + dx as usize) * CharInfo::BYTES;
            for (i, c) in row.into_iter().enumerate() {
                c.put(&mut output[start + i * CharInfo::BYTES..]);
            }
        }
        clipped.put(&mut fixed, 0);
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        reply.information = output.len();
        reply.output = output;
        reply.output_at = at as u32;
        reply
    }

    /// `CONSOLE_READCONSOLEOUTPUTSTRING_MSG`: ReadConsoleOutputCharacter
    /// and ReadConsoleOutputAttribute, from a position through the rows,
    /// as many as the output buffer holds (conhost `ReadOutputStringW`,
    /// `ReadOutputAttributes`). A wide character reads as one unit; a
    /// trailing half first or a leading half last reads as a space.
    pub(super) fn read_output_string(
        &self,
        p: &Packet<'_>,
        m: ApiMsg<'_>,
        mut fixed: Vec<u8>,
    ) -> Reply {
        let (Some(x), Some(y), Some(kind), true) = (
            msg::u16_at(&fixed, 0),
            msg::u16_at(&fixed, 2),
            msg::u32_at(&fixed, 4),
            fixed.len() >= STRING_LEN,
        ) else {
            return status_reply(p, msg::STATUS_ILLEGAL_FUNCTION, fixed);
        };
        msg::put_u32(&mut fixed, STRING_RECORDS, 0);
        if let Err(status) = self.output_object(p.object, false) {
            return status_reply(p, status, fixed);
        }
        let at = m.fixed.len();
        let room = (p.output_size as usize)
            .saturating_sub(at)
            .min(super::read::READ_REPLY_CAP);
        let wanted = match kind {
            ASCII => room,
            REAL_UNICODE | FALSE_UNICODE | ATTRIBUTE => room / 2,
            _ => return status_reply(p, msg::STATUS_FROM_INVALIDARG, fixed),
        };
        let cells = self.cells_from((x, y), wanted);
        // conhost pads only at the request's own ends.
        let edge = |i: usize, c: &CharInfo| {
            i == 0 && c.attr & TRAILING != 0 || i + 1 == wanted && c.attr & LEADING != 0
        };
        let (records, output) = if kind == ATTRIBUTE {
            let attrs: Vec<u8> = cells
                .iter()
                .enumerate()
                .flat_map(|(i, c)| (if edge(i, c) { c.attr & !LT } else { c.attr }).to_le_bytes())
                .collect();
            (cells.len(), attrs)
        } else {
            let units: Vec<u16> = cells
                .iter()
                .enumerate()
                .filter_map(|(i, c)| match () {
                    _ if edge(i, c) => Some(u16::from(b' ')),
                    _ if c.attr & TRAILING != 0 => None,
                    _ => Some(c.ch),
                })
                .collect();
            if kind == ASCII {
                let text = String::from_utf16_lossy(&units);
                let bytes = codepage::encode(&self.codec, self.output_cp.cp, &text);
                // conhost returns nothing at all when the text won't fit.
                if bytes.len() > room {
                    (0, Vec::new())
                } else {
                    (bytes.len(), bytes)
                }
            } else {
                (
                    units.len(),
                    units.iter().flat_map(|u| u.to_le_bytes()).collect(),
                )
            }
        };
        msg::put_u32(&mut fixed, STRING_RECORDS, records as u32);
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        reply.information = output.len();
        reply.output = output;
        reply.output_at = at as u32;
        reply
    }

    /// Up to `n` cells from (`x`, `y`) through the rows to the end of the
    /// screen; none from a position off it.
    fn cells_from(&self, (x, y): (u16, u16), n: usize) -> Vec<CharInfo> {
        let size = self.screen.size();
        let (w, h) = (usize::from(size.cols), usize::from(size.rows));
        let (mut x, mut y) = (usize::from(x), usize::from(y));
        let mut cells = Vec::new();
        while cells.len() < n && y < h && x < w {
            let end = w.min(x + (n - cells.len()));
            cells.extend(self.screen.cells(y, x, end));
            (x, y) = (0, y + 1);
        }
        cells
    }

    /// The unit an `A` call's byte stands for in the output code page.
    fn ansi_unit(&self, byte: u8) -> u16 {
        let text = codepage::decode(&self.codec, self.output_cp.cp, &[byte]);
        text.encode_utf16().next().unwrap_or(u16::from(b' '))
    }

    /// Queue `paint`; the reply carries `fixed` once it is done.
    fn paint(
        &mut self,
        p: &Packet<'_>,
        fixed: Vec<u8>,
        paint: Paint,
        out: &mut OutputQueue,
        fx: &mut Vec<Effect>,
    ) {
        let mut reply = Reply::ok(p.id);
        reply.write_back = fixed;
        self.write(reply, Text::Cells(Box::new(paint)), Origin::Host, out, fx);
    }
}

/// conhost `_ConvertCellsToWInplace`, for one row of a WriteConsoleOutputA:
/// a lead byte and the next cell's byte make one wide character, which
/// can't start in the last cell.
fn cells_from_ansi(row: &mut [CharInfo], cp: CodePage, codec: &Codec) {
    let unit = |bytes: &[u8]| {
        codepage::decode(codec, cp.cp, bytes)
            .encode_utf16()
            .next()
            .unwrap_or(u16::from(b' '))
    };
    let mut i = 0;
    while i < row.len() {
        row[i].attr &= !LT;
        let byte = row[i].ch as u8;
        if cp.is_lead(byte) && i + 1 < row.len() {
            row[i + 1].attr &= !LT;
            let ch = unit(&[byte, row[i + 1].ch as u8]);
            row[i] = CharInfo {
                ch,
                attr: row[i].attr | LEADING,
            };
            row[i + 1] = CharInfo {
                ch,
                attr: row[i + 1].attr | TRAILING,
            };
            i += 2;
            continue;
        }
        row[i].ch = if cp.is_lead(byte) {
            u16::from(b' ')
        } else {
            unit(&[byte])
        };
        i += 1;
    }
}

/// conhost `_ConvertCellsToAInplace`, for one row of a ReadConsoleOutputA:
/// a wide character's two cells hold its two bytes, a narrow one its byte.
/// What the code page can't hold in that room reads as 0.
fn cells_to_ansi(row: &mut [CharInfo], cp: u32, codec: &Codec) {
    let bytes = |unit: u16| {
        let text = String::from_utf16_lossy(&[unit]);
        codepage::encode(codec, cp, &text)
    };
    let mut i = 0;
    while i < row.len() {
        let c = row[i];
        if c.attr & LEADING != 0 {
            if i + 1 < row.len() {
                let pair = bytes(c.ch);
                let (a, b) = match pair.as_slice() {
                    [a] => (*a, 0),
                    [a, b] => (*a, *b),
                    _ => (0, 0),
                };
                row[i].ch = u16::from(a);
                row[i + 1].ch = u16::from(b);
                i += 2;
                continue;
            }
            row[i] = CharInfo {
                ch: u16::from(b' '),
                attr: c.attr & !LT,
            };
        } else if c.attr & LT == 0 {
            row[i].ch = match bytes(c.ch).as_slice() {
                &[b] => u16::from(b),
                _ => 0,
            };
        }
        i += 1;
    }
}

fn status_reply(p: &Packet<'_>, status: i32, fixed: Vec<u8>) -> Reply {
    let mut reply = Reply::status(p.id, status);
    reply.write_back = fixed;
    reply
}

#[cfg(test)]
mod tests {
    use super::super::tests::Host;
    use super::*;
    use crate::msg::api;

    const W: u16 = 10;
    const H: u16 = 4;

    fn host() -> (Host, usize) {
        let mut h = Host::sized(WinSize { rows: H, cols: W });
        let [_, _, output] = h.connect(1, "");
        (h, output)
    }

    fn coord(x: i16, y: i16) -> Vec<u8> {
        [x.to_le_bytes(), y.to_le_bytes()].concat()
    }

    fn rect(l: i16, t: i16, r: i16, b: i16) -> Vec<u8> {
        [l, t, r, b].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn rect_at(r: &Reply, at: usize) -> [i16; 4] {
        std::array::from_fn(|i| msg::u16_at(&r.write_back, at + 2 * i).expect("rect") as i16)
    }

    fn row(h: &Host, y: usize) -> String {
        h.c.screen
            .cells(y, 0, usize::from(W))
            .iter()
            .filter(|c| c.attr & TRAILING == 0)
            .map(|c| sanitize(c.ch))
            .collect()
    }

    fn attrs(h: &Host, y: usize) -> Vec<u16> {
        h.c.screen
            .cells(y, 0, usize::from(W))
            .iter()
            .map(|c| c.attr)
            .collect()
    }

    fn fill(h: &mut Host, out: usize, kind: u32, element: u16, len: u32, at: (i16, i16)) -> Reply {
        let mut fixed = coord(at.0, at.1);
        fixed.extend(kind.to_le_bytes());
        fixed.extend(element.to_le_bytes());
        fixed.extend([0, 0]);
        fixed.extend(len.to_le_bytes());
        h.call(out, api::FILL_OUTPUT, &fixed, &[], 0)
    }

    fn string_fixed(at: (i16, i16), kind: u32) -> Vec<u8> {
        let mut fixed = coord(at.0, at.1);
        fixed.extend(kind.to_le_bytes());
        fixed.extend([0; 4]);
        fixed
    }

    fn write_chars(h: &mut Host, out: usize, text: &str, at: (i16, i16)) -> Reply {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let fixed = string_fixed(at, REAL_UNICODE);
        h.call(out, api::WRITE_OUTPUT_STRING, &fixed, &bytes, 0)
    }

    fn write_attrs(h: &mut Host, out: usize, attrs: &[u16], at: (i16, i16)) -> Reply {
        let bytes: Vec<u8> = attrs.iter().flat_map(|a| a.to_le_bytes()).collect();
        let fixed = string_fixed(at, ATTRIBUTE);
        h.call(out, api::WRITE_OUTPUT_STRING, &fixed, &bytes, 0)
    }

    fn records(r: &Reply) -> u32 {
        msg::u32_at(&r.write_back, STRING_RECORDS).expect("NumRecords")
    }

    fn read_string(h: &mut Host, out: usize, kind: u32, at: (i16, i16), room: u32) -> Reply {
        let fixed = string_fixed(at, kind);
        h.call(out, api::READ_OUTPUT_STRING, &fixed, &[], room)
    }

    fn units(r: &Reply) -> String {
        String::from_utf16_lossy(&msg::utf16_units(&r.output))
    }

    fn block_fixed(region: [i16; 4], unicode: bool) -> Vec<u8> {
        let mut fixed = rect(region[0], region[1], region[2], region[3]);
        fixed.extend([u8::from(unicode), 0]);
        fixed
    }

    fn write_block(h: &mut Host, out: usize, cells: &[CharInfo], region: [i16; 4]) -> Reply {
        let mut bytes = vec![0; cells.len() * 4];
        for (i, c) in cells.iter().enumerate() {
            c.put(&mut bytes[i * 4..]);
        }
        h.call(
            out,
            api::WRITE_OUTPUT,
            &block_fixed(region, true),
            &bytes,
            0,
        )
    }

    fn read_block(h: &mut Host, out: usize, region: [i16; 4], room: u32) -> Reply {
        h.call(out, api::READ_OUTPUT, &block_fixed(region, true), &[], room)
    }

    fn block_cells(r: &Reply) -> Vec<CharInfo> {
        r.output
            .as_chunks::<4>()
            .0
            .iter()
            .map(CharInfo::read)
            .collect()
    }

    fn scroll(
        h: &mut Host,
        out: usize,
        source: [i16; 4],
        clip: Option<[i16; 4]>,
        dest: (i16, i16),
        fill: CharInfo,
    ) -> Reply {
        let mut fixed = rect(source[0], source[1], source[2], source[3]);
        let c = clip.unwrap_or([0; 4]);
        fixed.extend(rect(c[0], c[1], c[2], c[3]));
        fixed.extend([u8::from(clip.is_some()), 1]);
        fixed.extend(coord(dest.0, dest.1));
        fixed.extend(fill.ch.to_le_bytes());
        fixed.extend(fill.attr.to_le_bytes());
        h.call(out, api::SCROLL_SCREEN_BUFFER, &fixed, &[], 0)
    }

    fn length(r: &Reply) -> u32 {
        msg::u32_at(&r.write_back, FILL_LENGTH).expect("Length")
    }

    #[test]
    fn fill_runs_through_the_rows_and_stops_at_the_end() {
        let (mut h, out) = host();
        let r = fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), 12, (5, 1));
        assert_eq!((r.status, length(&r)), (msg::STATUS_SUCCESS, 12));
        assert_eq!(row(&h, 1), "     xxxxx");
        assert_eq!(row(&h, 2), "xxxxxxx   ");
        let r = fill(&mut h, out, ATTRIBUTE, 0x1e, 500, (8, 3));
        assert_eq!((r.status, length(&r)), (msg::STATUS_SUCCESS, 2));
        assert_eq!(&attrs(&h, 3)[7..], [0x07, 0x1e, 0x1e]);
        // The pen and cursor are where they were.
        assert_eq!(h.c.screen.cursor(), (0, 0));
        assert_eq!(h.c.screen.attributes(), 0x07);
    }

    #[test]
    fn fill_of_nothing_or_off_the_screen_does_nothing() {
        let (mut h, out) = host();
        let mark = h.vt.len();
        for (len, at) in [(0, (0, 0)), (5, (10, 0)), (5, (0, 4)), (5, (-1, 0))] {
            let r = fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), len, at);
            assert_eq!((r.status, length(&r)), (msg::STATUS_SUCCESS, 0), "{at:?}");
        }
        assert_eq!(h.vt.len(), mark, "nothing emitted");
        let r = fill(&mut h, out, 9, 0, 5, (0, 0));
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
    }

    #[test]
    fn fill_emits_conhost_runs_inside_decsc() {
        let (mut h, out) = host();
        let mark = h.vt.len();
        fill(&mut h, out, ATTRIBUTE, 0x1e, 3, (2, 1));
        let vt = String::from_utf8_lossy(&h.vt[mark..]).into_owned();
        assert_eq!(vt, "\x1b7\x1b(B\x0f\x1b[2;3H\x1b[0;93;44m   \x1b8");
    }

    #[test]
    fn a_wide_fill_pads_the_last_column_and_counts_characters() {
        let (mut h, out) = host();
        let r = fill(&mut h, out, REAL_UNICODE, 0x65E5, 6, (5, 0));
        // Two fit in row 0, the third would split: a space, then row 1.
        assert_eq!(length(&r), 6);
        assert_eq!(row(&h, 0), "     日日 ");
        assert_eq!(row(&h, 1), "日日日日  ");
        let r = read_string(&mut h, out, REAL_UNICODE, (5, 0), 2 * 4);
        assert_eq!((records(&r), units(&r)), (2, "日日".to_string()));
        let a = attrs(&h, 0);
        assert_eq!(a[5] & LT, LEADING);
        assert_eq!(a[6] & LT, TRAILING);
    }

    #[test]
    fn string_writes_count_what_fit() {
        let (mut h, out) = host();
        let r = write_chars(&mut h, out, "abcd", (8, 3));
        assert_eq!((r.status, records(&r)), (msg::STATUS_SUCCESS, 2));
        assert_eq!(row(&h, 3), "        ab");
        let r = write_attrs(&mut h, out, &[0x1e, 0x2f, 0x4e], (3, 2));
        assert_eq!(records(&r), 3);
        assert_eq!(&attrs(&h, 2)[3..6], [0x1e, 0x2f, 0x4e]);
        let r = read_string(&mut h, out, ATTRIBUTE, (3, 2), 2 * 3);
        assert_eq!(records(&r), 3);
        assert_eq!(msg::utf16_units(&r.output), [0x1e, 0x2f, 0x4e]);
    }

    #[test]
    fn ansi_string_writes_count_bytes() {
        let (mut h, out) = host();
        assert_eq!(h.set_cp(437, true), msg::STATUS_SUCCESS);
        let fixed = string_fixed((0, 0), ASCII);
        let r = h.call(out, api::WRITE_OUTPUT_STRING, &fixed, b"\x82t\xe9", 0);
        assert_eq!(records(&r), 3);
        assert_eq!(row(&h, 0), "étΘ       ");
        let r = read_string(&mut h, out, ASCII, (0, 0), 3);
        assert_eq!((records(&r), r.output.as_slice()), (3, &b"\x82t\xe9"[..]));
    }

    #[test]
    fn string_reads_stop_at_the_end_of_the_screen() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "yz", (8, 3));
        let r = read_string(&mut h, out, REAL_UNICODE, (8, 3), 100);
        assert_eq!((records(&r), units(&r)), (2, "yz".to_string()));
        let r = read_string(&mut h, out, REAL_UNICODE, (10, 0), 100);
        assert_eq!((r.status, records(&r)), (msg::STATUS_SUCCESS, 0));
        let r = read_string(&mut h, out, 7, (0, 0), 100);
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
    }

    #[test]
    fn a_string_read_pads_split_wide_characters() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "a日b", (0, 0));
        // From the trailing half: a space for it.
        let r = read_string(&mut h, out, REAL_UNICODE, (2, 0), 2 * 2);
        assert_eq!(units(&r), " b");
        // Ending on the leading half: a space for it.
        let r = read_string(&mut h, out, REAL_UNICODE, (0, 0), 2 * 2);
        assert_eq!(units(&r), "a ");
        let r = read_string(&mut h, out, ATTRIBUTE, (1, 0), 2);
        assert_eq!(msg::utf16_units(&r.output), [0x07]);
    }

    fn info(ch: char, attr: u16) -> CharInfo {
        CharInfo {
            ch: ch as u16,
            attr,
        }
    }

    #[test]
    fn a_block_write_is_clipped_at_each_edge() {
        let cells: Vec<CharInfo> = "abcdef"
            .chars()
            .zip([0x1e, 0x2f, 0x07, 0x4e, 0x70, 0x0c])
            .map(|(c, a)| info(c, a))
            .collect();
        let cases: [([i16; 4], [i16; 4], [&str; 2]); 4] = [
            // Off the right, the bottom, the left and the top.
            ([8, 1, 10, 2], [8, 1, 9, 2], ["ab", "de"]),
            ([2, 3, 4, 4], [2, 3, 4, 3], ["abc", ""]),
            ([-1, 0, 1, 1], [0, 0, 1, 1], ["bc", "ef"]),
            ([4, -1, 6, 0], [4, 0, 6, 0], ["def", ""]),
        ];
        for (region, wrote, text) in cases {
            let (mut h, out) = host();
            let r = write_block(&mut h, out, &cells, region);
            assert_eq!(r.status, msg::STATUS_SUCCESS, "{region:?}");
            assert_eq!(rect_at(&r, 0), wrote, "{region:?}");
            for (i, want) in text.iter().enumerate().filter(|(_, t)| !t.is_empty()) {
                let y = (wrote[1] as usize) + i;
                let got = &row(&h, y)[wrote[0] as usize..][..want.len()];
                assert_eq!(got, *want, "{region:?} row {y}");
            }
        }
    }

    #[test]
    fn a_block_write_off_the_screen_or_short_of_cells_fails_softly() {
        let (mut h, out) = host();
        let cells = vec![info('a', 7); 4];
        let r = write_block(&mut h, out, &cells, [10, 0, 11, 1]);
        assert_eq!(
            (r.status, rect_at(&r, 0)),
            (msg::STATUS_SUCCESS, [10, 0, 9, -1])
        );
        let r = write_block(&mut h, out, &cells, [0, 0, 2, 1]);
        assert_eq!(
            (r.status, rect_at(&r, 0)),
            (msg::STATUS_FROM_INVALIDARG, [0, 0, -1, -1])
        );
    }

    #[test]
    fn a_block_read_lays_out_the_request_and_clips() {
        let (mut h, out) = host();
        write_block(
            &mut h,
            out,
            &[info('a', 0x1e), info('b', 0x2f)],
            [8, 1, 9, 1],
        );
        let r = read_block(&mut h, out, [7, 0, 10, 2], 4 * 3 * 4);
        assert_eq!(
            (r.status, rect_at(&r, 0)),
            (msg::STATUS_SUCCESS, [7, 0, 9, 2])
        );
        let cells = block_cells(&r);
        assert_eq!(cells.len(), 12);
        // Row 1 of the request: ' ', a, b, then the column off the screen.
        assert_eq!(
            &cells[4..8],
            [
                info(' ', 7),
                info('a', 0x1e),
                info('b', 0x2f),
                info('\0', 0)
            ]
        );
    }

    #[test]
    fn a_block_read_needs_room_and_stays_under_the_reply_cap() {
        let (mut h, out) = host();
        let r = read_block(&mut h, out, [0, 0, 3, 1], 4 * 8 - 1);
        assert_eq!(r.status, msg::STATUS_FROM_INVALIDARG);
        // 512 x 513 cells is just over 1 MiB.
        let room = 512 * 513 * 4;
        let r = read_block(&mut h, out, [0, 0, 511, 512], room);
        assert_eq!(r.status, msg::STATUS_INVALID_PARAMETER);
        let r = read_block(&mut h, out, [0, 0, 511, 511], room);
        assert_eq!(
            (r.status, rect_at(&r, 0)),
            (msg::STATUS_SUCCESS, [0, 0, 9, 3])
        );
        assert_eq!(r.output.len(), super::super::read::READ_REPLY_CAP);
    }

    #[test]
    fn a_block_round_trips_wide_characters() {
        let (mut h, out) = host();
        let cells = [
            info('日', 7 | LEADING),
            info('日', 7 | TRAILING),
            info('x', 0x1e),
        ];
        write_block(&mut h, out, &cells, [0, 0, 2, 0]);
        assert_eq!(row(&h, 0), "日x       ");
        let r = read_block(&mut h, out, [0, 0, 2, 0], 12);
        assert_eq!(block_cells(&r), cells);
    }

    #[test]
    fn scroll_moves_text_and_fills_what_it_leaves() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "abcdef", (0, 0));
        let dot = info('.', 0x1e);
        let r = scroll(&mut h, out, [0, 0, 5, 0], None, (2, 2), dot);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
        assert_eq!(row(&h, 0), "......    ");
        assert_eq!(row(&h, 2), "  abcdef  ");
        assert_eq!(attrs(&h, 0)[0], 0x1e);
    }

    #[test]
    fn a_clipped_scroll_changes_nothing_outside_the_clip() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "abcdef", (0, 0));
        let dot = info('.', 0x1e);
        scroll(&mut h, out, [0, 0, 5, 0], Some([0, 0, 4, 3]), (2, 2), dot);
        assert_eq!(row(&h, 0), ".....f    ");
        assert_eq!(row(&h, 2), "  abc     ");
    }

    #[test]
    fn overlapping_scrolls_read_their_source_first() {
        let (mut h, out) = host();
        for (y, text) in ["0000", "1111", "2222", "3333"].iter().enumerate() {
            write_chars(&mut h, out, text, (0, y as i16));
        }
        let blank = info(' ', 7);
        // Down one: rows move like memmove.
        scroll(&mut h, out, [0, 0, 3, 2], None, (0, 1), blank);
        let rows: Vec<String> = (0..4).map(|y| row(&h, y)[..4].to_string()).collect();
        assert_eq!(rows, ["    ", "0000", "1111", "2222"]);
        // Up two.
        scroll(&mut h, out, [0, 2, 3, 3], None, (0, 0), blank);
        let rows: Vec<String> = (0..4).map(|y| row(&h, y)[..4].to_string()).collect();
        assert_eq!(rows, ["1111", "2222", "    ", "    "]);
        // Right two, within the row.
        scroll(&mut h, out, [0, 0, 3, 0], None, (2, 0), blank);
        assert_eq!(&row(&h, 0)[..6], "  1111");
    }

    #[test]
    fn scroll_fills_with_the_pen_for_a_zero_cell() {
        let (mut h, out) = host();
        h.write_w(out, "\x1b[44m");
        scroll(&mut h, out, [0, 0, 9, 3], None, (0, -4), info('\0', 0));
        assert!(attrs(&h, 3).iter().all(|&a| a == 0x17));
        assert_eq!(row(&h, 3), "          ");
    }

    #[test]
    fn a_scroll_that_moves_nothing_does_nothing() {
        let (mut h, out) = host();
        let mark = h.vt.len();
        let dot = info('.', 7);
        for (source, dest) in [([0, 0, 3, 0], (0, 0)), ([3, 0, 2, 0], (5, 5))] {
            let r = scroll(&mut h, out, source, None, dest, dot);
            assert_eq!(r.status, msg::STATUS_SUCCESS);
        }
        assert_eq!(h.vt.len(), mark);
    }

    #[test]
    fn a_large_paint_emits_in_slices() {
        let mut h = Host::sized(WinSize {
            rows: 200,
            cols: 400,
        });
        let [_, _, out] = h.connect(1, "");
        // 80,000 cells: many emission steps of CHUNK cells each.
        let r = fill(&mut h, out, ATTRIBUTE, 0x1e, 80_000, (0, 0));
        assert_eq!(r.status, i32::MIN, "held: more than one slice");
        let mut done = None;
        for _ in 0..100 {
            assert!(h.queued <= super::super::CLIENT_BUDGET + SLICE);
            let fx = h.drain();
            done = fx.iter().find_map(|e| match e {
                Effect::Complete(r) => Some(r.clone()),
                _ => None,
            });
            if done.is_some() {
                break;
            }
        }
        let done = done.expect("the fill completes");
        assert_eq!(length(&done), 80_000);
        assert_eq!(h.c.screen.cells(199, 399, 400)[0].attr, 0x1e);
    }

    /// A pen with a gridline (`COMMON_LVB_GRID_RVERTICAL`), which no SGR
    /// carries: only the host's own record keeps it.
    const GRID_PEN: u16 = 0x0807;

    fn set_pen(h: &mut Host, out: usize, attr: u16) {
        let r = h.call(out, api::SET_TEXT_ATTRIBUTE, &attr.to_le_bytes(), &[], 0);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
    }

    fn pen(h: &mut Host, out: usize) -> u16 {
        let r = h.call(out, api::GET_SCREEN_BUFFER_INFO, &[0; 92], &[], 0);
        msg::u16_at(&r.write_back, 12).expect("wAttributes")
    }

    #[test]
    fn painting_keeps_the_pens_gridlines() {
        let (mut h, out) = host();
        set_pen(&mut h, out, GRID_PEN);
        fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), 1, (0, 0));
        assert_eq!(row(&h, 0), "x         ");
        assert_eq!(pen(&mut h, out), GRID_PEN, "after a character fill");
        fill(&mut h, out, ATTRIBUTE, 0x1e, 2, (0, 1));
        assert_eq!(attrs(&h, 1)[..3], [0x1e, 0x1e, 7]);
        assert_eq!(pen(&mut h, out), GRID_PEN, "after an attribute fill");
        write_block(&mut h, out, &[info('b', 0x2f)], [3, 2, 3, 2]);
        assert_eq!(row(&h, 2), "   b      ");
        assert_eq!(pen(&mut h, out), GRID_PEN, "after a block write");
        scroll(&mut h, out, [0, 0, 9, 0], None, (0, 3), info('-', 0x07));
        assert_eq!(row(&h, 3), "x         ");
        assert_eq!(pen(&mut h, out), GRID_PEN, "after a scroll");
    }

    #[test]
    fn every_slice_of_a_held_paint_keeps_the_pens_gridlines() {
        let mut h = Host::sized(WinSize {
            rows: 200,
            cols: 400,
        });
        let [_, _, out] = h.connect(1, "");
        set_pen(&mut h, out, GRID_PEN);
        let r = fill(&mut h, out, ATTRIBUTE, 0x1e, 80_000, (0, 0));
        assert_eq!(r.status, i32::MIN, "held: more than one slice");
        let mut done = false;
        for _ in 0..100 {
            let fx = h.drain();
            assert_eq!(h.c.screen.attributes(), GRID_PEN, "between slices");
            done = fx.iter().any(|e| matches!(e, Effect::Complete(_)));
            if done {
                break;
            }
        }
        assert!(done, "the fill completes");
        assert_eq!(h.c.screen.cells(199, 399, 400)[0].attr, 0x1e);
        assert_eq!(pen(&mut h, out), GRID_PEN);
    }

    #[test]
    fn a_client_reset_still_clears_the_gridlines() {
        let (mut h, out) = host();
        set_pen(&mut h, out, GRID_PEN);
        fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), 1, (0, 0));
        h.write_a(out, b"\x1b[0m");
        assert_eq!(pen(&mut h, out), 0x07);
    }

    #[test]
    fn a_held_ansi_string_write_retains_only_its_bytes() {
        let mut h = Host::sized(WinSize {
            rows: 200,
            cols: 400,
        });
        let [_, _, out] = h.connect(1, "");
        let fixed = string_fixed((0, 0), ASCII);
        let text = vec![b'a'; 64_000];
        let r = h.call(out, api::WRITE_OUTPUT_STRING, &fixed, &text, 0);
        assert_eq!(r.status, i32::MIN, "held: more than one slice");
        assert_eq!(
            h.c.parked(),
            64_000 + STRING_LEN,
            "the bytes, not their UTF-16"
        );
        let mut done = None;
        for _ in 0..100 {
            let fx = h.drain();
            done = fx.iter().find_map(|e| match e {
                Effect::Complete(r) => Some(r.clone()),
                _ => None,
            });
            if done.is_some() {
                break;
            }
        }
        assert_eq!(records(&done.expect("the write completes")), 64_000);
        assert_eq!(h.c.screen.cells(159, 399, 400)[0].ch, u16::from(b'a'));
    }

    #[test]
    fn a_wide_scroll_fill_stays_in_its_span() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "abcdefghij", (0, 0));
        // Off the screen: all of the source is fill, five cells of it.
        scroll(&mut h, out, [0, 0, 4, 0], None, (0, 10), info('日', 7));
        assert_eq!(row(&h, 0), "日日 fghij");
        let flags: Vec<u16> = attrs(&h, 0)[..5].iter().map(|a| a & LT).collect();
        assert_eq!(flags, [LEADING, TRAILING, LEADING, TRAILING, 0]);
        assert_eq!(row(&h, 1), "          ", "nothing wraps");
    }

    #[test]
    fn painting_replaces_cells_under_insert_mode() {
        let (mut h, out) = host();
        write_chars(&mut h, out, "abcd", (0, 0));
        h.write_a(out, b"\x1b[4h");
        fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), 1, (0, 0));
        assert_eq!(row(&h, 0), "xbcd      ");
        assert!(
            h.c.screen.grid().insert_mode(),
            "the client's mode comes back"
        );
    }

    #[test]
    fn painting_prints_ascii_under_a_line_drawing_set() {
        let (mut h, out) = host();
        // Line drawing in G0, and in G1 shifted in with SO.
        h.write_a(out, b"\x1b(0\x1b)0\x0e");
        write_chars(&mut h, out, "q", (0, 0));
        assert_eq!(row(&h, 0), "q         ");
        h.write_a(out, b"q");
        // The paint left the cursor where it was, so this prints over it.
        assert_eq!(
            row(&h, 0),
            "\u{2500}         ",
            "the client's sets come back"
        );
    }

    #[test]
    fn one_step_on_tall_narrow_rows_fits_a_slice() {
        // One cell a row, each a run of its own with every wrapper: origin
        // and insert mode on, five-digit rows, the longest SGR, and the
        // longest client prefix the screen holds flushed ahead of them.
        let mut h = Host::sized(WinSize {
            rows: 20_000,
            cols: 1,
        });
        let [_, _, out] = h.connect(1, "");
        h.write_a(out, b"\x1b[?6h\x1b[4h");
        let mut held = b"\x1b[?".to_vec();
        held.resize(WATCHED_MAX - 1, b'1');
        h.write_a(out, &held);
        let op = Op::Stream {
            what: Stream::FillAttr(0xC4FF),
            len: 20_000,
            x: 0,
            y: 10_000,
            pos: 0,
        };
        let mut paint = Paint::new(op, None);
        let mut vt = OutputQueue::default();
        let codec = h.c.codec;
        assert!(!paint.step(&mut h.c.screen, &mut vt, &codec), "more to do");
        let bytes = vt.take();
        assert!(bytes.starts_with(&held), "the held prefix goes first");
        let runs = bytes.windows(2).filter(|w| w == b"\x1b7").count();
        assert_eq!(runs, CHUNK, "a whole chunk, a run a cell");
        assert!(bytes.len() <= SLICE, "{} bytes in one step", bytes.len());
        // 55 bytes a run here, so the old CHUNK of 1,260 made 69,300.
        assert_eq!(bytes.len(), held.len() + CHUNK * 55, "every wrapper is in");
    }

    #[test]
    fn a_split_run_keeps_wide_halves_together() {
        let mut row = vec![info('a', 7); CHUNK - 1];
        row.extend([
            info('日', 7 | LEADING),
            info('日', 7 | TRAILING),
            info('b', 7),
        ]);
        let mut h = Host::sized(WinSize {
            rows: 1,
            cols: (CHUNK + 2) as u16,
        });
        let [_, _, out] = h.connect(1, "");
        let width = row.len() as i16;
        write_block(&mut h, out, &row, [0, 0, width - 1, 0]);
        let back = h.c.screen.cells(0, CHUNK - 1, CHUNK + 2);
        assert_eq!(back, row[CHUNK - 1..]);
    }

    #[test]
    fn cell_calls_check_the_handle() {
        let (mut h, out) = host();
        let reader = h
            .create(1, msg::OBJECT_CURRENT_OUTPUT, msg::GENERIC_READ)
            .information;
        let r = fill(&mut h, reader, REAL_UNICODE, 0x20, 1, (0, 0));
        assert_eq!(r.status, msg::STATUS_FROM_ACCESS_DENIED);
        let writer = h
            .create(1, msg::OBJECT_CURRENT_OUTPUT, msg::GENERIC_WRITE)
            .information;
        let r = read_block(&mut h, writer, [0, 0, 0, 0], 4);
        assert_eq!(r.status, msg::STATUS_FROM_ACCESS_DENIED);
        let r = read_block(&mut h, out, [0, 0, 0, 0], 4);
        assert_eq!(r.status, msg::STATUS_SUCCESS);
    }

    #[test]
    fn a_paint_held_across_a_shrink_stays_on_the_screen() {
        let mut h = Host::sized(WinSize {
            rows: 200,
            cols: 400,
        });
        let [_, _, out] = h.connect(1, "");
        let r = fill(&mut h, out, REAL_UNICODE, u16::from(b'x'), 80_000, (0, 0));
        assert_eq!(r.status, i32::MIN, "held");
        h.ev(super::super::Event::Resize(WinSize { rows: 3, cols: 5 }));
        let done = (0..100)
            .flat_map(|_| h.drain())
            .find_map(|e| match e {
                Effect::Complete(r) => Some(r),
                _ => None,
            })
            .expect("the fill completes");
        assert_eq!(done.status, msg::STATUS_SUCCESS);
        assert_eq!(h.c.screen.size(), WinSize { rows: 3, cols: 5 });
    }
}
