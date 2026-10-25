//! Row stamps are only useful if no change slips past them: a renderer or a
//! mux client that skips a row whose stamp did not move would show stale
//! content. These tests replay real captures and a generated stream in small
//! chunks and check, after every chunk, that an unchanged stamp means an
//! unchanged row (and an unchanged history stamp an unchanged scrollback).

use super::{Cell, Grid, LineAttr};
use crate::AnsiParser;

/// What one live row shows, for comparison across chunks.
type RowState = (u64, Vec<Cell>, LineAttr, bool);

fn rows(g: &Grid) -> Vec<RowState> {
    g.screen
        .iter()
        .zip(&g.row_stamp)
        .map(|(l, &s)| (s, l.cells.clone(), l.attr, l.wrapped))
        .collect()
}

fn history(g: &Grid) -> (u64, Vec<Vec<Cell>>) {
    let lines = g.scrollback.iter().map(|l| l.cells.clone()).collect();
    (g.hist_stamp, lines)
}

/// Deterministic xorshift, so a failure replays.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Feed `data` in random chunks of 1..=64 bytes; after each, every row or
/// history whose stamp held still must be unchanged.
fn assert_stamps_track(data: &[u8], cols: usize, rows_n: usize, seed: u64) {
    let mut g = Grid::new(cols, rows_n);
    g.set_scrollback_max(40);
    let mut p = AnsiParser::new();
    let mut rng = Rng(seed);
    let (mut before, mut hist) = (rows(&g), history(&g));
    let mut at = 0;
    while at < data.len() {
        let end = (at + 1 + rng.below(64)).min(data.len());
        p.advance(&mut g, &data[at..end]);
        let (now, h) = (rows(&g), history(&g));
        if now.len() == before.len() {
            for (y, (b, n)) in before.iter().zip(&now).enumerate() {
                assert!(
                    b.0 != n.0 || b == n,
                    "row {y} changed under an unchanged stamp at byte {at}..{end}: {:?}\nbefore {:?}\nafter  {:?}",
                    String::from_utf8_lossy(&data[at.saturating_sub(16)..end]),
                    (b.2, b.3, b.1.iter().zip(&n.1).position(|(x, y)| x != y)),
                    (n.2, n.3, b.1.iter().zip(&n.1).find(|(x, y)| x != y))
                );
            }
        }
        assert!(
            hist.0 != h.0 || hist == h,
            "scrollback changed under an unchanged stamp at byte {at}..{end}"
        );
        (before, hist, at) = (now, h, end);
    }
}

#[test]
fn stamps_track_every_row_change_in_captures() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fidelity/corpus");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("corpus entry").path();
        if path.extension().is_some_and(|e| e == "vt") {
            let data = std::fs::read(&path).expect("capture");
            assert_stamps_track(&data, 80, 24, 0x9E37_79B9 + n);
            n += 1;
        }
    }
    assert!(n > 0, "no captures in {dir}");
}

/// Escape sequences that move, erase, scroll, insert or restyle, mixed with
/// text, wide and combining characters.
const PIECES: &[&str] = &[
    "\x1b[H",
    "\x1b[5;7H",
    "\x1b[2J",
    "\x1b[K",
    "\x1b[1K",
    "\x1b[2K",
    "\x1b[J",
    "\x1b[1J",
    "\x1b[3L",
    "\x1b[2M",
    "\x1b[4@",
    "\x1b[3P",
    "\x1b[5X",
    "\x1b[2S",
    "\x1b[2T",
    "\x1b[3;8r",
    "\x1b[r",
    "\x1bD",
    "\x1bM",
    "\x1bE",
    "\x1b7",
    "\x1b8",
    "\x1b[?1049h",
    "\x1b[?1049l",
    "\x1b[?7l",
    "\x1b[?7h",
    "\x1b#8",
    "\x1b#6",
    "\x1b#3",
    "\x1b[1;31m",
    "\x1b[0m",
    "\x1b[7m",
    "\x1b[38;5;200m",
    "\x1b[3b",
    "\x1b[?6h",
    "\x1b[?6l",
    "\x1b]8;;http://x\x07",
    "\x1b]8;;\x07",
    "\r\n",
    "\n",
    "\r",
    "\t",
    "\x08",
    "abc",
    "hello world ",
    "\u{4e2d}\u{6587}",
    "e\u{301}",
    "\u{5d0}\u{5d1}",
    "\x1b[?25l",
    "\x1b[?25h",
    "\x1b[2 q",
    "\x1bc",
    "\x1b[?69h\x1b[3;12s",
    "\x1b[?69l",
    "\x1b[2;4;3;9$z",
    "\x1b[1;1;2;5;3;3$v",
    "\x1b[3 @",
    "\x1b[2 A",
];

#[test]
fn stamps_track_every_row_change_in_a_generated_stream() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut data = String::new();
    for _ in 0..40_000 {
        data.push_str(PIECES[rng.below(PIECES.len())]);
    }
    for (cols, rows) in [(20, 6), (7, 3), (80, 24)] {
        assert_stamps_track(data.as_bytes(), cols, rows, rng.next());
    }
}

#[test]
fn stamps_differ_across_grids_and_increase() {
    let (mut a, b) = (Grid::new(4, 2), Grid::new(4, 2));
    assert_ne!(
        a.row_stamp(0),
        b.row_stamp(0),
        "a new grid never matches another"
    );
    let s0 = a.row_stamp(1);
    AnsiParser::new().advance(&mut a, b"\x1b[2;1Hx");
    let s1 = a.row_stamp(1);
    assert!(s1.1 > s0.1);
    assert_eq!(a.row_stamp(0).1, 0, "an untouched row keeps its stamp");
    a.clear_dirty();
    assert_eq!(
        a.row_stamp(1),
        s1,
        "clearing dirty flags leaves stamps alone"
    );
}
