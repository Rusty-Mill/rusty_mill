//! Memory and pause bounds of the 8-byte cell and its style table. In its
//! own test binary so the global allocator counts only these tests; they run
//! one at a time under a lock.
//!
//! The `#[ignore]` measurements are for release builds:
//! `cargo test --release -p rusty_term_core --test memory -- --ignored --nocapture`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rusty_term_core::{AnsiParser, Grid};

struct Counting;

/// Heap bytes currently allocated.
static HEAP: AtomicUsize = AtomicUsize::new(0);
/// Serializes the tests, which share [`HEAP`].
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

// SAFETY: every call forwards to `System` with the caller's own pointer and
// layout, and only counts bytes around it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP.fetch_add(layout.size(), Ordering::SeqCst);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HEAP.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: `ptr` came from `alloc` above with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MB: usize = 1 << 20;

/// A grid of `cols` x `rows` with `history` lines of scrollback, and the
/// heap it took to build it from `output`, fed in 4 KiB chunks.
fn build(cols: usize, rows: usize, history: usize, output: &[u8]) -> (Grid, usize) {
    let before = HEAP.load(Ordering::SeqCst);
    let mut g = Grid::new(cols, rows);
    g.set_scrollback_max(history);
    let mut p = AnsiParser::new();
    for chunk in output.chunks(4096) {
        p.advance(&mut g, chunk);
    }
    (g, HEAP.load(Ordering::SeqCst).saturating_sub(before))
}

/// Ordinary coloured output: eight colours, one per line.
fn eight_colours(lines: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..lines {
        let pad = "x".repeat(i % 60);
        out += &format!(
            "\x1b[3{}m{i:>6} the quick brown fox {pad}\x1b[0m\r\n",
            i % 8
        );
    }
    out.into_bytes()
}

/// `cells` glyphs, each in its own truecolour, wrapping at the margin.
fn rainbow(first: u32, cells: usize) -> Vec<u8> {
    let mut out = String::new();
    for c in first..first + cells as u32 {
        let (r, g, b) = (c >> 16 & 0xff, c >> 8 & 0xff, c & 0xff);
        out += &format!("\x1b[38;2;{r};{g};{b}mx");
    }
    out.into_bytes()
}

#[test]
fn ordinary_output_is_eight_bytes_a_cell_and_few_styles() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let (g, heap) = build(200, 50, 10_000, &eight_colours(12_000));
    let stats = g.style_stats();
    assert_eq!(stats.cell_bytes, 8);
    assert!(stats.live < 64, "{stats:?}");
    // 2.01M cells of 8 bytes is 16.1 MB; 47.7 MB before the change.
    assert!(heap <= 20 * MB, "{heap} bytes");
}

#[test]
fn a_style_per_cell_is_bounded_and_released() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let (cols, rows, history) = (80, 24, 1000);
    let cells = cols * (rows + history);
    let (mut g, heap) = build(cols, rows, history, &rainbow(1, cells));
    // Every cell kept its own colour, and the whole grid stays inside the
    // accepted worst case of about 170 bytes a cell (Q1).
    assert!(g.style_stats().live > cells - cols, "{:?}", g.style_stats());
    assert!(heap <= 170 * cells, "{} bytes a cell", heap / cells);
    let mut p = AnsiParser::new();
    p.advance(&mut g, b"\x1b[3J");
    let stats = g.style_stats();
    assert!(
        stats.live <= cols * rows + 64,
        "screen styles only: {stats:?}"
    );
    p.advance(&mut g, b"\x1b[0m\x1b[2J");
    let stats = g.style_stats();
    assert!(stats.live < 4096 && stats.bytes < MB, "{stats:?}");
}

/// Resident set size in bytes, from `/proc` (Linux only).
fn rss() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kb = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches(" kB").parse::<usize>().ok());
    kb.unwrap_or(0) * 1024
}

#[test]
#[ignore = "measurement; run in release"]
fn measure_rss() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for (cols, rows, history) in [(80, 24, 0), (200, 50, 10_000)] {
        let output = eight_colours(history + rows);
        let before = rss();
        let (g, heap) = build(cols, rows, history, &output);
        let grown = rss().saturating_sub(before);
        let stats = g.style_stats();
        println!(
            "{cols}x{rows}+{history}: heap {:.1} MB, rss +{:.1} MB, {stats:?}",
            heap as f64 / MB as f64,
            grown as f64 / MB as f64
        );
    }
}

/// The longest and the median time of one `advance` over `output`.
fn pauses(p: &mut AnsiParser, g: &mut Grid, output: &[u8]) -> (Duration, Duration) {
    let mut times: Vec<Duration> = output
        .chunks(4096)
        .map(|chunk| {
            let start = Instant::now();
            p.advance(g, chunk);
            start.elapsed()
        })
        .collect();
    times.sort();
    (times[times.len() - 1], times[times.len() / 2])
}

/// The worst sweep and compaction pauses at `cols` x (`rows` + `history`),
/// each cell its own style: (fill, turnover, compaction, median chunk).
fn rainbow_pauses(cols: usize, rows: usize, history: usize) -> [Duration; 4] {
    let cells = cols * (rows + history);
    let mut g = Grid::new(cols, rows);
    g.set_scrollback_max(history);
    let mut p = AnsiParser::new();
    let (fill, _) = pauses(&mut p, &mut g, &rainbow(1, cells));
    // Another full grid of new styles: sweeps free the evicted ones.
    let (turnover, median) = pauses(&mut p, &mut g, &rainbow(1 << 23, cells));
    // Plain text evicts every style: the table empties and compacts.
    let plain = "\x1b[0m".to_string() + &"y".repeat(cells);
    let (compact, _) = pauses(&mut p, &mut g, plain.as_bytes());
    [fill, turnover, compact, median]
}

/// Reports only. With a style per cell, the worst pause comes from the
/// style index growing (a rehash), not from a sweep or compaction; the owner
/// accepted these adversarial-only pauses on 2026-10-25 (G6, alongside Q1).
#[test]
#[ignore = "measurement; run in release"]
fn measure_sweep_and_compaction_pauses() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // The last is 4.19M cells, the mux limit.
    let sizes = [
        (80, 24, 1000),
        (200, 50, 1000),
        (200, 50, 10_000),
        (417, 50, 10_000),
    ];
    for (cols, rows, history) in sizes {
        let [fill, turnover, compact, median] = rainbow_pauses(cols, rows, history);
        println!(
            "{cols}x{rows}+{history}: fill {fill:?}, turnover {turnover:?}, \
             compaction {compact:?}, median chunk {median:?}"
        );
    }
}

#[test]
#[ignore = "measurement; run in release"]
fn measure_checkpoint_encode() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut g = Grid::new(200, 50);
    let mut p = AnsiParser::new();
    for chunk in eight_colours(12_000).chunks(4096) {
        p.advance(&mut g, chunk);
    }
    let mut times: Vec<Duration> = (0..9)
        .map(|_| {
            let start = Instant::now();
            let bytes = rusty_term_core::checkpoint::encode(&g, &p);
            let elapsed = start.elapsed();
            assert!(bytes.is_ok(), "encode failed");
            elapsed
        })
        .collect();
    times.sort();
    println!(
        "RTCP encode 200x50+10k: median {:?}, min {:?}",
        times[4], times[0]
    );
}
