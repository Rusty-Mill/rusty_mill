//! RTCP v2: a headless terminal continuation checkpoint, independent of RTSN.
//! Durable parser/Grid state is included; front-end view state and pending
//! host side effects are excluded. L13 channels are outside this profile.

use crate::{AnsiParser, Grid};
use std::{
    collections::VecDeque,
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

/// Encode a consistent headless Grid and parser. No output log is retained.
pub fn encode(grid: &Grid, parser: &AnsiParser) -> Result<Vec<u8>> {
    grid.validate_checkpoint()?;
    parser.validate_checkpoint()?;
    let mut w = Writer {
        bytes: b"RTCP\x02".to_vec(),
        now: Instant::now(),
        budget: MAX_DECODED,
    };
    grid.write(&mut w)?;
    parser.write(&mut w)?;
    Ok(w.bytes)
}

/// Decode untrusted RTCP v2, validating allocation budgets and internal indices
/// before returning objects that can safely continue parsing. No host actions
/// or terminal responses are replayed while restoring.
pub fn decode(data: &[u8]) -> Result<(Grid, AnsiParser)> {
    if data.len() > MAX_ENCODED || !data.starts_with(b"RTCP\x02") {
        return Err(Error("checkpoint header/version/size"));
    }
    let mut r = Reader {
        bytes: &data[5..],
        budget: MAX_DECODED,
        now: Instant::now(),
    };
    let grid = Grid::read(&mut r)?;
    let parser = AnsiParser::read(&mut r)?;
    grid.validate_checkpoint()?;
    parser.validate_checkpoint()?;
    if !r.bytes.is_empty() {
        return Err(Error("trailing checkpoint data"));
    }
    Ok((grid, parser))
}

pub(crate) struct Writer {
    bytes: Vec<u8>,
    now: Instant,
    budget: usize,
}
impl Writer {
    pub(crate) fn charge(&mut self, n: usize) -> Result<()> {
        self.budget = self
            .budget
            .checked_sub(n)
            .ok_or(Error("decoded checkpoint budget"))?;
        Ok(())
    }
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_ENCODED.saturating_sub(self.bytes.len()) {
            return Err(Error("encoded checkpoint budget"));
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = needed
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_ENCODED);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| Error("checkpoint allocation failed"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    budget: usize,
    now: Instant,
}
impl Reader<'_> {
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
            ("epoch", "renderer freshness counter"),
            ("view_offset", "client viewport"),
            ("selection", "client selection"),
            ("search", "GUI search"),
            ("host_out", "pending host side effects"),
            ("links_arc", "rebuilt hyperlink cache"),
            ("clusters_arc", "rebuilt grapheme cache"),
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
        let mut writer = Writer {
            bytes: Vec::new(),
            now: Instant::now(),
            budget: 1,
        };
        assert_eq!(
            grid.write(&mut writer),
            Err(Error("decoded checkpoint budget"))
        );
        let mut reader = Reader {
            bytes: &data[5..],
            now: Instant::now(),
            budget: 1,
        };
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
        let mut grid_bytes = Writer {
            bytes: Vec::new(),
            now: Instant::now(),
            budget: MAX_DECODED,
        };
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
}
