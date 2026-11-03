//! [`Grid::compact_history`] must be invisible: a grid that trims its
//! history reads exactly like one that never does. These tests run the same
//! stream into two grids, compact one of them at random points, and compare
//! everything a reader can see.

use super::{Cell, Grid, Line, LineAttr};
use crate::AnsiParser;
use crate::cell::PackedCell;

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

/// Text, wraps, wide glyphs, erases in a colored background (trailing
/// padding that is not the default blank), default-color changes, hard and
/// soft line ends.
const PIECES: &[&str] = &[
    "abc",
    "hello world ",
    "lorem ipsum dolor sit amet, consectetur ",
    "\u{4e2d}\u{6587}",
    "e\u{301}",
    "\r\n",
    "\r\n",
    "\r\n",
    "\n",
    "\t",
    "\x1b[44m\x1b[K",
    "\x1b[41mred\x1b[K\x1b[0m",
    "\x1b[0m",
    "\x1b[1;32m",
    "\x1b[7m",
    "\x1b]8;;http://x\x07link\x1b]8;;\x07",
    "\x1b]11;#102030\x07",
    "\x1b]11;#000000\x07",
    "\x1b[2K",
    "\x1b[1K",
    "\x1b[5X",
    "\x1b[?7l",
    "\x1b[?7h",
    "\x1b[2S",
    "\x1b[H",
    "\x1b[2J",
];

/// Everything a reader sees of `g`'s history and screen: each physical row
/// (padded and resolved, with its wrap bit), each cell through
/// [`Grid::line_cell`] up to two past the width, each row as
/// [`Grid::push_resolved`] gives it, each line's text, the viewport at a
/// few scroll offsets, and both serialized forms.
type Seen = (
    Vec<(Vec<Cell>, bool)>,
    Vec<Vec<Cell>>,
    Vec<Vec<Cell>>,
    Vec<Option<String>>,
    Vec<Vec<(usize, Vec<Cell>)>>,
    Vec<u8>,
    Vec<u8>,
);

fn seen(g: &mut Grid) -> Seen {
    let total = g.scrollback.len() + g.rows;
    let lines: Vec<&Line> = g.scrollback.iter().chain(&g.screen).collect();
    let phys = (0..total).map(|abs| g.phys_row(abs)).collect();
    let by_cell = lines
        .iter()
        .map(|l| (0..g.cols + 2).map(|x| g.line_cell(l, x)).collect())
        .collect();
    let by_row = lines
        .iter()
        .map(|l| {
            // Appended after a cell already there, as a frame row is.
            let mut out = vec![Cell::blank()];
            g.push_resolved(l, g.cols, &mut out);
            assert_eq!(out.len(), 1 + g.cols, "a row appends exactly `cols` cells");
            out.split_off(1)
        })
        .collect();
    let text = (0..total).map(|abs| g.line_text(abs)).collect();
    let live = g.view_offset;
    let hist = g.scrollback.len();
    let mut views = Vec::new();
    for off in [hist, hist / 2, 1.min(hist), 0] {
        g.view_offset = off;
        views.push(g.snapshot_viewport().rows);
    }
    g.view_offset = live;
    let checkpoint = crate::checkpoint::encode(g, &AnsiParser::new()).expect("checkpoint");
    (
        phys,
        by_cell,
        by_row,
        text,
        views,
        checkpoint,
        g.encode_snapshot(),
    )
}

/// Feed `data` to a plain grid and a compacting one in random chunks; the
/// compacting one trims a random budget after some chunks. Both must read
/// the same after every chunk, through a scrollback cap change and a
/// resize, and compaction must actually have trimmed something.
fn assert_compaction_invisible(data: &[u8], cols: usize, rows: usize, seed: u64) {
    let mut rng = Rng(seed);
    let (mut plain, mut packed) = (Grid::new(cols, rows), Grid::new(cols, rows));
    let (mut pp, mut kp) = (AnsiParser::new(), AnsiParser::new());
    for g in [&mut plain, &mut packed] {
        g.set_scrollback_max(60);
    }
    let mut trimmed = 0usize;
    let mut at = 0;
    let mut step = 0;
    while at < data.len() {
        let end = (at + 1 + rng.below(256)).min(data.len());
        pp.advance(&mut plain, &data[at..end]);
        kp.advance(&mut packed, &data[at..end]);
        if rng.below(3) == 0 {
            packed.compact_history(rng.below(40));
            trimmed += packed
                .scrollback
                .iter()
                .filter(|l| l.width() > l.cells().len())
                .count();
        }
        step += 1;
        match step {
            40 => {
                plain.set_scrollback_max(25);
                packed.set_scrollback_max(25);
            }
            80 => {
                plain.resize(cols + 7, rows + 2);
                packed.resize(cols + 7, rows + 2);
            }
            _ => {}
        }
        assert!(
            seen(&mut plain) == seen(&mut packed),
            "reads differ after byte {end}"
        );
        at = end;
    }
    assert!(trimmed > 0, "nothing was ever trimmed");
}

#[test]
fn compacted_history_reads_like_untouched_history() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut data = String::new();
    for _ in 0..12_000 {
        data.push_str(PIECES[rng.below(PIECES.len())]);
    }
    for (cols, rows) in [(20, 6), (7, 3), (80, 24)] {
        assert_compaction_invisible(data.as_bytes(), cols, rows, rng.next());
    }
}

#[test]
fn compaction_trims_only_hard_broken_padding_and_reports_progress() {
    let mut g = Grid::new(10, 2);
    let mut p = AnsiParser::new();
    // "ab" ends hard; "0123456789xy" soft-wraps after its tenth cell; the
    // erase leaves a blue run; the empty line is all padding.
    p.advance(
        &mut g,
        b"ab\r\n0123456789xy\r\n\x1b[44mc\x1b[K\x1b[0m\r\n\r\nz\r\n",
    );
    assert_eq!(g.scrollback.len(), 5);
    assert!(g.compact_history(2), "three lines still to look at");
    assert!(!g.compact_history(10));
    assert!(!g.compact_history(10), "nothing left after a full pass");
    let stored: Vec<usize> = g.scrollback.iter().map(|l| l.cells().len()).collect();
    assert_eq!(
        stored,
        [3, 10, 3, 2, 1],
        "wrapped row untouched, each run kept as one cell"
    );
    assert!(g.scrollback.iter().all(|l| l.width() == 10));
    let blue = g.line_cell(&g.scrollback[3], 9);
    assert_eq!(blue.ch, ' ');
    assert_ne!(blue.bg, Cell::blank().bg, "the run keeps the erase color");
}

#[test]
fn a_style_sweep_keeps_the_style_only_a_trimmed_run_uses() {
    // After "x", the erase color fills the rest of the row and nothing else
    // uses it, so once trimmed one stored cell holds that style; a sweep must
    // not free it for reuse.
    let mut g = Grid::new(8, 2);
    let mut p = AnsiParser::new();
    p.advance(&mut g, b"x\x1b[48;2;1;2;3m\x1b[K\x1b[0m\r\n\r\n\r\n");
    g.compact_history(usize::MAX);
    let want = g.line_cell(&g.scrollback[0], 7);
    g.collect_styles();
    // Enough new styles to take every slot a sweep could have freed.
    let fresh: String = (0..32).map(|i| format!("\x1b[48;2;9;9;{i}mn")).collect();
    p.advance(&mut g, fresh.as_bytes());
    assert_eq!(g.line_cell(&g.scrollback[0], 7), want);
}

#[test]
fn a_trim_keeps_one_cell_of_the_run_and_a_write_restores_the_rest() {
    let fill = PackedCell::blank();
    let mut line = Line::new(vec![fill; 6], false, LineAttr::Single);
    line.trim_tail(2);
    assert_eq!((line.cells().len(), line.width()), (3, 6));
    assert_eq!(line.cell(5), Some(fill));
    assert_eq!(line.cell(6), None);
    assert_eq!(line.cells_restyle().len(), 3, "a remap keeps the trim");
    assert_eq!(line.cells_mut().len(), 6, "a write stores the run again");
    assert_eq!(line.width(), 6);
}

#[test]
fn a_line_stays_small() {
    // A scroll moves every screen line; trimming must not grow them.
    assert!(
        size_of::<Line>() <= 32,
        "Line is {} bytes",
        size_of::<Line>()
    );
}

/// Cell memory a full history holds before and after compaction, for a
/// shell-like stream of short lines and for full-width text. Run with
/// `cargo test -p rusty_term_core --release measure_history_memory -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn measure_history_memory() {
    const LINES: usize = 10_000;
    let bytes = |g: &Grid| -> usize {
        let cells: usize = g.scrollback.iter().map(|l| l.cells().len()).sum();
        cells * size_of::<PackedCell>()
    };
    let short: String = (0..LINES)
        .map(|i| format!("$ ls -l file{i}.txt\r\n"))
        .collect();
    let full: String = (0..LINES).map(|i| format!("{:0>200}\r\n", i)).collect();
    for (name, data) in [("short lines", &short), ("full width", &full)] {
        let mut g = Grid::new(200, 50);
        g.set_scrollback_max(LINES);
        AnsiParser::new().advance(&mut g, data.as_bytes());
        let before = bytes(&g);
        while g.compact_history(1000) {}
        let after = bytes(&g);
        println!(
            "{name}: {:.1} MB -> {:.1} MB",
            before as f64 / 1e6,
            after as f64 / 1e6
        );
    }
}
