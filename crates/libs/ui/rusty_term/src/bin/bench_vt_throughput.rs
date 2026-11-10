//! Pure in-process throughput benchmark for rusty_term's parser+grid layer.
//!
//! Feeds pre-generated VT/ANSI byte-stream workloads (see
//! `bench/gen_workloads.py`) through `AnsiParser::advance` in realistic
//! PTY-read-sized chunks and reports MB/s (aggregate, plus best and median
//! per pass), plus the per-chunk processing
//! latency distribution (p50/p95/p99/p99.9). No process spawn, no PTY, no
//! display — this is the rusty_term-only half of the benchmark harness in
//! `bench/`, useful as a fast regression signal in CI. The other half
//! (`bench/run_bench.py`) drives rusty_term and other terminal emulators as
//! black-box processes for cross-terminal comparison.
//!
//! The latency numbers are per `advance()` call on one CHUNK-sized read,
//! i.e. "how long does rusty_term take to process one PTY read", not
//! end-to-end keystroke-to-pixel input latency — that would require driving
//! a real render, which this binary deliberately doesn't do.
//!
//! Usage:
//!   python3 bench/gen_workloads.py
//!   cargo run --release --bin bench_vt_throughput               # all workloads, 20 iterations each
//!   cargo run --release --bin bench_vt_throughput -- 100         # 100 iterations each
//!   cargo run --release --bin bench_vt_throughput -- 50 bench/workloads/sgr_churn.vt
//!   cargo run --release --bin bench_vt_throughput -- --once bench/workloads/scroll_heavy.vt
//!
//! `--once` feeds each file through the parser exactly once, with no warmup,
//! timing or latency pass, and prints nothing. It exists to be run under an
//! instruction counter (`bench/ir_gate.py` runs it under cachegrind), where
//! the deterministic count, not wall-clock time, is the CI regression gate.

use std::env;
use std::fs;
use std::time::{Duration, Instant};

use rusty_term::core::{AnsiParser, Grid};

const CHUNK: usize = 4096; // typical PTY read() size
const COLS: usize = 120;
const ROWS: usize = 40;
const DEFAULT_ITERATIONS: u32 = 20;
const DEFAULT_WORKLOAD_DIR: &str = "bench/workloads";

/// One pass: a fresh grid and parser fed the whole input in `CHUNK` reads.
fn feed(data: &[u8]) {
    let mut grid = Grid::new(COLS, ROWS);
    let mut parser = AnsiParser::new();
    for chunk in data.chunks(CHUNK) {
        parser.advance(&mut grid, chunk);
        parser.take_responses();
        grid.take_host_out();
    }
    std::hint::black_box(&grid);
}

/// Runs `iterations` passes and returns each pass's wall-clock time. One
/// `Instant::now()` per pass of a whole workload is noise next to the pass.
fn feed_timed(data: &[u8], iterations: u32) -> Vec<Duration> {
    (0..iterations)
        .map(|_| {
            let t0 = Instant::now();
            feed(data);
            t0.elapsed()
        })
        .collect()
}

/// MB/s for `bytes` processed in `elapsed`.
fn mb_per_s(bytes: u64, elapsed: Duration) -> f64 {
    (bytes as f64 / 1_000_000.0) / elapsed.as_secs_f64().max(1e-9)
}

/// Best (fastest pass) and median per-pass MB/s. `None` for no passes.
fn best_and_median(bytes: u64, passes: &[Duration]) -> Option<(f64, f64)> {
    let mut sorted = passes.to_vec();
    sorted.sort_unstable();
    let best = *sorted.first()?;
    let mid = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2
    } else {
        sorted[mid]
    };
    Some((mb_per_s(bytes, best), mb_per_s(bytes, median)))
}

/// Same work as `feed`, but timed per chunk instead of once per pass. Kept as a separate pass (not folded into `feed`'s hot loop) so the
/// `Instant::now()` call per chunk never perturbs the MB/s number above —
/// this exists purely to characterize the latency *distribution* of one
/// PTY-read-sized `advance()` call, not to replace the throughput figure.
fn feed_with_latencies(data: &[u8], iterations: u32, latencies_ns: &mut Vec<u64>) {
    for _ in 0..iterations {
        let mut grid = Grid::new(COLS, ROWS);
        let mut parser = AnsiParser::new();
        for chunk in data.chunks(CHUNK) {
            let t0 = Instant::now();
            parser.advance(&mut grid, chunk);
            parser.take_responses();
            grid.take_host_out();
            latencies_ns.push(t0.elapsed().as_nanos() as u64);
        }
        std::hint::black_box(&grid);
    }
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile_ns(sorted_ns: &[u64], p: f64) -> u64 {
    if sorted_ns.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted_ns.len() as f64 - 1.0)).round() as usize;
    sorted_ns[idx.min(sorted_ns.len() - 1)]
}

fn bench_one(path: &str, iterations: u32) {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip {path}: {e}");
            return;
        }
    };
    if data.is_empty() {
        eprintln!("skip {path}: empty file");
        return;
    }

    feed(&data); // warmup

    let start = Instant::now();
    let passes = feed_timed(&data, iterations);
    let elapsed = start.elapsed();

    let total_bytes = data.len() as u64 * iterations as u64;
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path);

    println!(
        "{name:22} {:>10} bytes  x{iterations:<5} {:>9.1} ms total  {:>9.1} MB/s",
        data.len(),
        elapsed.as_secs_f64() * 1000.0,
        mb_per_s(total_bytes, elapsed),
    );
    // Indented so `floors.py`, which parses the line above, never matches it.
    if let Some((best, median)) = best_and_median(data.len() as u64, &passes) {
        println!(
            "{:22} per pass  best {best:>9.1} MB/s  median {median:>9.1} MB/s",
            ""
        );
    }

    // Latency-distribution pass, capped independently of `iterations` so a
    // large `-- 1000` throughput run doesn't also multiply this pass by
    // 1000x — the percentiles converge well below that sample count.
    let latency_iterations = iterations.min(50);
    let mut latencies_ns = Vec::new();
    feed_with_latencies(&data, latency_iterations, &mut latencies_ns);
    latencies_ns.sort_unstable();

    println!(
        "{:22} chunk latency (n={:<6}) p50={:>7} p95={:>7} p99={:>7} p99.9={:>7} (ns/{CHUNK}B chunk)",
        "",
        latencies_ns.len(),
        percentile_ns(&latencies_ns, 50.0),
        percentile_ns(&latencies_ns, 95.0),
        percentile_ns(&latencies_ns, 99.0),
        percentile_ns(&latencies_ns, 99.9),
    );
}

fn discover_workloads(dir: &str) -> Vec<String> {
    let mut found: Vec<String> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("vt"))
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    found.sort();
    found
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--once") {
        args.remove(i);
        for path in &args {
            match fs::read(path) {
                Ok(data) => feed(&data),
                Err(e) => {
                    eprintln!("{path}: {e}");
                    std::process::exit(2);
                }
            }
        }
        return;
    }

    let (iterations, rest): (u32, &[String]) =
        match args.first().and_then(|s| s.parse::<u32>().ok()) {
            Some(n) => (n, &args[1..]),
            None => (DEFAULT_ITERATIONS, &args[..]),
        };

    let paths: Vec<String> = if rest.is_empty() {
        let found = discover_workloads(DEFAULT_WORKLOAD_DIR);
        if found.is_empty() {
            eprintln!(
                "no workload files given and `{DEFAULT_WORKLOAD_DIR}` has none — run \
                 `python3 bench/gen_workloads.py` first, or pass file paths explicitly."
            );
            std::process::exit(1);
        }
        found
    } else {
        rest.to_vec()
    };

    println!(
        "--- rusty_term parser+grid throughput ({COLS}x{ROWS} grid, {CHUNK}B chunks, x{iterations} each) ---"
    );
    for path in &paths {
        bench_one(path, iterations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn best_is_the_fastest_pass_and_median_the_middle_one() {
        // 1 MB per pass: 10 ms is 100 MB/s, 20 ms is 50 MB/s.
        let got = best_and_median(1_000_000, &[ms(40), ms(10), ms(20)]);
        assert_eq!(got, Some((100.0, 50.0)));
    }

    #[test]
    fn median_of_an_even_count_averages_the_middle_two() {
        let got = best_and_median(1_000_000, &[ms(10), ms(30), ms(20), ms(40)]);
        assert_eq!(got, Some((100.0, 40.0)));
    }

    #[test]
    fn no_passes_gives_no_figures() {
        assert_eq!(best_and_median(1_000_000, &[]), None);
    }
}
