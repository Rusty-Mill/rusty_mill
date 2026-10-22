//! Frame-time benchmark for the windowed front-end's CPU frame path.
//!
//! Drives a [`Grid`] and the real CPU compositor offscreen
//! (`rusty_term::gui::bench::FrameBench`, no window or present) and reports
//! frame time and grid-lock hold time. Two modes:
//!
//! - **Streaming.** A parser thread feeds a `bench/workloads` file into a
//!   shared `Mutex<Grid>` in 4 KiB chunks, locking per chunk as the PTY
//!   reader does, looping the file for at least `--secs`. A render thread
//!   paints at `--hz`, holding the grid lock for the whole frame as the window
//!   does today. Reported: frame ms, lock hold ms (the parser is blocked for
//!   it), the render thread's wait for the lock, and parser MB/s with
//!   rendering against parsing alone.
//! - **Static folds.** Frame ms for a full grid with 0 and with 1,024 command
//!   fold blocks (OSC 133 `C`..`D`), at the live view and scrolled back into
//!   history. Every cell drawn maps its view row through the fold blocks.
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
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusty_term::core::{AnsiParser, Grid};
use rusty_term::gui::bench::FrameBench;

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

fn main() -> ExitCode {
    let opts = match parse_args(env::args().skip(1).collect()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\nusage: bench_frame_time [--hz N] [--secs N] [WORKLOAD.vt...]");
            return ExitCode::from(2);
        }
    };
    if FrameBench::new(COLS, ROWS, FONT_PX).is_none() {
        eprintln!("no usable monospace font: set RUSTY_TERM_FONT");
        return ExitCode::from(2);
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
        let data = match fs::read(path) {
            Ok(d) if !d.is_empty() => d,
            Ok(_) => {
                eprintln!("{path}: empty");
                return ExitCode::from(2);
            }
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::from(2);
            }
        };
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".vt");
        let alone = parse_alone(&data, opts.secs);
        let s = streaming(&data, &opts);
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {alone:.1} | {:.1} |",
            s.frame.len(),
            ms(pct(&s.frame, 50.0)),
            ms(pct(&s.frame, 99.0)),
            ms(pct(&s.hold, 50.0)),
            ms(pct(&s.hold, 99.0)),
            ms(pct(&s.hold, 100.0)),
            ms(pct(&s.wait, 99.0)),
            s.mb_per_s,
        );
    }
    println!("\n## Static folds: {COLS}x{ROWS}, {STATIC_FRAMES} frames each\n");
    println!("| fold blocks | view | frame p50 | frame p99 |");
    println!("|---:|---|---:|---:|");
    for blocks in [0, FOLD_BLOCKS] {
        for scrolled in [false, true] {
            let (grid, made) = fold_grid(blocks, scrolled);
            let frames = static_frames(&grid);
            let view = if scrolled { "scrolled back" } else { "live" };
            println!(
                "| {made} | {view} | {} | {} |",
                ms(pct(&frames, 50.0)),
                ms(pct(&frames, 99.0))
            );
        }
    }
    ExitCode::SUCCESS
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

struct Streamed {
    frame: Vec<Duration>,
    hold: Vec<Duration>,
    wait: Vec<Duration>,
    mb_per_s: f64,
}

fn streaming(data: &[u8], opts: &Options) -> Streamed {
    let grid = Arc::new(Mutex::new(Grid::new(COLS, ROWS)));
    let done = Arc::new(AtomicBool::new(false));
    let render = {
        let (grid, done) = (Arc::clone(&grid), Arc::clone(&done));
        let period = Duration::from_secs_f64(1.0 / opts.hz);
        thread::spawn(move || render_loop(&grid, &done, period))
    };
    let mb_per_s = feed(&grid, data, opts.secs);
    done.store(true, Ordering::Relaxed);
    let (frame, hold, wait) = render.join().unwrap_or_default();
    Streamed {
        frame,
        hold,
        wait,
        mb_per_s,
    }
}

type FrameTimes = (Vec<Duration>, Vec<Duration>, Vec<Duration>);

/// Paint every `period` until `done`, as the window does today: take the
/// grid lock, paint the whole frame, release. Returns per-frame (frame,
/// lock hold, lock wait) times.
fn render_loop(grid: &Mutex<Grid>, done: &AtomicBool, period: Duration) -> FrameTimes {
    let mut times: FrameTimes = Default::default();
    let Some(mut fb) = FrameBench::new(COLS, ROWS, FONT_PX) else {
        return times;
    };
    let mut next = Instant::now();
    while !done.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        let g = grid.lock();
        let t1 = Instant::now();
        fb.draw(&g);
        drop(g);
        let t2 = Instant::now();
        times.0.push(t2 - t0);
        times.1.push(t2 - t1);
        times.2.push(t1 - t0);
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now; // over budget: do not try to catch up
        }
    }
    times
}

/// A full grid holding `blocks` finished command blocks (each a prompt
/// line and one output line), optionally scrolled back half a screen into
/// history. Returns the grid and the number of fold blocks it holds.
fn fold_grid(blocks: usize, scrolled: bool) -> (Grid, usize) {
    let mut g = Grid::new(COLS, ROWS);
    let mut p = AnsiParser::new();
    let mut stream = Vec::new();
    for i in 0..blocks.max(ROWS) {
        if i < blocks {
            stream.extend_from_slice(b"\x1b]133;A\x07$ make\r\n\x1b]133;C\x07");
            stream.extend_from_slice(format!("output line {i}\r\n").as_bytes());
            stream.extend_from_slice(b"\x1b]133;D;0\x07");
        } else {
            stream.extend_from_slice(format!("plain line {i}\r\n").as_bytes());
        }
    }
    p.advance(&mut g, &stream);
    if scrolled {
        g.scroll_view_up(ROWS / 2);
    }
    let made = g.fold_blocks().len();
    (g, made)
}

fn static_frames(grid: &Grid) -> Vec<Duration> {
    let Some(mut fb) = FrameBench::new(COLS, ROWS, FONT_PX) else {
        return Vec::new();
    };
    fb.draw(grid); // warm the glyph cache
    (0..STATIC_FRAMES)
        .map(|_| {
            let t = Instant::now();
            fb.draw(grid);
            t.elapsed()
        })
        .collect()
}

/// The `p`th percentile (nearest rank) of `v`; zero when empty.
fn pct(v: &[Duration], p: f64) -> Duration {
    let mut s = v.to_vec();
    s.sort_unstable();
    let Some(last) = s.len().checked_sub(1) else {
        return Duration::ZERO;
    };
    s[((p / 100.0 * last as f64).round() as usize).min(last)]
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn pct_is_nearest_rank_and_zero_when_empty() {
        let v: Vec<Duration> = (1..=100).rev().map(d).collect();
        assert_eq!(pct(&v, 0.0), d(1));
        assert_eq!(pct(&v, 50.0), d(51));
        assert_eq!(pct(&v, 99.0), d(99));
        assert_eq!(pct(&v, 100.0), d(100));
        assert_eq!(pct(&[d(7)], 99.0), d(7));
        assert_eq!(pct(&[], 50.0), Duration::ZERO);
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
}
