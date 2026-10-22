//! Pure in-process throughput benchmark for rusty_term's parser+grid layer.
//!
//! Feeds pre-generated VT/ANSI byte-stream workloads (see
//! `bench/gen_workloads.py`) through `AnsiParser::advance` in realistic
//! PTY-read-sized chunks and reports MB/s, plus the per-chunk processing
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
//!   cargo run --release --bin bench_vt_throughput -- 20 --floors bench/floors.txt
//!
//! `--floors FILE` turns the run into a regression gate (the CI bench job):
//! each line of FILE is `<workload> <minimum MB/s>`; the run prints a markdown
//! table of measured vs. floor and exits 1 if any workload falls below its
//! floor or a floored workload is missing.

use std::env;
use std::fs;
use std::time::Instant;

use rusty_term::core::{AnsiParser, Grid};

const CHUNK: usize = 4096; // typical PTY read() size
const COLS: usize = 120;
const ROWS: usize = 40;
const DEFAULT_ITERATIONS: u32 = 20;
const DEFAULT_WORKLOAD_DIR: &str = "bench/workloads";

fn feed(data: &[u8], iterations: u32) {
    for _ in 0..iterations {
        let mut grid = Grid::new(COLS, ROWS);
        let mut parser = AnsiParser::new();
        for chunk in data.chunks(CHUNK) {
            parser.advance(&mut grid, chunk);
            parser.take_responses();
            grid.take_host_out();
        }
        std::hint::black_box(&grid);
    }
}

/// Same work as `feed`, but timed per chunk instead of once for the whole
/// run. Kept as a separate pass (not folded into `feed`'s hot loop) so the
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

/// Run one workload; returns its name and MB/s, or `None` if it was skipped.
fn bench_one(path: &str, iterations: u32) -> Option<(String, f64)> {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip {path}: {e}");
            return None;
        }
    };
    if data.is_empty() {
        eprintln!("skip {path}: empty file");
        return None;
    }

    feed(&data, 1); // warmup

    let start = Instant::now();
    feed(&data, iterations);
    let elapsed = start.elapsed();

    let total_bytes = data.len() as u64 * iterations as u64;
    let secs = elapsed.as_secs_f64();
    let mb_per_s = (total_bytes as f64 / 1_000_000.0) / secs.max(1e-9);
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path);

    println!(
        "{name:22} {:>10} bytes  x{iterations:<5} {:>9.1} ms total  {:>9.1} MB/s",
        data.len(),
        elapsed.as_secs_f64() * 1000.0,
        mb_per_s,
    );

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
    Some((name.to_owned(), mb_per_s))
}

/// Parse a floors file: `<workload> <min MB/s>` per line; blank lines and
/// `#` comments are ignored.
fn parse_floors(text: &str) -> Result<Vec<(String, f64)>, String> {
    let mut floors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(min), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("line {}: expected `<workload> <min MB/s>`", n + 1));
        };
        let min: f64 = min
            .parse()
            .map_err(|_| format!("line {}: `{min}` is not a number", n + 1))?;
        floors.push((name.to_owned(), min));
    }
    Ok(floors)
}

/// Compare results with floors: a markdown table, and the failures (a
/// workload below its floor, or a floored workload that did not run).
fn check_floors(results: &[(String, f64)], floors: &[(String, f64)]) -> (String, Vec<String>) {
    let mut table = String::from("| workload | MB/s | floor | |\n|---|---:|---:|---|\n");
    let mut failures = Vec::new();
    for (name, min) in floors {
        match results.iter().find(|(r, _)| r == name) {
            Some((_, mbps)) => {
                let ok = *mbps >= *min;
                table.push_str(&format!(
                    "| {name} | {mbps:.1} | {min:.1} | {} |\n",
                    if ok { "ok" } else { "**below floor**" }
                ));
                if !ok {
                    failures.push(format!(
                        "{name}: {mbps:.1} MB/s is below the {min:.1} MB/s floor"
                    ));
                }
            }
            None => failures.push(format!("{name}: has a floor but did not run")),
        }
    }
    (table, failures)
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
    let floors = match args.iter().position(|a| a == "--floors") {
        Some(i) => {
            let Some(path) = args.get(i + 1).cloned() else {
                eprintln!("--floors needs a file argument");
                std::process::exit(2);
            };
            args.drain(i..=i + 1);
            let text = fs::read_to_string(&path).unwrap_or_else(|e| {
                eprintln!("cannot read floors file {path}: {e}");
                std::process::exit(2);
            });
            Some(parse_floors(&text).unwrap_or_else(|e| {
                eprintln!("{path}: {e}");
                std::process::exit(2);
            }))
        }
        None => None,
    };

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
    let results: Vec<(String, f64)> = paths
        .iter()
        .filter_map(|path| bench_one(path, iterations))
        .collect();

    let Some(floors) = floors else { return };
    let (table, failures) = check_floors(&results, &floors);
    println!("\n{table}");
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FLOOR: {f}");
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_parse_with_comments_and_blanks() {
        let f = parse_floors("# header\n\nascii_throughput 6.5  # note\nscroll_heavy 2\n").unwrap();
        assert_eq!(
            f,
            vec![
                ("ascii_throughput".into(), 6.5),
                ("scroll_heavy".into(), 2.0)
            ]
        );
    }

    #[test]
    fn floors_reject_malformed_lines() {
        assert!(
            parse_floors("ascii_throughput")
                .unwrap_err()
                .contains("line 1")
        );
        assert!(parse_floors("a 1 2").is_err());
        assert!(parse_floors("a fast").unwrap_err().contains("not a number"));
    }

    #[test]
    fn check_flags_slow_and_missing_workloads() {
        let floors = vec![("a".into(), 10.0), ("b".into(), 5.0), ("c".into(), 1.0)];
        let results = vec![("a".into(), 12.0), ("b".into(), 4.0)];
        let (table, failures) = check_floors(&results, &floors);
        assert_eq!(failures.len(), 2);
        assert!(failures[0].starts_with("b:"));
        assert!(failures[1].contains("did not run"));
        assert!(table.contains("| a | 12.0 | 10.0 | ok |"));
    }

    #[test]
    fn check_passes_at_or_above_floor() {
        let floors = vec![("a".into(), 10.0)];
        assert!(check_floors(&[("a".into(), 10.0)], &floors).1.is_empty());
    }
}
