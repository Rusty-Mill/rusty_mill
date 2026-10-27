//! RTCP v3: a headless terminal continuation checkpoint, independent of RTSN.
//! Durable parser/Grid state is included; front-end view state and pending
//! host side effects are excluded. L13 channels are outside this profile.
//! Lines are run-encoded (see `grid/line.rs`); v3 differs from v2 only there.

use crate::{
    AnsiParser, Grid,
    cell::{Cell, PackedCell},
    style::{CellStyle, GRID_STYLE_BYTES, StyleId, StyleTable},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

/// Maximum encoded checkpoint bytes (independent of lifetime PTY output).
pub const MAX_ENCODED: usize = 256 << 20;
/// Aggregate heap allocation allowed while decoding one checkpoint.
pub const MAX_DECODED: usize = 512 << 20;
/// Malformed, unsupported or over-budget checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub(crate) type Result<T> = std::result::Result<T, Error>;

/// Magic and version byte that open every checkpoint.
const HEADER: &[u8; 5] = b"RTCP\x03";

/// Encode a consistent headless Grid and parser. No output log is retained.
pub fn encode(grid: &Grid, parser: &AnsiParser) -> Result<Vec<u8>> {
    // Cell style ids are checked per run as the lines are written.
    grid.validate_checkpoint()?;
    parser.validate_checkpoint()?;
    let mut w = Writer::new(MAX_DECODED, Some(grid.styles()));
    w.put(HEADER)?;
    grid.write(&mut w)?;
    parser.write(&mut w)?;
    Ok(w.bytes)
}

/// Decode untrusted RTCP v3, validating allocation budgets and internal indices
/// before returning objects that can safely continue parsing. No host actions
/// or terminal responses are replayed while restoring.
pub fn decode(data: &[u8]) -> Result<(Grid, AnsiParser)> {
    if data.len() > MAX_ENCODED || !data.starts_with(HEADER) {
        return Err(Error("checkpoint header/version/size"));
    }
    let mut r = Reader::new(&data[HEADER.len()..], MAX_DECODED);
    let grid = Grid::read(&mut r)?;
    let parser = AnsiParser::read(&mut r)?;
    grid.validate_checkpoint()?;
    grid.validate_cell_styles()?;
    parser.validate_checkpoint()?;
    if !r.bytes.is_empty() {
        return Err(Error("trailing checkpoint data"));
    }
    Ok((grid, parser))
}

pub(crate) struct Writer<'a> {
    bytes: Vec<u8>,
    now: Instant,
    budget: usize,
    /// The encoded grid's style table, which `Line` runs resolve through.
    /// `None` only for writers that never see a `Line`.
    table: Option<&'a StyleTable>,
    /// Line run styles already defined, by index (see [`Writer::style`]).
    styles: HashMap<Cell, usize>,
    /// `1 + index` of each run style already defined, by grid style id and
    /// layout bits ([`layout_slot`]); 0 until first referenced. Spares
    /// resolving and hashing a style for every run.
    ids: Vec<u32>,
}
impl<'a> Writer<'a> {
    pub(crate) fn new(budget: usize, table: Option<&'a StyleTable>) -> Self {
        Writer {
            bytes: Vec::new(),
            now: Instant::now(),
            budget,
            table,
            styles: HashMap::new(),
            ids: vec![0; table.map_or(0, |t| t.len() * LAYOUTS)],
        }
    }
    /// LEB128 varint: 7 bits per byte, low group first, high bit = more.
    pub(crate) fn varint(&mut self, mut v: u64) -> Result<()> {
        let mut buf = [0u8; 10];
        let mut n = 0;
        loop {
            let low = (v & 0x7f) as u8;
            v >>= 7;
            buf[n] = low | if v == 0 { 0 } else { 0x80 };
            n += 1;
            if v == 0 {
                return self.put(&buf[..n]);
            }
        }
    }
    /// One scalar as its UTF-8 bytes.
    pub(crate) fn utf8(&mut self, ch: char) -> Result<()> {
        self.put(ch.encode_utf8(&mut [0; 4]).as_bytes())
    }
    /// Scalars as UTF-8, in one pass. ASCII goes straight into spare
    /// capacity, which [`Writer::reserve`] keeps within [`MAX_ENCODED`].
    pub(crate) fn text(&mut self, chars: impl Iterator<Item = char>) -> Result<()> {
        for ch in chars {
            if ch.is_ascii() && self.bytes.len() < self.bytes.capacity().min(MAX_ENCODED) {
                self.bytes.push(ch as u8);
            } else {
                self.utf8(ch)?;
            }
        }
        Ok(())
    }
    /// Reference the style of the run `cell` starts (see [`Writer::style`]).
    /// Every cell of a run shares its style id, so checking that the id is
    /// live here checks every cell written.
    pub(crate) fn run_style(&mut self, cell: PackedCell) -> Result<()> {
        let table = self.table.ok_or(Error("line without style table"))?;
        let id = cell.style();
        if table.free_mask().get(id.0 as usize) != Some(&false) {
            return Err(Error("cell bounds"));
        }
        let slot = layout_slot(cell);
        if let Some(&i) = slot.and_then(|s| self.ids.get(s))
            && i > 0
        {
            return self.varint(u64::from(i));
        }
        let index = self.style(cell.resolve(&table.get(id)))?;
        if let Some(s) = slot
            && let Some(entry) = self.ids.get_mut(s)
        {
            *entry = u32::try_from(index + 1).unwrap_or(0);
        }
        Ok(())
    }
    /// Reference a run style (a cell whose `ch` is ignored): `0` followed by
    /// its definition the first time, then `1 + index` of that definition.
    /// A new definition is charged for both the decoder's wire table and the
    /// grid style it may intern (every grid style arrives with a new wire
    /// style, so the grid table never outgrows the wire table plus id 0).
    /// Returns the style's index.
    pub(crate) fn style(&mut self, cell: Cell) -> Result<usize> {
        let key = Cell { ch: '\0', ..cell };
        if let Some(&i) = self.styles.get(&key) {
            self.varint(i as u64 + 1)?;
            return Ok(i);
        }
        let i = self.styles.len();
        self.charge(style_growth(i) * STYLE_COST)?;
        self.varint(0)?;
        write_style(&key, self)?;
        self.styles.insert(key, i);
        Ok(i)
    }
    pub(crate) fn charge(&mut self, n: usize) -> Result<()> {
        self.budget = self
            .budget
            .checked_sub(n)
            .ok_or(Error("decoded checkpoint budget"))?;
        Ok(())
    }
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.reserve(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    /// Room for `n` more bytes within [`MAX_ENCODED`], growing by doubling.
    fn reserve(&mut self, n: usize) -> Result<()> {
        if n > MAX_ENCODED.saturating_sub(self.bytes.len()) {
            return Err(Error("encoded checkpoint budget"));
        }
        let needed = self.bytes.len() + n;
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_ENCODED);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| Error("checkpoint allocation failed"))?;
        }
        Ok(())
    }
}
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    budget: usize,
    now: Instant,
    /// Line run styles defined so far, in definition order, each with the
    /// grid style id it interned as.
    styles: Vec<(Cell, StyleId)>,
    /// The decoded grid's style table, taken by `Grid::read`.
    table: StyleTable,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], budget: usize) -> Self {
        Reader {
            bytes,
            budget,
            now: Instant::now(),
            styles: Vec::new(),
            table: StyleTable::default(),
        }
    }
    /// The style table built while decoding lines.
    pub(crate) fn take_styles(&mut self) -> StyleTable {
        std::mem::take(&mut self.table)
    }
    /// See [`Writer::varint`]. More than 64 bits fails.
    pub(crate) fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.get(1)?[0];
            let group = u64::from(b & 0x7f);
            if shift == 63 && group > 1 {
                return Err(Error("varint overflow"));
            }
            v |= group << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(Error("varint overflow"))
    }
    /// See [`Writer::utf8`]: one validated scalar.
    pub(crate) fn utf8(&mut self) -> Result<char> {
        let len = match self.bytes.first() {
            Some(&b) if b.is_ascii() => {
                self.bytes = &self.bytes[1..];
                return Ok(char::from(b));
            }
            Some(0xc2..=0xdf) => 2,
            Some(0xe0..=0xef) => 3,
            Some(0xf0..=0xf4) => 4,
            Some(_) => return Err(Error("UTF8 scalar")),
            None => return Err(Error("truncated checkpoint")),
        };
        std::str::from_utf8(self.get(len)?)
            .ok()
            .and_then(|s| s.chars().next())
            .ok_or(Error("UTF8 scalar"))
    }
    /// See [`Writer::style`]: the referenced style, `ch` set to NUL, and its
    /// grid style id. A new definition is interned once, here; the grid
    /// table is reserved in step with the wire table, inside the charge.
    pub(crate) fn style(&mut self) -> Result<(Cell, StyleId)> {
        let i = self.varint()?;
        if i == 0 {
            let grow = style_growth(self.styles.len());
            if grow > 0 {
                self.charge(grow * STYLE_COST)?;
                self.styles
                    .try_reserve_exact(grow)
                    .map_err(|_| Error("allocation failed"))?;
                self.table
                    .try_reserve(grow)
                    .map_err(|_| Error("allocation failed"))?;
            }
            let style = read_style(self)?;
            let id = self.table.intern(CellStyle::of(&style));
            self.styles.push((style, id));
            return Ok((style, id));
        }
        usize::try_from(i - 1)
            .ok()
            .and_then(|i| self.styles.get(i).copied())
            .ok_or(Error("style index"))
    }
    fn get(&mut self, n: usize) -> Result<&[u8]> {
        if n > self.bytes.len() {
            return Err(Error("truncated checkpoint"));
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }
    pub(crate) fn charge(&mut self, n: usize) -> Result<()> {
        self.budget = self
            .budget
            .checked_sub(n)
            .ok_or(Error("decoded checkpoint budget"))?;
        Ok(())
    }
}
pub(crate) trait Wire: Sized {
    fn write(&self, w: &mut Writer) -> Result<()>;
    fn read(r: &mut Reader<'_>) -> Result<Self>;
}
macro_rules! integer { ($($t:ty),*) => { $(impl Wire for $t {
    fn write(&self, w: &mut Writer) -> Result<()> { w.put(&self.to_le_bytes()) }
    fn read(r: &mut Reader<'_>) -> Result<Self> { let a = r.get(size_of::<Self>())?.try_into().map_err(|_| Error("integer"))?; Ok(Self::from_le_bytes(a)) }
})* }; }
integer!(u8, u16, u32, u64, i32);
impl Wire for usize {
    fn write(&self, w: &mut Writer) -> Result<()> {
        (*self as u64).write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        usize::try_from(u64::read(r)?).map_err(|_| Error("integer overflow"))
    }
}
impl Wire for bool {
    fn write(&self, w: &mut Writer) -> Result<()> {
        u8::from(*self).write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match u8::read(r)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error("boolean")),
        }
    }
}
impl Wire for char {
    fn write(&self, w: &mut Writer) -> Result<()> {
        (*self as u32).write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        char::from_u32(u32::read(r)?).ok_or(Error("unicode scalar"))
    }
}
impl<T: Wire> Wire for Option<T> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.is_some().write(w)?;
        if let Some(v) = self {
            v.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        if bool::read(r)? {
            Ok(Some(T::read(r)?))
        } else {
            Ok(None)
        }
    }
}
impl<T: Wire> Wire for Vec<T> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(
            self.len()
                .checked_mul(size_of::<T>().max(1))
                .ok_or(Error("allocation overflow"))?,
        )?;
        self.len().write(w)?;
        for v in self {
            v.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let n = usize::read(r)?;
        r.charge(
            n.checked_mul(size_of::<T>().max(1))
                .ok_or(Error("allocation overflow"))?,
        )?;
        // Every element consumes at least one wire byte. Check before reserve.
        if n > r.bytes.len() {
            return Err(Error("truncated collection"));
        }
        let mut v = Vec::new();
        v.try_reserve_exact(n)
            .map_err(|_| Error("allocation failed"))?;
        for _ in 0..n {
            v.push(T::read(r)?);
        }
        Ok(v)
    }
}
impl<T: Wire> Wire for VecDeque<T> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(
            self.len()
                .checked_mul(size_of::<T>().max(1))
                .and_then(|n| n.checked_mul(2))
                .ok_or(Error("allocation overflow"))?,
        )?;
        self.len().write(w)?;
        for v in self {
            v.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let v = Vec::<T>::read(r)?;
        r.charge(v.len().saturating_mul(size_of::<T>()))?;
        Ok(v.into())
    }
}
impl<T: Wire> Wire for Arc<[T]> {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(
            self.len()
                .checked_mul(size_of::<T>().max(1))
                .and_then(|n| n.checked_mul(2))
                .and_then(|n| n.checked_add(2 * size_of::<usize>()))
                .ok_or(Error("allocation overflow"))?,
        )?;
        self.len().write(w)?;
        for v in self.iter() {
            v.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let v = Vec::<T>::read(r)?;
        r.charge(
            v.len()
                .saturating_mul(size_of::<T>())
                .saturating_add(2 * size_of::<usize>()),
        )?;
        Ok(v.into())
    }
}
impl<T: Wire, const N: usize> Wire for [T; N] {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(N.saturating_mul(size_of::<T>()))?;
        for v in self {
            v.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        r.charge(N.saturating_mul(size_of::<T>()))?;
        let mut v = Vec::with_capacity(N);
        for _ in 0..N {
            v.push(T::read(r)?);
        }
        v.try_into().map_err(|_| Error("array"))
    }
}
impl Wire for String {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(self.len())?;
        self.len().write(w)?;
        w.put(self.as_bytes())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let n = usize::read(r)?;
        r.charge(n)?;
        String::from_utf8(r.get(n)?.to_vec()).map_err(|_| Error("UTF8 string"))
    }
}
impl<A: Wire, B: Wire> Wire for (A, B) {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.0.write(w)?;
        self.1.write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok((A::read(r)?, B::read(r)?))
    }
}
impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn write(&self, w: &mut Writer) -> Result<()> {
        self.0.write(w)?;
        self.1.write(w)?;
        self.2.write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok((A::read(r)?, B::read(r)?, C::read(r)?))
    }
}
// Time origins are process-local. Persist elapsed time at capture, rebase on
// restore, and never include transfer latency in animation/sync timers.
impl Wire for Instant {
    fn write(&self, w: &mut Writer) -> Result<()> {
        let age = w.now.saturating_duration_since(*self);
        age.as_secs().write(w)?;
        age.subsec_nanos().write(w)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let s = u64::read(r)?;
        let n = u32::read(r)?;
        if n >= 1_000_000_000 {
            return Err(Error("duration"));
        }
        r.now
            .checked_sub(Duration::new(s, n))
            .ok_or(Error("time origin"))
    }
}
/// Layout-bit combinations of a [`PackedCell`] (wide trailer, protected).
const LAYOUTS: usize = 4;
/// Where `cell`'s run style sits in [`Writer::ids`]: one entry per grid
/// style and layout combination. `None` for layout bits it does not know.
fn layout_slot(cell: PackedCell) -> Option<usize> {
    let layout = cell.layout_index();
    (layout < LAYOUTS).then(|| cell.style().0 as usize * LAYOUTS + layout)
}
/// Budget bytes per reserved style: the decoder's wire entry plus the grid
/// style it may intern.
const STYLE_COST: usize = size_of::<Cell>() + GRID_STYLE_BYTES;
/// Style-table cells to reserve before defining entry `len`: 16, then
/// doubling. The reader charges and reserves exactly this capacity and the
/// writer charges the same, so the table's spare capacity is inside the
/// budget and both sides stop at the same point.
fn style_growth(len: usize) -> usize {
    match len {
        0 => 16,
        n if n >= 16 && n.is_power_of_two() => n,
        _ => 0,
    }
}
/// A run style: every [`Cell`] field but `ch`, in `Cell`'s v2 field order.
fn write_style(c: &Cell, w: &mut Writer) -> Result<()> {
    c.fg.write(w)?;
    c.bg.write(w)?;
    c.flags.write(w)?;
    c.link.write(w)?;
    c.cluster.write(w)?;
    c.underline_color.write(w)
}
fn read_style(r: &mut Reader<'_>) -> Result<Cell> {
    Ok(Cell {
        ch: '\0',
        fg: Wire::read(r)?,
        bg: Wire::read(r)?,
        flags: Wire::read(r)?,
        link: Wire::read(r)?,
        cluster: Wire::read(r)?,
        underline_color: Wire::read(r)?,
    })
}
macro_rules! fields {
    ($t:ty, [$($field:ident),* $(,)?]) => {
        impl crate::checkpoint::Wire for $t {
            fn write(&self, w: &mut crate::checkpoint::Writer) -> crate::checkpoint::Result<()> { $(crate::checkpoint::Wire::write(&self.$field,w)?;)* Ok(()) }
            fn read(r: &mut crate::checkpoint::Reader<'_>) -> crate::checkpoint::Result<Self> { Ok(Self { $($field: crate::checkpoint::Wire::read(r)?,)* }) }
        }
    };
}
pub(crate) use fields;
macro_rules! variants {
    ($t:ty, [$($v:ident),* $(,)?]) => {
        impl crate::checkpoint::Wire for $t {
            fn write(&self, w: &mut crate::checkpoint::Writer) -> crate::checkpoint::Result<()> { let all = [$(Self::$v),*]; let i = all.iter().position(|v| v == self).ok_or(crate::checkpoint::Error("enum"))?; crate::checkpoint::Wire::write(&(i as u8),w) }
            fn read(r: &mut crate::checkpoint::Reader<'_>) -> crate::checkpoint::Result<Self> { let i = <u8 as crate::checkpoint::Wire>::read(r)? as usize; [$(Self::$v),*].get(i).copied().ok_or(crate::checkpoint::Error("enum")) }
        }
    };
}
pub(crate) use variants;

#[cfg(test)]
mod tests {
    use super::*;

    fn same(grid: &Grid, parser: &AnsiParser) -> (Grid, AnsiParser) {
        decode(&encode(grid, parser).unwrap()).unwrap()
    }

    #[test]
    fn every_partial_sequence_and_durable_state_continues() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"\x1b[31m", b"X\x1b[0mY"),
            (b"\x1b[1;38;2;1", b";2;3mX"),
            (b"\x1b", b"[31mX"),
            (b"\xe2\x82", b"\xacX"),
            (b"\x1b]2;part", b"ial\x07X"),
            (b"\x1b]2;part\x1b", b"\\X"),
            (b"\x1b(", b"0qx\x1b(BX"),
            (b"\x1b#", b"6X"),
            (b"\x1bPq", b"~\x1b\\X"),
            (b"\x1bPq~\x1b", b"\\X"),
            (
                b"\x1b_Ga=t,f=32,s=1,v=1,i=42;",
                b"/wAA/w==\x1b\\\x1b_Ga=p,i=42\x1b\\X",
            ),
            (
                b"\x1b_Ga=t,f=32,s=1,v=1,i=42;/wAA/w==\x1b",
                b"\\\x1b_Ga=p,i=42\x1b\\X",
            ),
            (
                b"\x1b_Ga=t,f=32,s=1,v=1,i=42,m=1;/wAA\x1b\\",
                b"\x1b_Gm=0;/w==\x1b\\\x1b_Ga=p,i=42\x1b\\X",
            ),
            (b"\x1b^sink", b"\x1b\\X"),
            (b"\x1b^sink\x1b", b"\\X"),
            (b"\x1b]4;1;#123456\x07\x1b[31m", b"X\x1b]4;1;?\x07"),
            (b"\x1b]8;;https://example.test\x1b\\", b"X\x1b]8;;\x1b\\Y"),
            (b"\x1b[1\"q", b"X\x1b[?2J"),
            (b"\x1b(0\x1b)0\x0e", b"qx\x0fY"),
            (b"\x1b]2;old\x07\x1b[22t\x1b]2;new\x07", b"\x1b[23t"),
            (b"\x1b[>3u\x1b[>7u", b"\x1b[<u\x1b[?u"),
            (b"A", b"\x1b[3b"),
            (b"\x1b[?1049h\x1b[31mA", b"\x1b[?1049lX"),
            (
                b"\x1b[?2004h\x1b[?1h\x1b[?1003h\x1b=",
                b"X\x1b[?2004$p\x1b[?1$p",
            ),
            (
                b"\x1b[3g\x1b[1;4H\x1bH\x1b[?69h\x1b[2;18s\x1b[3;9r\x1b[?6h\x1b[4h",
                b"X\tY\nZ",
            ),
        ];
        for &(prefix, suffix) in cases {
            let mut grid = Grid::new(20, 12);
            let mut parser = AnsiParser::new();
            parser.advance(&mut grid, prefix);
            parser.take_responses();
            let (mut back, mut continuation) = same(&grid, &parser);
            parser.advance(&mut grid, suffix);
            continuation.advance(&mut back, suffix);
            assert_eq!(
                parser.take_responses(),
                continuation.take_responses(),
                "responses after {prefix:?}"
            );
            assert_eq!(
                encode(&grid, &parser).unwrap(),
                encode(&back, &continuation).unwrap(),
                "state after {prefix:?}"
            );
        }
    }

    #[test]
    fn hidden_cursor_restores_directly_and_answers_decrqm() {
        for hidden in [true, false] {
            let mut grid = Grid::new(20, 12);
            let mut parser = AnsiParser::new();
            parser.advance(&mut grid, if hidden { b"\x1b[?25l" } else { b"\x1b[?25h" });
            assert_eq!(grid.cursor_visible, !hidden);
            let (mut back, mut continuation) = same(&grid, &parser);
            // Direct state and protocol assertions catch symmetric omissions
            // that encode -> decode -> encode equality cannot reveal.
            assert_eq!(back.cursor_visible, !hidden);
            parser.advance(&mut grid, b"\x1b[?25$p");
            continuation.advance(&mut back, b"\x1b[?25$p");
            let expected = if hidden {
                b"\x1b[?25;2$y"
            } else {
                b"\x1b[?25;1$y"
            };
            assert_eq!(parser.take_responses(), expected);
            assert_eq!(continuation.take_responses(), expected);
            continuation.advance(&mut back, b"X\x1b[?25h\x1b[?25$p");
            assert!(back.cursor_visible);
            assert_eq!(continuation.take_responses(), b"\x1b[?25;1$y");
        }
    }

    #[test]
    fn every_grid_field_is_encoded_or_explicitly_excluded() {
        // Inventory is intentionally independent of the wire field list.
        // New Grid fields must get a durable encoding or an audited reason
        // to be outside the headless VT profile.
        let excluded = [
            ("dirty", "rebuilt row damage cache"),
            ("row_stamp", "rebuilt row damage stamps"),
            ("hist_stamp", "rebuilt scrollback damage stamp"),
            ("stamp", "renderer change counter"),
            ("id", "fresh per-grid renderer cache key"),
            ("spare_screen", "recycled blank allocation"),
            ("blank_row", "rebuilt blank-row template"),
            (
                "untrimmed",
                "memory compaction progress; restored lines are all untrimmed",
            ),
            ("styles", "rebuilt from the line run styles"),
            ("epoch", "renderer freshness counter"),
            ("view_offset", "client viewport"),
            ("selection", "client selection"),
            ("search", "GUI search"),
            ("host_out", "pending host side effects"),
            ("links_arc", "rebuilt hyperlink cache"),
            ("clusters_arc", "rebuilt grapheme cache"),
            ("cluster_ids", "rebuilt grapheme id index"),
            ("cluster_scratch", "reused append buffer"),
            ("bell", "pending host bell"),
            ("min_contrast", "renderer configuration"),
            ("bidi", "renderer configuration; VT bdsm state is encoded"),
            ("command_began", "GUI command notification timer"),
            ("finished_commands", "pending GUI notifications"),
            ("clipboard_set", "pending host clipboard action"),
            ("clipboard_set_primary", "pending host clipboard action"),
            ("clipboard_query", "pending host clipboard query"),
            ("clipboard_query_primary", "pending host clipboard query"),
            ("notifications", "pending host notifications"),
            ("collect_program_status", "host configuration"),
            ("program_status", "pending host status signals"),
            ("ime_preedit", "GUI input composition"),
            ("total_scrolled", "GUI overlay image anchor"),
            (
                "images",
                "GUI-only pixel overlays; headless cells/store are encoded",
            ),
            ("fold_pending_start", "GUI fold state"),
            ("fold_blocks", "GUI fold state"),
            ("channel", "L13 application channel"),
            ("command_start", "L13 command capture"),
            ("last_command_output", "L13 command capture"),
        ];
        let source = include_str!("grid.rs");
        let declaration = source
            .split_once("pub struct Grid {")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        let codec = source
            .split_once("impl crate::checkpoint::Wire for Grid")
            .unwrap()
            .1;
        let (write, read) = codec.split_once("    fn read(").unwrap();
        let fields: Vec<_> = declaration
            .lines()
            .filter_map(|line| {
                let line = line
                    .trim()
                    .strip_prefix("pub(crate) ")
                    .or_else(|| line.trim().strip_prefix("pub "))
                    .unwrap_or(line.trim());
                let (name, _) = line.split_once(": ")?;
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    .then_some(name)
            })
            .collect();
        for field in &fields {
            if excluded.iter().any(|(name, _)| name == field) {
                continue;
            }
            assert!(
                write.contains(&format!("&self.{field},")),
                "Grid::{field} missing from encoder"
            );
            assert!(
                read.contains(&format!("grid.{field} = crate::checkpoint::Wire::read")),
                "Grid::{field} missing from decoder"
            );
        }
        for (field, reason) in excluded {
            assert!(
                fields.contains(&field),
                "stale exclusion for {field}: {reason}"
            );
        }
    }

    #[test]
    fn normal_history_and_large_graphics_round_trip() {
        let mut grid = Grid::new(80, 24);
        let mut parser = AnsiParser::new();
        for _ in 0..10_100 {
            parser.advance(&mut grid, b"history\r\n");
        }
        let (back, _) = same(&grid, &parser);
        assert_eq!(grid.encode_snapshot(), back.encode_snapshot());
        grid.kitty_store(42, 2048, 2048, vec![Some(0xabcdef); 2048 * 2048]);
        let bytes = encode(&grid, &parser).unwrap();
        assert!(bytes.len() > 16 << 20, "must exceed a single mux frame");
        let (back, _) = decode(&bytes).unwrap();
        assert_eq!(grid.kitty_frame(42), back.kitty_frame(42));
        assert_eq!(grid.encode_snapshot(), back.encode_snapshot());
    }

    /// The least budget at which `f` succeeds, by bisection.
    fn least_budget(f: impl Fn(usize) -> bool) -> usize {
        let (mut lo, mut hi) = (0, MAX_DECODED);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if f(mid) { hi = mid } else { lo = mid + 1 }
        }
        lo
    }

    #[test]
    fn a_cluster_table_that_encodes_within_a_budget_decodes_within_it() {
        // Decoding rebuilds the cluster index as well as the `Arc` tables, so
        // encode must charge for it too, or a large continuation (here 1 MiB)
        // would encode and then fail to decode under the same budget.
        let mut grid = Grid::new(1, 1);
        let mut parser = AnsiParser::new();
        parser.advance(&mut grid, "e\u{301}".as_bytes());
        grid.clusters.push("\u{301}".repeat(1 << 19));
        grid.rebuild_cluster_caches();
        let mut w = Writer::new(MAX_DECODED, Some(grid.styles()));
        grid.write(&mut w).unwrap();
        let bytes = w.bytes;
        let write_need =
            least_budget(|b| grid.write(&mut Writer::new(b, Some(grid.styles()))).is_ok());
        let read_need = least_budget(|b| Grid::read(&mut Reader::new(&bytes, b)).is_ok());
        assert!(write_need > 1 << 20, "the continuation is charged");
        assert_eq!(read_need, write_need);
    }

    #[test]
    fn random_grids_charge_the_writer_and_reader_alike() {
        // Styles, clusters, wide glyphs, links, DECSCA and history, some
        // rethemed so styles merge: the reader's charge must equal the
        // writer's, or one side stops before the other.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..9 {
            let mut grid = Grid::new(12, 4);
            let mut parser = AnsiParser::new();
            let mut input = Vec::new();
            for _ in 0..400 {
                match next() % 8 {
                    0 => {
                        let (r, b) = (next() % 4 * 64, next() % 4 * 64);
                        input.extend(format!("\x1b[38;2;{r};0;{b}m").bytes());
                    }
                    1 => input.extend(b"\x1b[0m\x1b[1;4m"),
                    2 => input.extend("e\u{301}".bytes()),
                    3 => input.extend("\u{4e2d}".bytes()),
                    4 => input.extend(b"\x1b]8;;http://x/\x07L\x1b]8;;\x07"),
                    5 => input.extend(b"\r\n"),
                    6 => input.extend(b"\x1b[1\"qP\x1b[0\"q"),
                    _ => input.push(b'a' + (next() % 26) as u8),
                }
            }
            parser.advance(&mut grid, &input);
            if case % 3 == 0 {
                let new = crate::color::Theme {
                    fg: 0,
                    bg: 0x40_00_40,
                    ..Default::default()
                };
                let old = parser.retheme(new);
                grid.retheme(&old, &new);
            }
            let mut w = Writer::new(MAX_DECODED, Some(grid.styles()));
            grid.write(&mut w).unwrap();
            let bytes = w.bytes;
            let write_need =
                least_budget(|b| grid.write(&mut Writer::new(b, Some(grid.styles()))).is_ok());
            let read_need = least_budget(|b| Grid::read(&mut Reader::new(&bytes, b)).is_ok());
            assert_eq!(read_need, write_need, "case {case}");
        }
    }

    #[test]
    fn oversized_cluster_tables_fail_to_decode_without_panicking() {
        // Encode refuses these tables (CLUSTER_MAX is 8192), so write the grid
        // directly, past validation, as a hostile checkpoint would.
        for n in [8193, usize::from(u16::MAX) + 1] {
            let mut grid = Grid::new(2, 1);
            grid.clusters = (0..n).map(|i| i.to_string()).collect();
            let mut w = Writer::new(MAX_DECODED, Some(grid.styles()));
            w.put(HEADER).unwrap();
            grid.write(&mut w).unwrap();
            AnsiParser::new().write(&mut w).unwrap();
            assert_eq!(decode(&w.bytes).err(), Some(Error("grid bounds")), "{n}");
        }
    }

    #[test]
    fn retained_state_resizes_and_allocation_budgets_are_symmetric() {
        let mut grid = Grid::new(20, 12);
        let mut parser = AnsiParser::new();
        for _ in 0..100 {
            parser.advance(&mut grid, b"history\r\n");
        }
        parser.advance(&mut grid, b"\x1b[?1049hALT\x1b[");
        let (mut back, mut other) = same(&grid, &parser);
        grid.resize(15, 8);
        back.resize(15, 8);
        parser.advance(&mut grid, b"31mX\x1b[?1049lY");
        other.advance(&mut back, b"31mX\x1b[?1049lY");
        assert_eq!(
            encode(&grid, &parser).unwrap(),
            encode(&back, &other).unwrap()
        );
        let data = encode(&grid, &parser).unwrap();
        let mut writer = Writer::new(1, Some(grid.styles()));
        assert_eq!(
            grid.write(&mut writer),
            Err(Error("decoded checkpoint budget"))
        );
        let mut reader = Reader::new(&data[HEADER.len()..], 1);
        assert!(matches!(
            Grid::read(&mut reader),
            Err(Error("decoded checkpoint budget"))
        ));
    }

    #[test]
    fn malformed_and_budget_inputs_fail_without_exposing_partial_state() {
        let (grid, parser) = (Grid::new(8, 3), AnsiParser::new());
        let good = encode(&grid, &parser).unwrap();
        for len in 0..good.len() {
            assert!(decode(&good[..len]).is_err(), "prefix {len}");
        }
        let mut bad = good.clone();
        bad[4] = 1;
        assert!(decode(&bad).is_err());
        let mut bad = good.clone();
        bad.extend([0]);
        assert!(decode(&bad).is_err());
        // cols, rows, then the screen Vec count: a tiny claim cannot reserve
        // arbitrarily many Line headers before checking the decoded budget.
        let mut bad = good.clone();
        bad[21..29].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&bad).is_err());
        let mut grid_bytes = Writer::new(MAX_DECODED, Some(grid.styles()));
        grid.write(&mut grid_bytes).unwrap();
        let mut bad = good.clone();
        bad[5 + grid_bytes.bytes.len()] = 3; // Utf8, but zero remaining bytes
        assert!(decode(&bad).is_err());
        for i in (5..good.len()).step_by(17) {
            let mut bad = good.clone();
            bad[i] ^= 0xff;
            if let Ok((mut grid, mut parser)) = decode(&bad) {
                parser.advance(&mut grid, b"\t\nX\x1b[2J\x1b[?1049hY\x1b[?1049l");
            }
        }
    }

    use crate::grid::{Line, LineAttr};

    /// A line of `cells`, its styles interned in `table`.
    fn line_in(table: &mut StyleTable, cells: &[Cell], wrapped: bool, attr: LineAttr) -> Line {
        let cells = cells
            .iter()
            .map(|c| PackedCell::new(c, table.intern(CellStyle::of(c))))
            .collect();
        Line::new(cells, wrapped, attr)
    }

    /// `lines` resolved through `table`, to compare lines across tables.
    fn resolved(table: &StyleTable, lines: &[Line]) -> Vec<(Vec<Cell>, bool, LineAttr)> {
        lines
            .iter()
            .map(|l| {
                let cells = l.cells().iter().map(|c| c.resolve(&table.get(c.style())));
                (cells.collect(), l.wrapped, l.attr)
            })
            .collect()
    }

    fn line_bytes(table: &StyleTable, lines: &[Line]) -> Vec<u8> {
        let mut w = Writer::new(MAX_DECODED, Some(table));
        lines.iter().for_each(|l| l.write(&mut w).unwrap());
        w.bytes
    }

    fn read_lines(bytes: &[u8], n: usize) -> Result<(Vec<Line>, StyleTable)> {
        let mut r = Reader::new(bytes, MAX_DECODED);
        let lines = (0..n).map(|_| Line::read(&mut r)).collect::<Result<_>>()?;
        if !r.bytes.is_empty() {
            return Err(Error("trailing"));
        }
        Ok((lines, r.take_styles()))
    }

    #[test]
    fn line_runs_round_trip_every_shape() {
        let red = Cell {
            fg: 0xff0000,
            ..Cell::blank()
        };
        let link = Cell {
            link: 3,
            flags: 1 << 4,
            ..Cell::blank()
        };
        let row = |s: &str, style: Cell| -> Vec<Cell> {
            s.chars().map(|ch| Cell { ch, ..style }).collect()
        };
        let mut mixed = row("abc", red);
        mixed.extend(row("aaaa", red));
        mixed.extend(row("aaa", link));
        mixed.extend(row("x\u{1f600}\u{e9}\0\u{10ffff}", Cell::blank()));
        mixed.extend(row("     ", Cell::blank()));
        mixed.extend(row("abca", red));
        let t = &mut StyleTable::default();
        let single = LineAttr::Single;
        let lines = [
            line_in(t, &[], false, single),
            line_in(t, &[Cell::blank(); 80], false, single),
            line_in(t, &mixed, true, LineAttr::DoubleWidth),
            line_in(t, &row("aaab", red), false, single),
            line_in(t, &row("abbbb", red), false, single),
            line_in(t, &row(&"ab".repeat(2048), link), false, single),
            line_in(t, &[red; 4096], false, single), // the widest row a checkpoint accepts
        ];
        let bytes = line_bytes(t, &lines);
        let (back, table) = read_lines(&bytes, lines.len()).unwrap();
        assert_eq!(resolved(&table, &back), resolved(t, &lines));
        // A blank row is one style, one repeat run and two attribute bytes,
        // and a reused style costs one byte.
        let blank = line_in(t, &[Cell::blank(); 200], false, single);
        assert!(line_bytes(t, &[blank]).len() <= 32);
        let red_row = line_in(t, &[red; 200], false, single);
        let two = line_bytes(t, &[red_row.clone(), red_row.clone()]);
        assert!(two.len() <= line_bytes(t, &[red_row]).len() + 8);
    }

    /// Encoding checks cell style ids per run, not in a pass over every cell
    /// beforehand: a freed or unknown id still fails cleanly, mid-line too.
    #[test]
    fn runs_naming_a_dead_style_fail_to_encode() {
        let mut table = StyleTable::default();
        let red = Cell {
            fg: 1,
            ..Cell::blank()
        };
        let line = line_in(&mut table, &[Cell::blank(), red], false, LineAttr::Single);
        let write = |table: &StyleTable, line: &Line| {
            line.write(&mut Writer::new(MAX_DECODED, Some(table)))
        };
        assert!(write(&table, &line).is_ok());
        let unknown = PackedCell::new(&red, StyleId(table.len() as u32));
        let cells = vec![line.cells()[0], unknown];
        let bad = Line::new(cells, false, LineAttr::Single);
        assert_eq!(write(&table, &bad), Err(Error("cell bounds")));
        table.sweep(std::iter::empty());
        assert_eq!(write(&table, &line), Err(Error("cell bounds")));
    }

    #[test]
    fn hostile_line_runs_fail_cleanly() {
        type Build<'a> = &'a dyn Fn(&mut Writer);
        let define = |w: &mut Writer| w.style(Cell::blank()).unwrap();
        let case = |build: Build| {
            let mut w = Writer::new(MAX_DECODED, None);
            build(&mut w);
            read_lines(&w.bytes, 1)
        };
        let cases: [(&str, Build); 7] = [
            ("line width", &|w| w.varint(4097).unwrap()),
            ("varint overflow", &|w| w.put(&[0xff; 10]).unwrap()),
            ("style index", &|w| {
                w.varint(1).unwrap();
                w.varint(1).unwrap();
            }),
            ("line run length", &|w| {
                w.varint(2).unwrap();
                define(w);
                w.varint(3 << 1 | 1).unwrap();
            }),
            ("line run length", &|w| {
                w.varint(2).unwrap();
                define(w);
                w.varint(0).unwrap();
            }),
            ("UTF8 scalar", &|w| {
                w.varint(1).unwrap();
                define(w);
                w.varint(1 << 1).unwrap();
                w.put(&[0xed, 0xa0, 0x80]).unwrap(); // a surrogate
            }),
            ("truncated checkpoint", &|w| {
                w.varint(4).unwrap();
                define(w);
                w.varint(4 << 1).unwrap();
                w.put(b"ab").unwrap();
            }),
        ];
        for (want, build) in cases {
            assert_eq!(case(build).err(), Some(Error(want)), "{want}");
        }
        // Every prefix and every single-byte flip of a valid row fails or
        // decodes without panicking.
        let t = &mut StyleTable::default();
        let cells: Vec<Cell> = "ab\u{e9}cccc"
            .chars()
            .map(|ch| Cell {
                ch,
                ..Cell::blank()
            })
            .collect();
        let line = line_in(t, &cells, true, LineAttr::Single);
        let good = line_bytes(t, &[line]);
        for len in 0..good.len() {
            assert!(read_lines(&good[..len], 1).is_err(), "prefix {len}");
        }
        for i in 0..good.len() {
            for bit in 0..8 {
                let mut bad = good.clone();
                bad[i] ^= 1 << bit;
                let _ = read_lines(&bad, 1);
            }
        }
    }

    #[test]
    fn style_table_capacity_is_charged_symmetrically() {
        let cell = size_of::<Cell>();
        for n in [1usize, 16, 17, 32, 33, 64, 65] {
            // One row of `n` distinct styles: the row's cells plus the
            // tables' reserved capacity, 16 then doubling, each entry
            // charged for the wire table and the grid table.
            let cells: Vec<Cell> = (0..n)
                .map(|i| Cell {
                    fg: i as u32,
                    ..Cell::blank()
                })
                .collect();
            let t = &mut StyleTable::default();
            let line = line_in(t, &cells, false, LineAttr::Single);
            let capacity = n.next_power_of_two().max(16);
            let need = n * cell + capacity * STYLE_COST;
            let bytes = line_bytes(t, std::slice::from_ref(&line));
            for (budget, ok) in [(need, true), (need - 1, false)] {
                let mut w = Writer::new(budget, Some(t));
                assert_eq!(line.write(&mut w).is_ok(), ok, "write {n} at {budget}");
                let mut r = Reader::new(&bytes, budget);
                assert_eq!(Line::read(&mut r).is_ok(), ok, "read {n} at {budget}");
            }
            let mut r = Reader::new(&bytes, need);
            Line::read(&mut r).unwrap();
            assert_eq!(r.styles.capacity(), capacity, "{n}");
        }
    }

    /// Guards the v3 size win: about 0.64 MB here, against 44 MB for v2 and
    /// 0.65 MB for the RTSN snapshot of the same grid.
    #[test]
    fn coloured_history_checkpoint_stays_compact() {
        let mut grid = Grid::new(200, 50);
        let mut parser = AnsiParser::new();
        for i in 0..12_000 {
            let line = format!(
                "\x1b[3{}m{i:>6} the quick brown fox jumps over the lazy dog {}\x1b[0m\r\n",
                i % 8,
                "x".repeat(i % 60)
            );
            parser.advance(&mut grid, line.as_bytes());
        }
        let bytes = encode(&grid, &parser).unwrap();
        let rtsn = grid.encode_snapshot().len();
        assert!(bytes.len() <= 2 * rtsn, "{} > 2 x {rtsn}", bytes.len());
        let (back, _) = decode(&bytes).unwrap();
        assert_eq!(back.encode_snapshot(), grid.encode_snapshot());
    }
}
