//! Frame-time benchmark for the windowed front-end's CPU frame path.
//!
//! Drives a [`Grid`] and the real CPU compositor offscreen
//! (`rusty_term::gui::bench::FrameBench`, no window or present) and reports
//! frame time and grid-lock hold time. Two modes:
//!
//! - **Streaming.** A parser thread feeds a `bench/workloads` file into a
//!   shared `Mutex<Grid>` in 4 KiB chunks, locking per chunk as the PTY
//!   reader does (`lock_bulk`), looping the file for at least `--secs`. With
//!   `--pipe` the chunks come from 4 KiB reads of an OS pipe that a writer
//!   thread fills, as a PTY read would; without it the parser re-locks back
//!   to back, the worst case for the render thread. A render thread paints at
//!   `--hz` as the window does: lock, snapshot the frame
//!   ([`Grid::frame_view`]), unlock, paint. Reported: frame ms, the reader's
//!   hold per chunk, the render thread's snapshot hold (the parser is blocked
//!   for it) and its wait for the lock, and parser MB/s with rendering
//!   against parsing alone. Reader hold excludes lock acquisition and deferral.
//! - **Static folds.** Frame ms for the same text with 0 and with 1,024
//!   command fold blocks (OSC 133 `C`..`D` around each command's output), at
//!   the live view and scrolled back half a screen into history. Every cell
//!   drawn maps its view row through the fold blocks.
//!
//! A run that cannot measure (no font, a renderer that fails to start or
//! panics, no samples) exits non-zero instead of printing zeros.
//!
//! Usage:
//!   python3 bench/gen_workloads.py --size quick
//!   cargo run --release --features gui --bin bench_frame_time
//!   cargo run --release --features gui --bin bench_frame_time -- --hz 120 --secs 3 bench/workloads/scroll_heavy.vt
//!   cargo run --release --features gui --bin bench_frame_time -- --pipe
//!
//! The font is the window's default (`$RUSTY_TERM_FONT` or a system
//! monospace font).

use std::env;
use std::fs;
use std::io::{Read, Write};
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
    pipe: bool,
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
        format!("{e}\nusage: bench_frame_time [--hz N] [--secs N] [--pipe] [WORKLOAD.vt...]")
    })?;
    if frame_bench().is_none() {
        return Err("no usable monospace font: set RUSTY_TERM_FONT".to_owned());
    }
    let feed = if opts.pipe { "pipe" } else { "back to back" };
    println!(
        "## Streaming: {COLS}x{ROWS}, {CHUNK} B chunks ({feed}), render at {} Hz, >= {} s per workload\n",
        opts.hz, opts.secs
    );
    println!(
        "| workload | frames | frame p50 | frame p99 | reader hold p50 | reader hold p99 | snapshot hold p99 | render wait p99 | render wait max | parse MB/s alone | parse MB/s rendering |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
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
        let alone = parse_alone(&data, &opts)
            .map_err(|e| format!("{name}: {e}"))?
            .mb_per_s;
        let s = streaming(&data, &opts, frame_bench).map_err(|e| format!("{name}: {e}"))?;
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {alone:.1} | {:.1} |",
            s.frame.len(),
            pct_ms(&s.frame, 50.0)?,
            pct_ms(&s.frame, 99.0)?,
            pct_ms(&s.reader_hold, 50.0)?,
            pct_ms(&s.reader_hold, 99.0)?,
            pct_ms(&s.hold, 99.0)?,
            pct_ms(&s.wait, 99.0)?,
            pct_ms(&s.wait, 100.0)?,
            s.mb_per_s,
        );
    }
    println!("\n## Static folds: {COLS}x{ROWS}, {STATIC_FRAMES} frames each\n");
    println!("| fold blocks | view | frame p50 | frame p99 |");
    println!("|---:|---|---:|---:|");
    for folds in [false, true] {
        for scrolled in [false, true] {
            let grid = fold_grid(folds, scrolled)?;
            let mut fb = frame_bench().ok_or("renderer failed to start")?;
            let frames = static_frames(&mut fb, &grid);
            let view = if scrolled { "scrolled back" } else { "live" };
            println!(
                "| {} | {view} | {} | {} |",
                grid.fold_blocks().len(),
                pct_ms(&frames, 50.0)?,
                pct_ms(&frames, 99.0)?
            );
        }
    }
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut opts = Options {
        hz: 60.0,
        secs: 2.0,
        pipe: false,
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
            "--pipe" => opts.pipe = true,
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

/// What [`feed`] measured: parser MB/s and its lock hold per chunk.
struct Fed {
    mb_per_s: f64,
    hold: Vec<Duration>,
}

/// Feed `data` (looped) through a parser into `grid` in [`CHUNK`]-byte
/// pieces, locking per chunk like the PTY reader, until `opts.secs` have
/// passed and at least one whole pass is done. With `opts.pipe` the pieces
/// are reads of a pipe that a writer thread fills.
fn feed(grid: &Mutex<Grid>, data: &[u8], opts: &Options) -> Result<Fed, String> {
    let mut parser = AnsiParser::new();
    let mut hold = Vec::new();
    let mut step = |chunk: &[u8]| {
        let mut g = grid.lock_bulk();
        let t = Instant::now(); // the hold, not the wait to acquire
        parser.advance(&mut g, chunk);
        let _ = g.take_host_out();
        drop(g);
        hold.push(t.elapsed());
        let _ = parser.take_responses();
    };
    let start = Instant::now();
    let bytes = if opts.pipe {
        feed_pipe(data, opts.secs, &mut step)?
    } else {
        let mut bytes = 0usize;
        loop {
            data.chunks(CHUNK).for_each(&mut step);
            bytes += data.len();
            if start.elapsed().as_secs_f64() >= opts.secs {
                break bytes;
            }
        }
    };
    let mb_per_s = bytes as f64 / 1e6 / start.elapsed().as_secs_f64();
    Ok(Fed { mb_per_s, hold })
}

/// Write `data` (looped, at least once) into a pipe from a writer thread for
/// `secs`, and hand each read of up to [`CHUNK`] bytes to `step`. Returns the
/// bytes read.
///
/// The read end is owned by the scope's closure, so an early return or a
/// panic in `step` closes it before the scope joins the writer: the writer's
/// `write_all` then fails instead of blocking on a full pipe forever.
fn feed_pipe(data: &[u8], secs: f64, step: &mut impl FnMut(&[u8])) -> Result<usize, String> {
    let (rx, tx) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;
    feed_pipe_with_reader(data, secs, rx, tx, step)
}

// Keep the real pipe writer, but allow tests to inject read failures.
fn feed_pipe_with_reader(
    data: &[u8],
    secs: f64,
    rx: impl Read,
    mut tx: std::io::PipeWriter,
    step: &mut impl FnMut(&[u8]),
) -> Result<usize, String> {
    thread::scope(|s| {
        // Own the reader inside the scope body: errors and unwinding must
        // close it before scope implicitly joins a possibly blocked writer.
        let mut rx = rx;
        let writer = s.spawn(move || -> std::io::Result<()> {
            let start = Instant::now();
            loop {
                tx.write_all(data)?;
                if start.elapsed().as_secs_f64() >= secs {
                    return Ok(()); // dropping `tx` ends the reader
                }
            }
        });
        let mut buf = vec![0u8; CHUNK];
        let mut bytes = 0usize;
        loop {
            let n = match rx.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("pipe read: {e}")),
            };
            step(&buf[..n]);
            bytes += n;
        }
        drop(rx);
        writer
            .join()
            .map_err(|_| "pipe writer panicked".to_owned())?
            .map_err(|e| format!("pipe write: {e}"))?;
        Ok(bytes)
    })
}

fn parse_alone(data: &[u8], opts: &Options) -> Result<Fed, String> {
    feed(&Mutex::new(Grid::new(COLS, ROWS)), data, opts)
}

#[derive(Debug)]
struct Streamed {
    frame: Vec<Duration>,
    /// The render thread's snapshot hold.
    hold: Vec<Duration>,
    wait: Vec<Duration>,
    reader_hold: Vec<Duration>,
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
    let fed = if started {
        feed(&grid, data, opts)
    } else {
        Ok(Fed {
            mb_per_s: 0.0,
            hold: Vec::new(),
        })
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
    let fed = fed?;
    Ok(Streamed {
        frame,
        hold,
        wait,
        reader_hold: fed.hold,
        mb_per_s: fed.mb_per_s,
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
            (o.hz, o.secs, o.pipe, o.workloads),
            (120.0, 2.0, false, vec!["a.vt".to_owned()])
        );
        assert!(
            parse_args(vec!["--pipe".into(), "a.vt".into()])
                .expect("valid")
                .pipe
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
            pipe: false,
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
        assert!(!ok.reader_hold.is_empty());
    }

    /// A consumer that fails on its first chunk must not leave the writer
    /// blocked on a full pipe: the read end closes before the writer is
    /// joined, so `write_all` fails and the feed returns.
    #[test]
    fn pipe_feed_returns_when_the_consumer_fails_early() {
        let data = vec![b'x'; 1 << 20]; // far more than a pipe buffer
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                feed_pipe(&data, 60.0, &mut |_| panic!("consumer fails"))
            }));
            let _ = tx.send(r.is_err());
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
    }

    #[test]
    fn pipe_feed_parses_every_byte_at_least_once() {
        let data = b"hello\r\n".repeat(2048); // several chunks
        let opts = Options {
            pipe: true,
            ..short()
        };
        let fed = parse_alone(&data, &opts).expect("pipe feed");
        assert!(fed.mb_per_s > 0.0);
        assert!(fed.hold.len() >= data.len().div_ceil(CHUNK));
        let ok = streaming(&data, &opts, || Some(Counting(0))).expect("measures");
        assert!(!ok.reader_hold.is_empty());
    }

    struct InterruptedOnce(std::io::PipeReader, bool);

    impl Read for InterruptedOnce {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.1 {
                self.1 = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.0.read(buf)
        }
    }

    #[test]
    fn pipe_feed_retries_interrupted_reads_without_losing_bytes() {
        let data = b"hello\r\n".repeat(2048);
        let (rx, tx) = std::io::pipe().expect("pipe");
        let mut received = Vec::new();
        let bytes = feed_pipe_with_reader(
            &data,
            0.0, // exactly one complete write
            InterruptedOnce(rx, false),
            tx,
            &mut |chunk| received.extend_from_slice(chunk),
        )
        .expect("interrupted read retried");
        assert_eq!(bytes, data.len());
        assert_eq!(received, data);
    }

    struct FailingReader(std::io::PipeReader, bool);

    impl Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.1 {
                return Err(std::io::Error::other("injected read failure"));
            }
            self.1 = true;
            self.0.read(buf)
        }
    }

    #[test]
    fn pipe_feed_closes_reader_before_join_on_consumer_failure() {
        const CASE: &str = "RUSTY_TERM_PIPE_FAILURE_CASE";
        if let Ok(case) = env::var(CASE) {
            // Larger than the pipe buffer: the first write cannot complete
            // while the consumer stops after at most one CHUNK-sized read.
            let data = vec![b'x'; 16 * 1024 * 1024];
            match case.as_str() {
                "read-error" => {
                    let (rx, tx) = std::io::pipe().expect("pipe");
                    let err = feed_pipe_with_reader(
                        &data,
                        0.0,
                        FailingReader(rx, false),
                        tx,
                        &mut |_| {},
                    )
                    .expect_err("injected failure");
                    assert_eq!(err, "pipe read: injected read failure");
                }
                "step-panic" => {
                    let result = std::panic::catch_unwind(|| {
                        let _ = feed_pipe(&data, 0.0, &mut |_| panic!("consumer failed"));
                    });
                    assert!(result.is_err(), "consumer panic must propagate");
                }
                _ => panic!("unknown case: {case}"),
            }
            return;
        }

        for case in ["step-panic", "read-error"] {
            // A subprocess makes a join deadlock a bounded test failure.
            let mut child =
                std::process::Command::new(env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "tests::pipe_feed_closes_reader_before_join_on_consumer_failure",
                        "--nocapture",
                    ])
                    .env(CASE, case)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("failure subprocess");
            let start = Instant::now();
            loop {
                if let Some(status) = child.try_wait().expect("subprocess status") {
                    assert!(status.success(), "{case}: {status}");
                    break;
                }
                if start.elapsed() > Duration::from_secs(10) {
                    child.kill().expect("kill deadlocked subprocess");
                    child.wait().expect("reap subprocess");
                    panic!("{case}: pipe writer join hung after consumer failure");
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
