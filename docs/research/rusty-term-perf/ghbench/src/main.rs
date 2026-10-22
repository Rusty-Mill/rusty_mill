//! Scratch oracle baseline: same workloads and fill as rtbench, run through
//! libghostty-vt (phall1 fork, prebuilt). Not committed; comparison only.
use libghostty_vt::Terminal;

mod oracle;
use std::time::Instant;

fn rss_kb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(field))
            .and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn fill_bytes(cols: usize, rows: usize, lines: usize) -> Vec<u8> {
    let line: String = (0..cols).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    let mut buf = Vec::new();
    for i in 0..lines + rows {
        buf.extend_from_slice(format!("\x1b[3{}m", i % 8).as_bytes());
        buf.extend_from_slice(&line.as_bytes()[..cols - 8]);
        buf.extend_from_slice(format!("{:06}\r\n", i % 1_000_000).as_bytes());
    }
    buf
}

fn throughput(dir: &str, iters: u32) {
    let mut paths: Vec<_> = std::fs::read_dir(dir).expect("workload dir").filter_map(|e| e.ok())
        .map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "vt")).collect();
    paths.sort();
    for p in paths {
        let data = std::fs::read(&p).expect("read");
        let run = |n: u32| for _ in 0..n {
            let mut t = Terminal::new(120, 40).expect("term");
            t.set_scrollback_max_bytes(None).expect("sbb");
            t.set_scrollback_max_lines(Some(10_000)).expect("sb");
            for c in data.chunks(4096) { t.vt_write(c); }
            std::hint::black_box(&t);
        };
        run(1);
        let t0 = Instant::now();
        run(iters);
        let s = t0.elapsed().as_secs_f64();
        println!("{:22} {:>9.1} MB/s", p.file_stem().unwrap().to_string_lossy(),
            data.len() as f64 * iters as f64 / 1e6 / s);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "grid" {
        print!("{}", oracle::dump(&args[2]));
        return;
    }
    if args.len() > 2 && args[1] == "dump" {
        let data = std::fs::read(&args[2]).expect("read");
        let mut t = Terminal::new(120, 40).expect("term");
        for c in data.chunks(4096) { t.vt_write(c); }
        let opts = libghostty_vt::fmt::FormatterOptions::new().with_format(libghostty_vt::fmt::Format::Plain).with_trim(true).with_unwrap(false);
        let mut f = libghostty_vt::fmt::Formatter::new(&t, opts).expect("fmt");
        let out = f.format_alloc(None).expect("format");
        let text = String::from_utf8_lossy(&out);
        let lines: Vec<&str> = text.lines().collect();
        for l in &lines[lines.len().saturating_sub(40)..] { println!("{}", l.trim_end()); }
        return;
    }
    if args.len() > 1 { throughput(&args[1], 20); return; }
    let (cols, rows, lines) = (200u16, 50u16, 10_000usize);
    let buf = fill_bytes(cols as usize, rows as usize, lines);
    let rss0 = rss_kb("VmRSS:");
    let t0 = Instant::now();
    let mut t = Terminal::new(cols, rows).expect("term");
    t.set_scrollback_max_bytes(None).expect("sbb");
    t.set_scrollback_max_lines(Some(lines)).expect("sb");
    println!("max_bytes={:?} max_lines={:?}", t.scrollback_max_bytes(), t.scrollback_max_lines());
    for c in buf.chunks(65536) { t.vt_write(c); }
    let fill = t0.elapsed();
    let rss1 = rss_kb("VmRSS:");
    println!("fill {lines} lines @ {cols}x{rows}: {:.1} ms; RSS delta {} KiB ({:.1} B/cell); VmHWM {} KiB",
        fill.as_secs_f64()*1e3, rss1 - rss0,
        (rss1 - rss0) as f64 * 1024.0 / ((lines + rows as usize) * cols as usize) as f64, rss_kb("VmHWM:"));
    println!("scrollback_rows={:?} total_rows={:?}", t.scrollback_rows(), t.total_rows());
    let mut out = Vec::new();
    let t0 = Instant::now();
    t.encode_snapshot(&mut out).expect("snapshot");
    println!("encode_snapshot (screen + {lines} lines history): {:.2} ms, {} bytes", t0.elapsed().as_secs_f64()*1e3, out.len());
    for (c, r) in [(150u16, 50u16), (200, 50), (80, 24), (200, 50)] {
        let t0 = Instant::now();
        t.resize(c, r, 8, 16).expect("resize");
        println!("resize -> {c}x{r}: {:.2} ms", t0.elapsed().as_secs_f64()*1e3);
    }
}
