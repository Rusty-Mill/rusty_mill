//! Head-to-head pty benchmark: rusty_lines vs rustyline vs reedline.
//!
//! Spawns each crate's minimal REPL (`src/bin/ed_*.rs`) under a real
//! pseudo-terminal (80×24) and feeds every editor the *identical* byte
//! stream. The driver acts as a minimal terminal: cursor-position
//! queries (`ESC[6n`) are answered, which reedline requires — it queries
//! the cursor on every repaint and blocks until the reply arrives (that
//! round-trip is part of reedline's real-terminal behavior and therefore
//! part of its numbers here).
//!
//! Two kinds of measurement:
//!   - **paced**: one key at a time, waiting for the repaint to finish —
//!     true per-keystroke latency (write → last output byte).
//!   - **burst**: many keys in one write — sustained throughput; editors
//!     that coalesce repaints under pending input (readline's trick,
//!     which rustyline shares) shine here.
//!
//! Bytes written to the terminal per keystroke are recorded too: that is
//! what a slow ssh link actually pays for.
//!
//! Run: `cd bench && cargo run --release`
//!
//! Caveats: wall-clock numbers on the current machine; each editor
//! paints its own prompt decoration, so byte counts are comparable but
//! not identical in content. Treat small deltas as noise — the signal is
//! orders of magnitude and scaling shape.

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Output is considered quiescent after this long with no bytes.
const QUIET: Duration = Duration::from_millis(150);
/// Per-key quiescence for the paced scenario.
const KEY_QUIET: Duration = Duration::from_millis(12);

struct Term {
    master: OwnedFd,
    file: std::fs::File,
    /// Tail bytes of the previous chunk, so a cursor-position query split
    /// across reads is still detected (and none is counted twice).
    carry: Vec<u8>,
}

impl Term {
    fn spawn(bin: &str) -> (Term, Child) {
        let mut master: libc::c_int = 0;
        let mut slave: libc::c_int = 0;
        let mut ws = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut ws,
            )
        };
        assert_eq!(rc, 0, "openpty failed");
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        let mut path = std::env::current_exe().expect("current_exe");
        path.pop();
        path.push(bin);
        let child = Command::new(&path)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {}: {e}", path.display()));
        let file = std::fs::File::from(master.try_clone().unwrap());
        (
            Term {
                master,
                file,
                carry: Vec::new(),
            },
            child,
        )
    }

    /// Poll up to `timeout_ms` for output; append it to `out`, answering
    /// any cursor-position query seen. Returns bytes read (0 = nothing,
    /// -1 = EOF/EIO).
    fn pump(&mut self, timeout_ms: i32, out: &mut Vec<u8>) -> isize {
        let mut pfd = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if n <= 0 || pfd.revents & libc::POLLIN == 0 {
            return 0;
        }
        let mut buf = [0u8; 65536];
        let r = unsafe { libc::read(self.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if r <= 0 {
            return -1;
        }
        let chunk = &buf[..r as usize];
        out.extend_from_slice(chunk);
        // Answer ESC[6n cursor-position queries (scan carry + chunk; the
        // carry is < 4 bytes, so completed queries are never recounted).
        let mut scan = std::mem::take(&mut self.carry);
        scan.extend_from_slice(chunk);
        let queries = scan.windows(4).filter(|w| w == b"\x1b[6n").count();
        for _ in 0..queries {
            let _ = self.file.write_all(b"\x1b[1;1R");
        }
        if queries > 0 {
            let _ = self.file.flush();
        }
        self.carry = scan[scan.len().saturating_sub(3)..].to_vec();
        r
    }

    /// Read until the pty has been silent for `QUIET`; returns the bytes
    /// and the instant the last byte arrived (the quiet window itself
    /// never counts toward a measurement).
    fn drain_quiet(&mut self) -> (Vec<u8>, Instant) {
        let mut out = Vec::new();
        let mut last = Instant::now();
        loop {
            match self.pump(25, &mut out) {
                r if r > 0 => last = Instant::now(),
                0 if last.elapsed() > QUIET => break,
                0 => {}
                _ => break,
            }
        }
        (out, last)
    }

    /// Write `bytes`, measure time to the last output byte.
    fn feed(&mut self, bytes: &[u8]) -> Measure {
        let t0 = Instant::now();
        self.file.write_all(bytes).unwrap();
        self.file.flush().unwrap();
        let (out, last) = self.drain_quiet();
        Measure {
            secs: last
                .checked_duration_since(t0)
                .unwrap_or_default()
                .as_secs_f64(),
            bytes: out.len(),
        }
    }

    /// One key at a time, waiting for each repaint to settle: per-key
    /// latency (write → last byte of that key's repaint) and painted
    /// bytes per key.
    fn feed_paced(&mut self, key: u8, n: usize) -> (f64, usize) {
        let mut busy = Duration::ZERO;
        let mut bytes = 0usize;
        for _ in 0..n {
            let t0 = Instant::now();
            self.file.write_all(&[key]).unwrap();
            self.file.flush().unwrap();
            let mut out = Vec::new();
            let mut last = t0;
            let mut saw_output = false;
            loop {
                match self.pump(2, &mut out) {
                    r if r > 0 => {
                        last = Instant::now();
                        saw_output = true;
                    }
                    0 if last.elapsed() > KEY_QUIET => break,
                    0 => {}
                    _ => break,
                }
            }
            if saw_output {
                busy += last - t0;
            }
            bytes += out.len();
        }
        (busy.as_secs_f64() * 1e6 / n as f64, bytes / n)
    }
}

struct Measure {
    secs: f64,
    bytes: usize,
}

struct Row {
    name: &'static str,
    paced_us: f64,
    paced_bytes: usize,
    burst_us: f64,
    edit_us: f64,
    edit_bytes_per_key: usize,
    paste_ms: f64,
    hist_us: f64,
}

fn run(name: &'static str, bin: &str) -> Row {
    let (mut term, mut child) = Term::spawn(bin);
    term.drain_quiet(); // initial prompt paint

    // Paced keystrokes at a short line: true per-key repaint latency.
    let (paced_us, paced_bytes) = term.feed_paced(b'p', 200);
    term.feed(b"\x03"); // abandon the line
    // Burst: 1000 keystrokes in one write.
    let burst = term.feed("a".repeat(1000).as_bytes());
    // 200 more keystrokes editing at a ~1200-char line.
    let edit = term.feed("b".repeat(200).as_bytes());
    // One 20 KB bracketed paste on top.
    let paste = term.feed(format!("\x1b[200~{}\x1b[201~", "x".repeat(20_000)).as_bytes());
    term.feed(b"\x03");
    // History: 30 short lines, then 100 Up + 100 Down.
    let lines: String = (0..30).map(|i| format!("command-{i}\r")).collect();
    term.feed(lines.as_bytes());
    let arrows = format!("{}{}", "\x1b[A".repeat(100), "\x1b[B".repeat(100));
    let hist = term.feed(arrows.as_bytes());
    term.feed(b"\x03");
    let _ = term.file.write_all(b"\x04");
    let _ = term.file.flush();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            _ if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }

    Row {
        name,
        paced_us,
        paced_bytes,
        burst_us: burst.secs * 1e6 / 1000.0,
        edit_us: edit.secs * 1e6 / 200.0,
        edit_bytes_per_key: edit.bytes / 200,
        paste_ms: paste.secs * 1e3,
        hist_us: hist.secs * 1e6 / 200.0,
    }
}

fn main() {
    println!("editor-bench: identical byte streams under an 80x24 pty");
    println!("paced = per-key latency (write -> last repaint byte); burst = throughput\n");
    let rows = [
        run("rusty_lines", "ed_rusty"),
        run("rustyline", "ed_rustyline"),
        run("reedline", "ed_reedline"),
    ];
    println!(
        "| {:<12} | {:>14} | {:>9} | {:>14} | {:>16} | {:>12} | {:>13} | {:>15} |",
        "editor",
        "paced (µs/key)",
        "bytes/key",
        "burst (µs/key)",
        "edit@1k (µs/key)",
        "bytes/key@1k",
        "20KB paste ms",
        "hist (µs/arrow)"
    );
    for r in rows {
        println!(
            "| {:<12} | {:>14.0} | {:>9} | {:>14.1} | {:>16.1} | {:>12} | {:>13.1} | {:>15.1} |",
            r.name,
            r.paced_us,
            r.paced_bytes,
            r.burst_us,
            r.edit_us,
            r.edit_bytes_per_key,
            r.paste_ms,
            r.hist_us
        );
    }
    println!(
        "\nNotes: reedline's numbers include its per-repaint ESC[6n cursor query\n\
         round-trip (its real-terminal behavior). rustyline/readline coalesce\n\
         repaints while input is pending, which dominates the burst columns."
    );
}
