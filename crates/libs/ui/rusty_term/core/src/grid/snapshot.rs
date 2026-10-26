//! Full-state snapshot of a [`Grid`]: a versioned binary encoding of what a
//! client needs to draw the terminal, and its decoder. A mux server sends one
//! when a client attaches.
//!
//! Included: size, cursor (position, saved position, visibility, shape,
//! blink), scroll region, autowrap, title, the grapheme-cluster and hyperlink
//! tables, scrollback, the live screen and, while the alternate screen is
//! active, the stashed primary screen. Not included: parser state, input
//! modes (mouse, keypad, bracketed paste), the colour palette, images,
//! selection, search and folds.
//!
//! # Format (version 1)
//!
//! All integers are unsigned LEB128 varints unless noted.
//!
//! ```text
//! snapshot = "RTSN" version:u8 header tables lines(history) lines(rows) primary
//! header   = cols rows cursor.x cursor.y saved.x saved.y scroll_top scroll_bottom
//!            flags:u8 shape:u8 title:string
//! tables   = count string*  (clusters), count string*  (links)
//! lines(n) = n line*        (history is preceded by its count; the screen is not)
//! primary  = 0:u8 | 1:u8 cursor.x cursor.y saved.x saved.y mode:u8 lines(rows)
//! line     = ncells meta:u8 run*         (runs cover exactly ncells cells)
//! run      = (len << 2 | repeat << 1 | rich) style glyph{repeat ? 1 : len}
//! style    = id                          (id == styles seen so far: a new
//!            | id fg bg flags link ul     style, defined inline)
//! glyph    = char                        (plain run)
//!          | (char << 2 | trailer << 1 | clustered) [cluster]   (rich run)
//! string   = len bytes                   (UTF-8)
//! ```
//!
//! A style is a cell's colours, attributes and link; the wide-glyph trailer
//! bit travels with the glyph, so a wide glyph does not split its run. A run
//! with no trailer and no cluster is plain: each glyph is its bare code point,
//! one byte for ASCII.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use rusty_wire::Reader;

use super::{AltMode, CursorShape, Grid, Line, LineAttr, SavedScreen};
use crate::cell::{Cell, PackedCell, WIDE_TRAILER};
use crate::style::{CellStyle, StyleId, StyleTable};

const MAGIC: &[u8; 4] = b"RTSN";
const VERSION: u8 = 1;
/// Largest width or height a snapshot may declare.
const MAX_DIM: usize = 4096;
/// Most cells a snapshot may decode to (history and screens together), so a
/// small hostile input cannot claim gigabytes through repeat runs.
const MAX_CELLS: usize = 1 << 25;
/// Most lines a snapshot may decode to (history and screens together). Each
/// line costs a header even when empty, so lines have a budget of their own.
const MAX_LINES: usize = 1 << 20;
/// Most distinct grid styles a snapshot may decode to. A grid style adds the
/// cluster to a snapshot style, so a hostile input could otherwise multiply
/// the two tables into one far larger than either.
const MAX_STYLES: usize = 1 << 22;
/// Identical cells in a row from which a repeat run beats a literal one.
const MIN_REPEAT: usize = 4;

const FLAG_CURSOR_VISIBLE: u8 = 1;
const FLAG_CURSOR_BLINK: u8 = 2;
const FLAG_AUTOWRAP: u8 = 4;
const META_WRAPPED: u8 = 1;

/// Why a snapshot could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The input ended inside a field.
    Truncated,
    /// The input does not start with the snapshot magic.
    BadMagic,
    /// The snapshot was written by an unknown format version.
    UnsupportedVersion(u8),
    /// A field holds a value the format does not allow.
    Invalid(&'static str),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::Truncated => write!(f, "snapshot truncated"),
            SnapshotError::BadMagic => write!(f, "not a rusty_term snapshot"),
            SnapshotError::UnsupportedVersion(v) => write!(f, "unsupported snapshot version {v}"),
            SnapshotError::Invalid(what) => write!(f, "invalid snapshot: {what}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<rusty_wire::Error> for SnapshotError {
    fn from(_: rusty_wire::Error) -> Self {
        SnapshotError::Truncated
    }
}

type Result<T> = std::result::Result<T, SnapshotError>;

/// The part of a cell shared by a run: everything except the glyph.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Style {
    fg: u32,
    bg: u32,
    flags: u16,
    link: u16,
    underline_color: u32,
}

impl Style {
    fn of(cell: &Cell) -> Self {
        Style {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags & !WIDE_TRAILER,
            link: cell.link,
            underline_color: cell.underline_color,
        }
    }
}

impl Grid {
    /// Encode the grid's drawable state (see the module docs) into a
    /// self-contained, versioned snapshot.
    pub fn encode_snapshot(&self) -> Vec<u8> {
        let screen_cells = self.cols * self.rows;
        let history_cells: usize = self.scrollback.iter().map(|l| l.cells().len()).sum();
        let mut enc = Encoder {
            out: Vec::with_capacity(history_cells + screen_cells + 1024),
            styles: HashMap::new(),
            table: &self.styles,
            row: Vec::with_capacity(self.cols),
        };
        enc.out.extend_from_slice(MAGIC);
        enc.out.push(VERSION);
        for v in [
            self.cols,
            self.rows,
            self.cursor.0,
            self.cursor.1,
            self.saved_cursor.0,
            self.saved_cursor.1,
            self.scroll_top,
            self.scroll_bottom,
        ] {
            enc.varint(v as u64);
        }
        let mut flags = 0;
        for (on, bit) in [
            (self.cursor_visible, FLAG_CURSOR_VISIBLE),
            (self.cursor_blink, FLAG_CURSOR_BLINK),
            (self.autowrap, FLAG_AUTOWRAP),
        ] {
            if on {
                flags |= bit;
            }
        }
        enc.out.push(flags);
        enc.out.push(shape_code(self.cursor_shape));
        enc.string(&self.title);
        for table in [&self.clusters, &self.links] {
            enc.varint(table.len() as u64);
            for s in table {
                enc.string(s);
            }
        }
        enc.varint(self.scrollback.len() as u64);
        for line in &self.scrollback {
            enc.line(line);
        }
        for line in &self.screen {
            enc.line(line);
        }
        match &self.primary {
            None => enc.out.push(0),
            Some(p) => {
                enc.out.push(1);
                for v in [p.cursor.0, p.cursor.1, p.saved_cursor.0, p.saved_cursor.1] {
                    enc.varint(v as u64);
                }
                enc.out.push(mode_code(p.mode));
                for line in &p.screen {
                    enc.line(line);
                }
            }
        }
        enc.out
    }

    /// Rebuild a grid from [`Grid::encode_snapshot`] output. Input is
    /// validated as untrusted: every count, index and dimension is checked,
    /// and the total decoded size is capped.
    pub fn decode_snapshot(bytes: &[u8]) -> Result<Grid> {
        let mut d = Decoder::new(bytes);
        if d.r.read_array::<4>().map_err(|_| SnapshotError::BadMagic)? != *MAGIC {
            return Err(SnapshotError::BadMagic);
        }
        let version = d.r.read_u8()?;
        if version != VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }
        let cols = d.dim()?;
        let rows = d.dim()?;
        // Everything is read and validated before the grid is built, so a
        // truncated or hostile header allocates no screen.
        let cursor = d.point(cols, rows)?;
        let saved_cursor = d.point(cols, rows)?;
        let (top, bottom) = (d.usize()?, d.usize()?);
        if top > bottom || bottom >= rows {
            return Err(SnapshotError::Invalid("scroll region"));
        }
        let flags = d.r.read_u8()?;
        let cursor_shape = shape_from(d.r.read_u8()?)?;
        let title = d.string()?;
        let clusters = d.table()?;
        let links = d.table()?;
        (d.clusters, d.links) = (clusters.len(), links.len());
        let history = d.usize()?;
        if history > MAX_LINES {
            return Err(SnapshotError::Invalid("too many lines"));
        }
        let mut scrollback = VecDeque::new();
        for _ in 0..history {
            scrollback.push_back(d.line(cols, false)?);
        }
        let screen = d.screen(cols, rows)?;
        let primary = match d.r.read_u8()? {
            0 => None,
            1 => {
                let cursor = d.point(cols, rows)?;
                let saved_cursor = d.point(cols, rows)?;
                let mode = mode_from(d.r.read_u8()?)?;
                let screen = d.screen(cols, rows)?;
                Some(SavedScreen {
                    screen,
                    cursor,
                    saved_cursor,
                    mode,
                })
            }
            _ => return Err(SnapshotError::Invalid("primary screen marker")),
        };
        if !d.r.is_empty() {
            return Err(SnapshotError::Invalid("trailing bytes"));
        }
        let mut g = Grid::with_screen(cols, rows, screen, d.table);
        (g.cursor, g.saved_cursor) = (cursor, saved_cursor);
        (g.scroll_top, g.scroll_bottom) = (top, bottom);
        g.cursor_visible = flags & FLAG_CURSOR_VISIBLE != 0;
        g.cursor_blink = flags & FLAG_CURSOR_BLINK != 0;
        g.autowrap = flags & FLAG_AUTOWRAP != 0;
        g.cursor_shape = cursor_shape;
        g.title = title;
        g.links_arc = Arc::from(links.clone());
        (g.clusters, g.links) = (clusters, links);
        g.rebuild_cluster_caches();
        g.scrollback_max = g.scrollback_max.max(history);
        g.scrollback = scrollback;
        g.primary = primary;
        g.reset_damage();
        g.touch_hist();
        Ok(g)
    }
}

struct Encoder<'a> {
    out: Vec<u8>,
    styles: HashMap<Style, u32>,
    /// The grid's style table, to resolve stored cells.
    table: &'a StyleTable,
    /// The line being written, resolved; reused across lines.
    row: Vec<Cell>,
}

impl Encoder<'_> {
    fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.out.push(v as u8 | 0x80);
            v >>= 7;
        }
        self.out.push(v as u8);
    }

    fn string(&mut self, s: &str) {
        self.varint(s.len() as u64);
        self.out.extend_from_slice(s.as_bytes());
    }

    /// A line as runs: [`MIN_REPEAT`] or more identical cells become a
    /// repeat run; other cells group into literal runs of one style.
    fn line(&mut self, line: &Line) {
        let mut row = std::mem::take(&mut self.row);
        row.clear();
        let table = self.table;
        row.extend(
            line.cells()
                .iter()
                .map(|c| c.resolve(&table.get(c.style()))),
        );
        let cells = &row[..];
        self.varint(cells.len() as u64);
        let wrapped = if line.wrapped { META_WRAPPED } else { 0 };
        self.out.push(wrapped | attr_code(line.attr) << 1);
        let mut i = 0;
        while i < cells.len() {
            let first = cells[i];
            let same = cells[i..].iter().take_while(|c| **c == first).count();
            if same >= MIN_REPEAT {
                self.run(&cells[i..=i], same, true, is_rich(&first));
                i += same;
                continue;
            }
            let mut rich = is_rich(&first);
            let mut j = i + 1;
            while j < cells.len() && same_style(&cells[j], &first) && !starts_repeat(&cells[j..]) {
                rich |= is_rich(&cells[j]);
                j += 1;
            }
            self.run(&cells[i..j], j - i, false, rich);
            i = j;
        }
        self.row = row;
    }

    /// One run of `len` cells: `glyphs` holds the one repeated cell, or all
    /// `len` cells of a literal run. Every glyph shares the first's style;
    /// `rich` says whether any has a trailer or a cluster.
    fn run(&mut self, glyphs: &[Cell], len: usize, repeat: bool, rich: bool) {
        self.varint((len as u64) << 2 | u64::from(repeat) << 1 | u64::from(rich));
        self.style(Style::of(&glyphs[0]));
        for c in glyphs {
            if !rich {
                match u8::try_from(c.ch) {
                    Ok(b) if b < 0x80 => self.out.push(b),
                    _ => self.varint(u64::from(c.ch)),
                }
                continue;
            }
            let trailer = u64::from(c.flags & WIDE_TRAILER != 0);
            let clustered = u64::from(c.cluster != 0);
            self.varint(u64::from(c.ch) << 2 | trailer << 1 | clustered);
            if c.cluster != 0 {
                self.varint(u64::from(c.cluster));
            }
        }
    }

    fn style(&mut self, style: Style) {
        let next = self.styles.len() as u32;
        let id = *self.styles.entry(style).or_insert(next);
        self.varint(u64::from(id));
        if id == next {
            for v in [
                style.fg,
                style.bg,
                style.flags.into(),
                style.link.into(),
                style.underline_color,
            ] {
                self.varint(u64::from(v));
            }
        }
    }
}

/// Whether `cell` needs a rich glyph: a wide-glyph trailer or a cluster.
fn is_rich(cell: &Cell) -> bool {
    cell.cluster != 0 || cell.flags & WIDE_TRAILER != 0
}

/// Whether `a` and `b` share a [`Style`], without building either.
fn same_style(a: &Cell, b: &Cell) -> bool {
    a.fg == b.fg
        && a.bg == b.bg
        && (a.flags ^ b.flags) & !WIDE_TRAILER == 0
        && a.link == b.link
        && a.underline_color == b.underline_color
}

/// Whether `cells` starts with [`MIN_REPEAT`] identical cells.
fn starts_repeat(cells: &[Cell]) -> bool {
    cells.len() >= MIN_REPEAT && cells[1..MIN_REPEAT].iter().all(|c| *c == cells[0])
}

struct Decoder<'a> {
    r: Reader<'a>,
    styles: Vec<Style>,
    /// The decoded grid's styles.
    table: StyleTable,
    /// The last style interned and its id: runs repeat one style.
    last: Option<(CellStyle, StyleId)>,
    /// [`MAX_STYLES`]; lowered by tests.
    max_styles: usize,
    cells_left: usize,
    lines_left: usize,
    clusters: usize,
    links: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Decoder {
            r: Reader::new(bytes),
            styles: Vec::new(),
            table: StyleTable::default(),
            last: None,
            max_styles: MAX_STYLES,
            cells_left: MAX_CELLS,
            lines_left: MAX_LINES,
            clusters: 0,
            links: 0,
        }
    }

    fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.r.read_u8()?;
            // The tenth byte holds bit 63 only: a larger payload, or a
            // further byte, would overflow `u64`.
            if shift == 63 && b > 1 {
                return Err(SnapshotError::Invalid("varint overflow"));
            }
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(SnapshotError::Invalid("varint overflow"))
    }

    fn usize(&mut self) -> Result<usize> {
        usize::try_from(self.varint()?).map_err(|_| SnapshotError::Invalid("integer too large"))
    }

    fn u32(&mut self) -> Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| SnapshotError::Invalid("integer too large"))
    }

    fn u16(&mut self) -> Result<u16> {
        u16::try_from(self.varint()?).map_err(|_| SnapshotError::Invalid("integer too large"))
    }

    fn dim(&mut self) -> Result<usize> {
        match self.usize()? {
            n @ 1..=MAX_DIM => Ok(n),
            _ => Err(SnapshotError::Invalid("dimension")),
        }
    }

    /// A cursor position: column up to `cols` (the pending-wrap column),
    /// row below `rows`.
    fn point(&mut self, cols: usize, rows: usize) -> Result<(usize, usize)> {
        let (x, y) = (self.usize()?, self.usize()?);
        if x > cols || y >= rows {
            return Err(SnapshotError::Invalid("cursor position"));
        }
        Ok((x, y))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.usize()?;
        let bytes = self.r.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| SnapshotError::Invalid("string is not UTF-8"))
    }

    fn table(&mut self) -> Result<Vec<String>> {
        let n = self.usize()?;
        if n > usize::from(u16::MAX) {
            return Err(SnapshotError::Invalid("table size"));
        }
        (0..n).map(|_| self.string()).collect()
    }

    fn screen(&mut self, cols: usize, rows: usize) -> Result<Vec<Line>> {
        (0..rows).map(|_| self.line(cols, true)).collect()
    }

    /// One line of at most `cols` cells, exactly `cols` on a screen.
    fn line(&mut self, cols: usize, screen: bool) -> Result<Line> {
        let n = self.usize()?;
        if n > cols || screen && n != cols {
            return Err(SnapshotError::Invalid("line width"));
        }
        self.cells_left = self
            .cells_left
            .checked_sub(n)
            .ok_or(SnapshotError::Invalid("too many cells"))?;
        self.lines_left = self
            .lines_left
            .checked_sub(1)
            .ok_or(SnapshotError::Invalid("too many lines"))?;
        let meta = self.r.read_u8()?;
        if meta >> 3 != 0 {
            return Err(SnapshotError::Invalid("line meta"));
        }
        let mut cells = Vec::with_capacity(n);
        while cells.len() < n {
            let head = self.usize()?;
            let (len, repeat, rich) = (head >> 2, head & 2 != 0, head & 1 != 0);
            if len == 0 || len > n - cells.len() {
                return Err(SnapshotError::Invalid("run length"));
            }
            let style = self.style()?;
            if repeat {
                let cell = self.glyph(style, rich)?;
                let cell = self.pack(&cell)?;
                cells.resize(cells.len() + len, cell);
            } else {
                for _ in 0..len {
                    let cell = self.glyph(style, rich)?;
                    cells.push(self.pack(&cell)?);
                }
            }
        }
        Ok(Line::new(
            cells,
            meta & META_WRAPPED != 0,
            attr_from(meta >> 1)?,
        ))
    }

    fn style(&mut self) -> Result<Style> {
        let id = self.usize()?;
        if let Some(style) = self.styles.get(id) {
            return Ok(*style);
        }
        if id != self.styles.len() {
            return Err(SnapshotError::Invalid("style id"));
        }
        let style = Style {
            fg: self.u32()?,
            bg: self.u32()?,
            flags: self.u16()?,
            link: self.u16()?,
            underline_color: self.u32()?,
        };
        if style.flags & WIDE_TRAILER != 0 || usize::from(style.link) > self.links {
            return Err(SnapshotError::Invalid("style"));
        }
        self.styles.push(style);
        Ok(style)
    }

    /// Store `cell`, interning its style into the decoded grid's table.
    fn pack(&mut self, cell: &Cell) -> Result<PackedCell> {
        let style = CellStyle::of(cell);
        if let Some((last, id)) = self.last
            && last == style
        {
            return Ok(PackedCell::new(cell, id));
        }
        let id = self.table.intern(style);
        if self.table.len() > self.max_styles {
            return Err(SnapshotError::Invalid("too many styles"));
        }
        self.last = Some((style, id));
        Ok(PackedCell::new(cell, id))
    }

    fn glyph(&mut self, style: Style, rich: bool) -> Result<Cell> {
        let v = self.varint()?;
        let (code, bits) = if rich { (v >> 2, v & 3) } else { (v, 0) };
        let ch = u32::try_from(code)
            .ok()
            .and_then(char::from_u32)
            .ok_or(SnapshotError::Invalid("character"))?;
        let cluster = if bits & 1 == 1 {
            match self.u16()? {
                id @ 1.. if usize::from(id) <= self.clusters => id,
                _ => return Err(SnapshotError::Invalid("cluster id")),
            }
        } else {
            0
        };
        let trailer = if bits & 2 == 2 { WIDE_TRAILER } else { 0 };
        Ok(Cell {
            ch,
            cluster,
            fg: style.fg,
            bg: style.bg,
            flags: style.flags | trailer,
            link: style.link,
            underline_color: style.underline_color,
        })
    }
}

fn attr_code(attr: LineAttr) -> u8 {
    match attr {
        LineAttr::Single => 0,
        LineAttr::DoubleWidth => 1,
        LineAttr::DoubleTop => 2,
        LineAttr::DoubleBottom => 3,
    }
}

fn attr_from(code: u8) -> Result<LineAttr> {
    match code {
        0 => Ok(LineAttr::Single),
        1 => Ok(LineAttr::DoubleWidth),
        2 => Ok(LineAttr::DoubleTop),
        3 => Ok(LineAttr::DoubleBottom),
        _ => Err(SnapshotError::Invalid("line attribute")),
    }
}

fn shape_code(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Bar => 2,
    }
}

fn shape_from(code: u8) -> Result<CursorShape> {
    match code {
        0 => Ok(CursorShape::Block),
        1 => Ok(CursorShape::Underline),
        2 => Ok(CursorShape::Bar),
        _ => Err(SnapshotError::Invalid("cursor shape")),
    }
}

fn mode_code(mode: AltMode) -> u8 {
    match mode {
        AltMode::Dec47 => 0,
        AltMode::Dec1047 => 1,
        AltMode::Dec1049 => 2,
    }
}

fn mode_from(code: u8) -> Result<AltMode> {
    match code {
        0 => Ok(AltMode::Dec47),
        1 => Ok(AltMode::Dec1047),
        2 => Ok(AltMode::Dec1049),
        _ => Err(SnapshotError::Invalid("alternate screen mode")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(bytes: &[u8]) -> Result<u64> {
        let mut d = Decoder::new(bytes);
        d.varint()
    }

    #[test]
    fn varint_accepts_the_full_u64_range_and_rejects_overflow() {
        let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(varint(&max), Ok(u64::MAX));
        let top_bit = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        assert_eq!(varint(&top_bit), Ok(1 << 63));
        assert_eq!(varint(&[0x00]), Ok(0));
        assert_eq!(varint(&[0x7f]), Ok(127));
        assert_eq!(varint(&[0x80, 0x01]), Ok(128));
        // A tenth byte with payload 2 or 127, or one that continues, would
        // overflow (81 80 .. 80 02 used to wrap to 1).
        let overflow = Err(SnapshotError::Invalid("varint overflow"));
        let nine = [0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
        for tenth in [0x02, 0x7f, 0x81, 0x80] {
            let mut v = nine.to_vec();
            v.push(tenth);
            v.push(0x00); // a continuing tenth byte must not reach this
            assert_eq!(varint(&v), overflow, "tenth byte {tenth:#04x}");
        }
        assert_eq!(varint(&nine), Err(SnapshotError::Truncated));
    }
    #[test]
    fn decoding_past_the_style_cap_fails() {
        // One line of three cells, each a new style (fg 1, 2, 3): with the
        // blank style the table reaches four entries.
        let mut line = vec![3, 0];
        for (id, fg) in [(0u8, 1u8), (1, 2), (2, 3)] {
            line.extend([1 << 2, id, fg, 0, 0, 0, 0, b'a']);
        }
        for (cap, want) in [
            (4, Ok(())),
            (3, Err(SnapshotError::Invalid("too many styles"))),
        ] {
            let mut d = Decoder::new(&line);
            d.max_styles = cap;
            assert_eq!(d.line(3, true).map(|_| ()), want, "cap {cap}");
        }
    }
}
