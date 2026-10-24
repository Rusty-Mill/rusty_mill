//! Frame-time benchmark for the windowed front-end's CPU frame path.
//!
//! Drives a [`Grid`] and the real CPU compositor offscreen
//! (`rusty_term::gui::bench::FrameBench`, no window or present) and reports
//! frame time and grid-lock hold time. Two modes:
//!
//! - **Streaming.** A parser thread feeds a `bench/workloads` file into a
//!   shared `Mutex<Grid>` in 4 KiB chunks, locking per chunk as the PTY
//!   reader does, looping the file for at least `--secs`. A render thread
//!   paints at `--hz` as the window does: lock, snapshot the frame
//!   ([`Grid::frame_view`]), unlock, paint. Reported: frame ms, lock hold ms
//!   (the parser is blocked for it), the render thread's wait for the lock,
//!   and parser MB/s with rendering against parsing alone.
//! - **Static folds.** Frame ms for the same text with 0 and with 1,024
//!   command fold blocks (OSC 133 `C`..`D` around each command's output), at
//!   the live view and scrolled back half a screen into history. Every cell
//!   drawn maps its view row through the fold blocks.
//! - **Typing.** Frame ms when each frame adds one character at the prompt
//!   of a full screen.
//!
//! The renderer repaints only the rows that changed since the last frame, as
//! the window does on a backend that keeps its buffer; the `full repaint`
//! figures repaint every row, as before damage tracking.
//!
//! A run that cannot measure (no font, a renderer that fails to start or
//! panics, no samples) exits non-zero instead of printing zeros.
//!
//! Usage:
//!   python3 bench/gen_workloads.py --size quick
//!   cargo run --release --features gui --bin bench_frame_time
//!   cargo run --release --features gui --bin bench_frame_time -- --hz 120 --secs 3 bench/workloads/scroll_heavy.vt
//!
//! The font is the window's default (`$RUSTY_TERM_FONT` or a system
//! monospace font).

use std::env;
use std::fs;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rusty_term::core::{AnsiParser, FrameView, Grid};
use rusty_term::gui::bench::FrameBench;
use rusty_term::sync::Mutex;

const CHUNK: usize = 4096;
const COLS: usize = 120;
const ROWS: usize = 40;
const FONT_PX: f32 = 16.0;
const FOLD_BLOCKS: usize = 1024;
const STATIC_FRAMES: usize = 60;
const DEFAULT_WORKLOAD_DIR: &str = "bench/workloads";

struct Options {
    hz: f64,
    secs: f64,
    workloads: Vec<String>,
}

/// Something that paints one frame snapshot: the real CPU frame path, or a
/// stand-in in tests.
trait Painter {
    fn paint(&mut self, frame: &FrameView);
}

impl Painter for FrameBench {
    fn paint(&mut self, grid: &FrameView) {
        self.draw(grid);
    }
}

fn frame_bench() -> Option<FrameBench> {
    FrameBench::new(COLS, ROWS, FONT_PX)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bench_frame_time: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let opts = parse_args(env::args().skip(1).collect()).map_err(|e| {
        format!("{e}\nusage: bench_frame_time [--hz N] [--secs N] [WORKLOAD.vt...]")
    })?;
    if frame_bench().is_none() {
        return Err("no usable monospace font: set RUSTY_TERM_FONT".to_owned());
    }
    println!(
        "## Streaming: {COLS}x{ROWS}, {CHUNK} B chunks, render at {} Hz, >= {} s per workload\n",
        opts.hz, opts.secs
    );
    println!(
        "| workload | frames | frame p50 | frame p99 | lock hold p50 | lock hold p99 | lock hold max | render wait p99 | parse MB/s alone | parse MB/s rendering |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for path in &opts.workloads {
        let data = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        if data.is_empty() {
            return Err(format!("{path}: empty"));
        }
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".vt");
        let alone = parse_alone(&data, opts.secs);
        let s = streaming(&data, &opts, frame_bench).map_err(|e| format!("{name}: {e}"))?;
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {alone:.1} | {:.1} |",
            s.frame.len(),
            pct_ms(&s.frame, 50.0)?,
            pct_ms(&s.frame, 99.0)?,
            pct_ms(&s.hold, 50.0)?,
            pct_ms(&s.hold, 99.0)?,
            pct_ms(&s.hold, 100.0)?,
            pct_ms(&s.wait, 99.0)?,
            s.mb_per_s,
        );
    }
    println!("\n## Static folds: {COLS}x{ROWS}, {STATIC_FRAMES} frames each\n");
    println!("| fold blocks | view | frame p50 | frame p99 | full repaint p50 |");
    println!("|---:|---|---:|---:|---:|");
    for folds in [false, true] {
        for scrolled in [false, true] {
            let grid = fold_grid(folds, scrolled)?;
            let mut fb = frame_bench().ok_or("renderer failed to start")?;
            let frames = static_frames(&mut fb, &grid);
            let mut fb = frame_bench()
                .ok_or("renderer failed to start")?
                .full_repaint();
            let full = static_frames(&mut fb, &grid);
            let view = if scrolled { "scrolled back" } else { "live" };
            println!(
                "| {} | {view} | {} | {} | {} |",
                grid.fold_blocks().len(),
                pct_ms(&frames, 50.0)?,
                pct_ms(&frames, 99.0)?,
                pct_ms(&full, 50.0)?
            );
        }
    }
    println!("\n## Typing: {COLS}x{ROWS}, one character per frame, {STATIC_FRAMES} frames\n");
    println!("| repaint | frame p50 | frame p99 |");
    println!("|---|---:|---:|");
    for full in [false, true] {
        let fb = frame_bench().ok_or("renderer failed to start")?;
        let mut fb = if full { fb.full_repaint() } else { fb };
        let frames = typing_frames(&mut fb, &mut fold_grid(false, false)?);
        let repaint = if full { "full" } else { "changed rows" };
        println!(
            "| {repaint} | {} | {} |",
            pct_ms(&frames, 50.0)?,
            pct_ms(&frames, 99.0)?
        );
    }
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut opts = Options {
        hz: 60.0,
        secs: 2.0,
        workloads: Vec::new(),
    };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--hz" | "--secs" => {
                let v: f64 = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|v: &f64| *v > 0.0 && v.is_finite())
                    .ok_or(format!("{a} needs a positive number"))?;
                if a == "--hz" {
                    opts.hz = v;
                } else {
                    opts.secs = v;
                }
            }
            _ => opts.workloads.push(a),
        }
    }
    if opts.workloads.is_empty() {
        let mut found: Vec<String> = fs::read_dir(DEFAULT_WORKLOAD_DIR)
            .map_err(|e| format!("{DEFAULT_WORKLOAD_DIR}: {e} (run bench/gen_workloads.py)"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "vt"))
            .map(|p| p.display().to_string())
            .collect();
        found.sort();
        opts.workloads = found;
    }
    Ok(opts)
}

/// Feed `data` (looped) through a parser into `grid` in [`CHUNK`]-byte
/// pieces, locking per chunk like the PTY reader, until `secs` have passed
/// and at least one whole pass is done. Returns MB/s.
fn feed(grid: &Mutex<Grid>, data: &[u8], secs: f64) -> f64 {
    let mut parser = AnsiParser::new();
    let start = Instant::now();
    let mut bytes = 0usize;
    loop {
        for chunk in data.chunks(CHUNK) {
            let mut g = grid.lock();
            parser.advance(&mut g, chunk);
            let _ = g.take_host_out();
            drop(g);
            let _ = parser.take_responses();
        }
        bytes += data.len();
        if start.elapsed().as_secs_f64() >= secs {
            break;
        }
    }
    bytes as f64 / 1e6 / start.elapsed().as_secs_f64()
}

fn parse_alone(data: &[u8], secs: f64) -> f64 {
    feed(&Mutex::new(Grid::new(COLS, ROWS)), data, secs)
}

#[derive(Debug)]
struct Streamed {
    frame: Vec<Duration>,
    hold: Vec<Duration>,
    wait: Vec<Duration>,
    mb_per_s: f64,
}

/// Run the parser against a render thread built by `make`. The render
/// thread reports readiness before the parser starts, so timing covers
/// rendering from the first chunk. A renderer that fails to start, panics or
/// records no frame is an error.
fn streaming<P: Painter>(
    data: &[u8],
    opts: &Options,
    make: impl FnOnce() -> Option<P> + Send + 'static,
) -> Result<Streamed, String> {
    let grid = Arc::new(Mutex::new(Grid::new(COLS, ROWS)));
    let done = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel();
    let render = {
        let (grid, done) = (Arc::clone(&grid), Arc::clone(&done));
        let period = Duration::from_secs_f64(1.0 / opts.hz);
        thread::spawn(move || {
            let Some(mut painter) = make() else {
                let _ = ready_tx.send(false);
                return FrameTimes::default();
            };
            let _ = ready_tx.send(true);
            render_loop(&mut painter, &grid, &done, period)
        })
    };
    // A renderer that panics before reporting drops the sender: also a failure.
    let started = ready_rx.recv().unwrap_or(false);
    let mb_per_s = if started {
        feed(&grid, data, opts.secs)
    } else {
        0.0
    };
    done.store(true, Ordering::Relaxed);
    let (frame, hold, wait) = render
        .join()
        .map_err(|_| "render thread panicked".to_owned())?;
    if !started {
        return Err("renderer failed to start".to_owned());
    }
    if frame.is_empty() {
        return Err("no frames recorded".to_owned());
    }
    Ok(Streamed {
        frame,
        hold,
        wait,
        mb_per_s,
    })
}

type FrameTimes = (Vec<Duration>, Vec<Duration>, Vec<Duration>);

/// Paint every `period` until `done`, as the window does: take the grid
/// lock, snapshot the frame, release, paint. Always paints at least once.
/// Returns per-frame (frame, lock hold, lock wait) times.
fn render_loop(
    painter: &mut impl Painter,
    grid: &Mutex<Grid>,
    done: &AtomicBool,
    period: Duration,
) -> FrameTimes {
    let mut times: FrameTimes = Default::default();
    let mut next = Instant::now();
    loop {
        let t0 = Instant::now();
        let g = grid.lock();
        let t1 = Instant::now();
        let frame = g.frame_view();
        drop(g);
        let t2 = Instant::now();
        painter.paint(&frame);
        times.0.push(t0.elapsed());
        times.1.push(t2 - t1);
        times.2.push(t1 - t0);
        if done.load(Ordering::Relaxed) {
            return times;
        }
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now; // over budget: do not try to catch up
        }
    }
}

/// A full grid with the same text either way: [`FOLD_BLOCKS`] commands, each
/// a prompt line and an output line. With `folds`, OSC 133 marks each
/// command's output, making it a fold block. With `scrolled`, the view is
/// scrolled back half a screen. Errors if the grid does not end up so.
fn fold_grid(folds: bool, scrolled: bool) -> Result<Grid, String> {
    let mut stream = Vec::new();
    for i in 0..FOLD_BLOCKS {
        let (a, c, d): (&[u8], &[u8], &[u8]) = if folds {
            (b"\x1b]133;A\x07", b"\x1b]133;C\x07", b"\x1b]133;D;0\x07")
        } else {
            (b"", b"", b"")
        };
        stream.extend_from_slice(a);
        stream.extend_from_slice(b"$ make\r\n");
        stream.extend_from_slice(c);
        stream.extend_from_slice(format!("output line {i}\r\n").as_bytes());
        stream.extend_from_slice(d);
    }
    let mut g = Grid::new(COLS, ROWS);
    AnsiParser::new().advance(&mut g, &stream);
    let want_blocks = if folds { FOLD_BLOCKS } else { 0 };
    if g.fold_blocks().len() != want_blocks {
        return Err(format!(
            "fold fixture: {} blocks, expected {want_blocks}",
            g.fold_blocks().len()
        ));
    }
    if scrolled {
        g.scroll_view_up(ROWS / 2);
        if g.view_offset != ROWS / 2 {
            return Err(format!(
                "fold fixture: view offset {}, expected {}",
                g.view_offset,
                ROWS / 2
            ));
        }
    }
    Ok(g)
}

/// [`STATIC_FRAMES`] timed frames of `grid` (snapshot and paint), after one
/// untimed frame that warms the glyph cache.
fn static_frames(painter: &mut impl Painter, grid: &Grid) -> Vec<Duration> {
    painter.paint(&grid.frame_view());
    (0..STATIC_FRAMES)
        .map(|_| {
            let t = Instant::now();
            painter.paint(&grid.frame_view());
            t.elapsed()
        })
        .collect()
}

/// [`STATIC_FRAMES`] timed frames that each type one character at the
/// prompt of `grid` first, after one untimed frame.
fn typing_frames(painter: &mut impl Painter, grid: &mut Grid) -> Vec<Duration> {
    let mut parser = AnsiParser::new();
    painter.paint(&grid.frame_view());
    (0..STATIC_FRAMES)
        .map(|_| {
            parser.advance(grid, b"x");
            let t = Instant::now();
            painter.paint(&grid.frame_view());
            t.elapsed()
        })
        .collect()
}

/// The `p`th percentile of `v` by nearest rank: the value at rank
/// `ceil(p / 100 * n)`, clamped to `1..=n`. `None` when `v` is empty.
fn pct(v: &[Duration], p: f64) -> Option<Duration> {
    let mut s = v.to_vec();
    s.sort_unstable();
    let n = s.len();
    let rank = ((p / 100.0 * n as f64).ceil() as usize).clamp(1, n.max(1));
    s.get(rank - 1).copied()
}

/// [`pct`] formatted in milliseconds; an error when there are no samples.
fn pct_ms(v: &[Duration], p: f64) -> Result<String, String> {
    let d = pct(v, p).ok_or("no samples")?;
    Ok(format!("{:.2} ms", d.as_secs_f64() * 1e3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn pct_is_nearest_rank_with_defined_endpoints() {
        let hundred: Vec<Duration> = (1..=100).rev().map(d).collect();
        assert_eq!(pct(&hundred, 0.0), Some(d(1)));
        assert_eq!(pct(&hundred, 50.0), Some(d(50)));
        assert_eq!(pct(&hundred, 99.0), Some(d(99)));
        assert_eq!(pct(&hundred, 100.0), Some(d(100)));
        // 60 frames: p99 is rank ceil(59.4) = 60, the worst frame.
        let sixty: Vec<Duration> = (1..=60).map(d).collect();
        assert_eq!(pct(&sixty, 99.0), Some(d(60)));
        assert_eq!(pct(&sixty, 50.0), Some(d(30)));
        assert_eq!(pct(&[d(7)], 99.0), Some(d(7)));
        assert_eq!(pct(&[], 50.0), None);
        assert!(pct_ms(&[], 50.0).is_err());
    }

    #[test]
    fn parse_args_reads_options_and_rejects_bad_numbers() {
        let o = parse_args(vec!["--hz".into(), "120".into(), "a.vt".into()]).expect("valid");
        assert_eq!(
            (o.hz, o.secs, o.workloads),
            (120.0, 2.0, vec!["a.vt".to_owned()])
        );
        for bad in [
            vec!["--hz"],
            vec!["--hz", "0"],
            vec!["--secs", "-1"],
            vec!["--secs", "x"],
        ] {
            let args = bad.into_iter().map(String::from).collect();
            assert!(parse_args(args).is_err());
        }
    }

    #[test]
    fn fold_fixtures_match_except_for_fold_blocks() {
        for scrolled in [false, true] {
            let plain = fold_grid(false, scrolled).expect("plain fixture");
            let folded = fold_grid(true, scrolled).expect("folded fixture");
            assert_eq!(plain.fold_blocks().len(), 0);
            assert_eq!(folded.fold_blocks().len(), FOLD_BLOCKS);
            assert_eq!(plain.scrollback_len(), folded.scrollback_len());
            assert_eq!(plain.view_offset, folded.view_offset);
            assert_eq!(plain.view_offset, if scrolled { ROWS / 2 } else { 0 });
            for y in 0..ROWS {
                assert_eq!(plain.row(y), folded.row(y), "row {y}");
            }
        }
    }

    struct Counting(usize);
    impl Painter for Counting {
        fn paint(&mut self, _: &FrameView) {
            self.0 += 1;
        }
    }

    struct Panicking;
    impl Painter for Panicking {
        fn paint(&mut self, _: &FrameView) {
            panic!("paint failed");
        }
    }

    /// Records whether another thread could take the grid lock while it
    /// painted. Uses only `lock`, so it holds for any mutex type.
    struct LockProbe(Arc<Mutex<Grid>>, Vec<bool>);
    impl Painter for LockProbe {
        fn paint(&mut self, _: &FrameView) {
            let (tx, rx) = mpsc::channel();
            let grid = Arc::clone(&self.0);
            // Left blocked if the lock is held; released once paint returns.
            thread::spawn(move || {
                let _g = grid.lock();
                let _ = tx.send(());
            });
            self.1.push(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        }
    }

    #[test]
    fn render_loop_paints_without_the_grid_lock() {
        let grid = Arc::new(Mutex::new(Grid::new(4, 2)));
        let mut probe = LockProbe(Arc::clone(&grid), Vec::new());
        let done = AtomicBool::new(true);
        let times = render_loop(&mut probe, &grid, &done, Duration::ZERO);
        assert_eq!(probe.1, [true], "one frame, painted unlocked");
        assert_eq!(times.1.len(), 1);
    }

    fn short() -> Options {
        Options {
            hz: 1000.0,
            secs: 0.01,
            workloads: Vec::new(),
        }
    }

    #[test]
    fn streaming_reports_failures_instead_of_empty_samples() {
        let data = b"hello\r\n".repeat(64);
        let none = streaming(&data, &short(), || None::<Counting>);
        assert_eq!(none.err().as_deref(), Some("renderer failed to start"));
        let panics = streaming(&data, &short(), || Some(Panicking));
        assert_eq!(panics.err().as_deref(), Some("render thread panicked"));
        let ok = streaming(&data, &short(), || Some(Counting(0))).expect("measures");
        assert!(!ok.frame.is_empty());
        assert_eq!(ok.frame.len(), ok.hold.len());
        assert_eq!(ok.frame.len(), ok.wait.len());
    }
}
