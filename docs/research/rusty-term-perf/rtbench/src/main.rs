//! Scratch baseline: scrollback memory, resize/reflow and a full-state
//! re-encode proxy for rusty_term core. Not committed.
use rusty_term::core::{AnsiParser, Grid};
use std::time::Instant;

fn rss_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(field))
            .and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn filled(cols: usize, rows: usize, lines: usize) -> (Grid, AnsiParser) {
    let mut g = Grid::new(cols, rows);
    let mut p = AnsiParser::new();
    let line: String = (0..cols).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    let mut buf = Vec::new();
    for i in 0..lines + rows {
        buf.extend_from_slice(format!("\x1b[3{}m", i % 8).as_bytes());
        buf.extend_from_slice(&line.as_bytes()[..cols - 8]);
        buf.extend_from_slice(format!("{:06}\r\n", i % 1_000_000).as_bytes());
    }
    for c in buf.chunks(65536) { p.advance(&mut g, c); }
    (g, p)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "dump" {
        let data = std::fs::read(&args[2]).expect("read");
        let mut g = Grid::new(120, 40);
        let mut p = AnsiParser::new();
        for c in data.chunks(4096) { p.advance(&mut g, c); }
        for r in 0..40 {
            let row: String = g.row(r).iter().filter(|c| c.flags & rusty_term::core::WIDE_TRAILER == 0).map(|c| c.ch).collect();
            println!("{}", row.trim_end());
        }
        return;
    }
    let (cols, rows, lines) = (200, 50, 10_000);
    let rss0 = rss_kb("VmRSS:");
    let t = Instant::now();
    let (mut g, _p) = filled(cols, rows, lines);
    let fill = t.elapsed();
    let rss1 = rss_kb("VmRSS:");
    let hwm = rss_kb("VmHWM:");
    println!("fill {lines} lines @ {cols}x{rows}: {:.1} ms; RSS delta {} KiB ({:.1} B/cell over {} cells); VmHWM {} KiB",
        fill.as_secs_f64()*1e3, rss1 - rss0, (rss1 - rss0) as f64 * 1024.0 / ((lines + rows) * cols) as f64,
        (lines + rows) * cols, hwm);
    // Resize / reflow: shrink then grow columns (full history rewrap each time).
    for (c, r) in [(150, 50), (200, 50), (80, 24), (200, 50)] {
        let t = Instant::now();
        g.resize(c, r);
        println!("resize -> {c}x{r}: {:.2} ms", t.elapsed().as_secs_f64()*1e3);
    }
    // Full snapshot of 200x50 + 10k lines, compared with libghostty's
    // encode_snapshot (ghbench). Re-fill so the resizes above don't shape it.
    let (g2, _p2) = filled(cols, rows, lines);
    let t = Instant::now();
    let snap = g2.encode_snapshot();
    let enc = t.elapsed();
    let t = Instant::now();
    let back = rusty_term::core::Grid::decode_snapshot(&snap).expect("decode");
    println!("encode_snapshot ({cols}x{rows} + {} lines): {:.2} ms, {} bytes ({:.2} B/cell); decode {:.2} ms",
        g2.scrollback_len(), enc.as_secs_f64()*1e3, snap.len(),
        snap.len() as f64 / ((g2.scrollback_len() + rows) * cols) as f64, t.elapsed().as_secs_f64()*1e3);
    std::hint::black_box(back);
    // Snapshot proxy: no serializer exists; time the closest existing
    // full-state read (viewport snapshot) and a full re-encode of the
    // visible screen via the TUI renderer's frame data.
    let t = Instant::now();
    let mut n = 0usize;
    for _ in 0..100 { let f = g.snapshot_viewport(); n += std::hint::black_box(&f).rows.len(); }
    println!("snapshot_viewport (viewport only) x100: {:.3} ms each ({n} rows)", t.elapsed().as_secs_f64()*1e3/100.0);
}
