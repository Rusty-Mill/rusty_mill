//! `OwnPty` end to end on Windows: real clients on a console we host,
//! and the parity probes under ConPTY and under our host (design §8.1).
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use platform::error::ErrorKind;
use platform::process::{Child, Command, ExitStatus, GroupSpec, Spawner as _};
use platform::pty::PtyMaster;
use platform::term::WinSize;
use platform_windows::{WindowsChild, WindowsPty};
use rusty_conhost::{OwnPty, OwnPtyMaster};

const SIZE: WinSize = WinSize {
    rows: 30,
    cols: 200,
};
/// How long any one test waits for output or an exit.
const BUDGET: Duration = Duration::from_secs(30);

/// Serializes the tests: each owns a console and a child.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn command(program: &str, args: &[&str]) -> Command {
    let cwd = std::env::current_dir().expect("current_dir");
    let mut cmd = Command::new(program, cwd);
    cmd.argv = args.iter().map(Into::into).collect();
    cmd
}

/// The `conhost_probe` example, which `cargo test` builds next to this
/// binary. Example names share one directory across the workspace, hence
/// the prefix.
fn probe() -> String {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().and_then(|d| d.parent()).expect("target dir");
    let path: PathBuf = dir.join("examples").join("conhost_probe.exe");
    // `cargo test --test own_pty` builds no examples, so an old probe
    // would answer for new probes.
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/conhost_probe");
    let newest = std::fs::read_dir(&sources)
        .expect("the probe's sources")
        .filter_map(|e| modified(&e.ok()?.path()))
        .max();
    assert!(
        modified(&path) >= newest,
        "build the conhost_probe example (cargo build -p rusty_conhost --example \
         conhost_probe): {} is missing or older than its sources",
        path.display()
    );
    path.to_string_lossy().into_owned()
}

/// Read until EOF on another thread; `None` past the budget.
fn read_all<M: PtyMaster + Send + 'static>(master: M) -> Option<(Vec<u8>, M)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match master.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        let _ = tx.send((out, master));
    });
    rx.recv_timeout(BUDGET).ok()
}

/// Text with escape sequences removed, as lines.
fn plain_lines(vt: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(vt);
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters up to a final byte.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ST.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    plain.lines().map(|l| l.trim().to_owned()).collect()
}

/// Which console a probe runs on.
#[derive(Clone, Copy, Debug)]
enum Host {
    ConPty,
    Own,
}

/// What precedes a probe's result line.
const RESULT: &str = "#> ";

/// Run the probe with `args` on `host` and return its result line.
fn probe_line(host: Host, args: &[&str]) -> String {
    let cmd = command(&probe(), args);
    let (out, child) = match host {
        Host::ConPty => {
            let (master, child) = WindowsPty.spawn_concrete(&cmd, SIZE).expect("ConPTY spawn");
            (read_all(master).map(|(out, _)| out), child)
        }
        Host::Own => {
            let (master, child) = own(&cmd);
            (read_all(master).map(|(out, _)| out), child)
        }
    };
    let out = out.unwrap_or_else(|| panic!("{host:?}: no EOF within the budget"));
    assert_eq!(Box::new(child).wait().expect("wait"), ExitStatus::Code(0));
    let head = format!("{RESULT}{} ", args[0]);
    plain_lines(&out)
        .iter()
        .find_map(|l| Some(l[l.find(&head)? + RESULT.len()..].to_owned()))
        .unwrap_or_else(|| {
            panic!(
                "{host:?}: no {head:?} line in {:?}",
                String::from_utf8_lossy(&out)
            )
        })
}

fn own(cmd: &Command) -> (OwnPtyMaster, WindowsChild) {
    OwnPty.spawn_concrete(cmd, SIZE).expect("OwnPty::spawn")
}

#[test]
fn cmd_c_echo_prints_and_reaches_eof_once() {
    let _g = lock();
    let (master, child) = own(&command("cmd.exe", &["/c", "echo hi"]));
    let (out, master) = read_all(master).expect("EOF within the budget");
    assert!(
        plain_lines(&out).iter().any(|l| l == "hi"),
        "{:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(Box::new(child).wait().expect("wait"), ExitStatus::Code(0));
    let mut buf = [0u8; 16];
    assert_eq!(master.read(&mut buf).expect("read after EOF"), 0);
}

/// A console's output as it arrives, read on another thread.
struct Stream {
    rx: mpsc::Receiver<Vec<u8>>,
    out: Vec<u8>,
}

impl Stream {
    fn new(master: Arc<OwnPtyMaster>) -> Stream {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = master.read(&mut buf) {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Stream {
            rx,
            out: Vec::new(),
        }
    }

    /// Read until a plain line contains `text`; panics past the budget.
    fn until(&mut self, text: &str) {
        let deadline = Instant::now() + BUDGET;
        while !plain_lines(&self.out).iter().any(|l| l.contains(text)) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(chunk) => self.out.extend(chunk),
                Err(_) => panic!("no {text:?} in {:?}", String::from_utf8_lossy(&self.out)),
            }
        }
    }
}

/// A child that is killed, with its children where it has a job, if the
/// test fails before it exits, so a failing test leaves nothing running.
struct Reaped(WindowsChild);

impl Drop for Reaped {
    fn drop(&mut self) {
        use platform::process::Signal;
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        // The tree too, where there is a job: cmd's own children.
        if self.0.kill_tree(Signal::Kill).is_err() {
            let _ = self.0.kill_single(Signal::Kill);
        }
    }
}

/// Wait for `child` to exit within the budget.
fn exit_of(child: &mut Reaped) -> ExitStatus {
    let deadline = Instant::now() + BUDGET;
    loop {
        if let Some(status) = child.0.try_wait().expect("try_wait") {
            return status;
        }
        assert!(Instant::now() < deadline, "no exit within the budget");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn interactive_cmd_runs_typed_lines() {
    let _g = lock();
    let (master, child) = own(&command("cmd.exe", &["/d", "/q"]));
    let mut child = Reaped(child);
    let master = Arc::new(master);
    let mut stream = Stream::new(Arc::clone(&master));
    // Both lines at once: the second waits on the handle for cmd's next
    // read. The caret keeps the echoed line from matching the output.
    master.write(b"echo ho^ld\rexit 7\r").expect("write");
    stream.until("hold");
    assert_eq!(exit_of(&mut child), ExitStatus::Code(7));
}

/// `chcp 850` (§8.3): cmd sets both code pages, and a raw write's bytes
/// are then read as 850; an uninstalled code page is refused.
#[test]
fn chcp_changes_how_a_raw_write_reads() {
    let _g = lock();
    let (master, child) = own(&command("cmd.exe", &["/d", "/q"]));
    let mut child = Reaped(child);
    let master = Arc::new(master);
    let mut stream = Stream::new(Arc::clone(&master));
    let typed = format!("chcp 850\r\"{}\" raw 9b\rchcp 1\rexit 5\r", probe());
    master.write(typed.as_bytes()).expect("write");
    stream.until("Active code page: 850");
    stream.until("\u{f8}");
    stream.until("Invalid code page");
    assert_eq!(exit_of(&mut child), ExitStatus::Code(5));
}

/// Clear this test process's "ignore Ctrl+C" flag, which children
/// inherit: nextest and the CI runner start tests with it set. The host
/// leaves the caller's policy alone, as `spawn_attached` does, so a
/// caller that wants its children to see Ctrl+C opts in itself.
#[allow(unsafe_code)]
fn accept_ctrl_c() {
    use platform_windows::ffi::win32_surface as w;
    // SAFETY: a null handler only sets this process's Ctrl+C flag.
    let ok = unsafe { w::SetConsoleCtrlHandler(None, 0) };
    assert_ne!(ok, 0, "SetConsoleCtrlHandler(NULL, FALSE)");
}

#[test]
fn ctrl_c_stops_the_running_command_not_the_shell() {
    let _g = lock();
    accept_ctrl_c();
    let (master, child) = own(&command("cmd.exe", &["/d", "/q"]));
    let mut child = Reaped(child);
    let master = Arc::new(master);
    let mut stream = Stream::new(Arc::clone(&master));
    master.write(b"ping -n 60 127.0.0.1\r").expect("write");
    stream.until("Reply from 127.0.0.1");
    master.write(b"\x03").expect("Ctrl+C");
    stream.until("Control-C");
    master.write(b"exit 3\r").expect("write");
    assert_eq!(exit_of(&mut child), ExitStatus::Code(3));
}

/// The parity suite (§8.1, the first Phase B criterion in §9.2): every
/// API our host serves, and those it doesn't yet, each called by a real
/// client through kernelbase under ConPTY and under our host. Probes take
/// `conhost_probe`'s escapes (`\e`, `\s`, ...).
fn parity_probes() -> Vec<Vec<String>> {
    const FIXED: &[&[&str]] = &[
        // Modes.
        &["modes"],
        &["set-mode", "in", "1e4"],
        &["set-mode", "in", "400"],
        &["set-mode", "in", "0"],
        &["set-mode", "in", "f7"],
        &["set-mode", "out", "20"],
        &["set-mode", "out", "5"],
        &["set-mode", "out", "7"],
        &["set-mode", "out", "1f"],
        // Code pages.
        &["cp"],
        &["set-cp", "65001"],
        &["set-cp", "437"],
        &["set-cp", "850"],
        &["set-cp", "1"],
        &["out-cp"],
        &["set-out-cp", "65001"],
        &["set-out-cp", "437"],
        &["set-out-cp", "1"],
        // WriteConsoleW: controls, wrap and VT, by output mode.
        &["print", "3", "ab\\tc"],
        &["print", "7", "ab\\tc"],
        &["print", "3", "abc\\rd"],
        &["print", "3", "a\\nb"],
        &["print", "7", "a\\nb"],
        &["print", "f", "a\\nb"],
        &["print", "3", "\\bx"],
        &["print", "3", "ab\\b\\bx"],
        &["print", "3", "a\\x07b"],
        &["print", "3", "\\e[31m"],
        &["print", "1", "\\e[31m"],
        &["print", "7", "\\e[5;10H"],
        &["print", "7", "\\e[31;44m"],
        &["print", "7", "\\e[1;7m"],
        &["print", "7", "\\e[4m"],
        &["print", "7", "\\e[38;2;1;2;3mx"],
        &["print", "7", "ab\\e7\\e[5;5H\\e8c"],
        &["print", "7", "\\e[3;7r\\e[H\\e[99B"],
        &["print", "7", "\\u{65e5}\\u{672c}"],
        &["print", "3", "\\u{65e5}\\u{672c}"],
        &["print", "7", "\\e[53m"],
        &["print-a", "437", "41420d0a43"],
        &["print-a", "437", "e9"],
        &["print-a", "65001", "c3a9"],
        &["print-a", "65001", "e9"],
        &["print-a", "850", "e9"],
        // A row exactly full, one past it, and one past it with wrap off.
        &["print-x", "3", "200"],
        &["print-x", "7", "200"],
        &["print-x", "3", "205"],
        &["print-x", "1", "205"],
        // One write at the 64 KiB packet cap and past it.
        &["print-x", "7", "32768"],
        &["print-x", "7", "100000"],
        // The screen buffer.
        &["sbi"],
        &["sbi-ex"],
        &["colors"],
        &["cursor-info"],
        &["set-cursor-info", "50", "1"],
        &["set-cursor-info", "100", "0"],
        &["set-cursor-info", "0", "1"],
        &["set-cursor-info", "101", "1"],
        &["set-cursor", "5", "3"],
        &["set-cursor", "199", "29"],
        &["set-cursor", "200", "0"],
        &["set-cursor", "0", "30"],
        &["set-cursor", "-1", "0"],
        &["set-attr", "1e"],
        &["set-attr", "8007"],
        &["set-attr", "ffff"],
        &["set-attr", "407"],
        &["set-attr", "1c4f"],
        // Legacy gridlines under VT: only a rendition reset clears them.
        &["attr-then", "807", "7", "text"],
        &["attr-then", "807", "3", "text"],
        &["attr-then", "807", "7", "\\e[0m"],
        &["attr-then", "807", "7", "\\e[m"],
        &["attr-then", "807", "7", "\\e[!p"],
        &["attr-then", "807", "7", "\\e[38;5;0m"],
        &["attr-then", "8007", "7", "\\e[24m"],
        &["attr-then", "181e", "7", "\\e[31m"],
        &["attr-then", "181e", "7", "\\e[0m\\e[93;44m"],
        &["attr-then", "407", "7", "\\e[55m"],
        &["set-buffer", "200", "30"],
        &["set-buffer", "200", "9001"],
        &["set-buffer", "100", "30"],
        &["set-window", "1", "0", "0", "199", "29"],
        &["set-window", "1", "0", "0", "99", "19"],
        &["set-window", "0", "0", "0", "0", "0"],
        &["set-window", "1", "0", "0", "300", "29"],
        &["largest"],
        &["font"],
        &["window"],
        // Cells.
        &["fill", "x", "1e", "10", "2", "1"],
        &["fill", "x", "1e", "500", "190", "29"],
        &["fill", "x", "1e", "0", "0", "0"],
        &["fill", "x", "1e", "5", "200", "0"],
        &["cells", "abc", "1e,2f,4e", "3", "2"],
        &["cells", "abcd", "1e,2f,4e,70", "198", "0"],
        &["cells", "abcd", "1e,2f,4e,70", "198", "29"],
        &["block"],
        &["scroll", "0"],
        &["scroll", "1"],
        // Titles and processes.
        &["title"],
        &["set-title", "hello\\sworld"],
        &["set-title", ""],
        &["procs"],
        // Input and control events.
        &["input"],
        &["cooked", "hi\\r"],
        &["cooked", "ab\\bc\\r"],
        &["ctrl-c", "0"],
        &["ctrl-c", "4242"],
    ];
    FIXED
        .iter()
        .map(|args| args.iter().map(|a| (*a).to_owned()).collect())
        .collect()
}

/// Answers that differ from ConPTY by design (§5.10), as (probe, ours).
const DIVERGENCES: &[(&[&str], &str)] = &[
    // V4: both code pages start at 65001 where ConPTY starts at the OEM
    // one, which a refused set leaves in place.
    (&["cp"], "cp in=65001"),
    (&["set-cp", "1"], "set-cp error=87 after=65001"),
    (&["out-cp"], "out-cp out=65001"),
    (&["set-out-cp", "1"], "set-out-cp error=87 after=65001"),
    // V5: no console window.
    (&["window"], "window null=1"),
];

/// Gaps the suite has found and that are tracked but not yet closed, as
/// (probe, our answer, issue). Our answer is pinned, as a divergence's
/// is, so the rest of the probe still catches regressions: a gap whose
/// answer changes in any other way is a mismatch, and one that starts to
/// agree fails the suite until it is taken off this list.
const GAPS: &[(&[&str], &str, &str)] = &[
    (&["fill", "x", "1e", "10", "2", "1"], NO_FILL, "#1330"),
    (&["fill", "x", "1e", "500", "190", "29"], NO_FILL, "#1330"),
    (&["fill", "x", "1e", "0", "0", "0"], NO_FILL, "#1330"),
    (&["fill", "x", "1e", "5", "200", "0"], NO_FILL, "#1330"),
    (&["scroll", "0"], NO_SCROLL, "#1330"),
    (&["scroll", "1"], NO_SCROLL, "#1330"),
    (&["cells", "abc", "1e,2f,4e", "3", "2"], NO_CELLS, "#1333"),
    (
        &["cells", "abcd", "1e,2f,4e,70", "198", "0"],
        NO_CELLS,
        "#1333",
    ),
    (
        &["cells", "abcd", "1e,2f,4e,70", "198", "29"],
        NO_CELLS,
        "#1333",
    ),
    (&["block"], "block wrote=error=1 read=error=1", "#1333"),
    (&["set-buffer", "200", "30"], NO_BUFFER, "#1333"),
    (&["set-buffer", "200", "9001"], NO_BUFFER, "#1333"),
    (&["set-buffer", "100", "30"], NO_BUFFER, "#1333"),
    (
        &["set-window", "1", "0", "0", "199", "29"],
        NO_WINDOW,
        "#1333",
    ),
    (
        &["set-window", "1", "0", "0", "99", "19"],
        NO_WINDOW,
        "#1333",
    ),
    (&["set-window", "0", "0", "0", "0", "0"], NO_WINDOW, "#1333"),
    (
        &["set-window", "1", "0", "0", "300", "29"],
        NO_WINDOW,
        "#1333",
    ),
    (&["largest"], "largest size=error=1", "#1333"),
    (
        &["input"],
        "input before=0 wrote=error=1 queued=0 peek=0: read=0: left=0 flush=ok after=0",
        "#1335",
    ),
    (&["cooked", "hi\\r"], NO_COOKED, "#1335"),
    (&["cooked", "ab\\bc\\r"], NO_COOKED, "#1335"),
    (
        &["print-x", "1", "205"],
        "print-x mode=ok n=205 cursor=199,0 attr=0x7",
        "#1336",
    ),
    (&["print-x", "7", "32768"], NO_BIG_WRITE, "#1337"),
    (&["print-x", "7", "100000"], NO_BIG_WRITE, "#1337"),
];

// Our answers while those APIs are unserved (ERROR_INVALID_FUNCTION).
const NO_FILL: &str = "fill chars=error=1 attrs=error=1 text=error=1 attrs=error=1";
const NO_SCROLL: &str = "scroll painted=error=1 error=1 row0=[text=error=1 attrs=error=1] row3=[text=error=1 attrs=error=1]";
const NO_CELLS: &str = "cells chars=error=1 attrs=error=1 text=error=1 attrs=error=1";
const NO_BUFFER: &str =
    "set-buffer error=1 after=size=200,30 cursor=0,0 attr=0x7 window=0,0,199,29 max=200,30";
const NO_WINDOW: &str =
    "set-window error=1 after=size=200,30 cursor=0,0 attr=0x7 window=0,0,199,29 max=200,30";
const NO_COOKED: &str = "cooked wrote=error=1 read= cursor=0,0 attr=0x7";
const NO_BIG_WRITE: &str = "print-x mode=ok n=error=6 cursor=0,0 attr=0x7";

/// How one probe compared.
#[derive(Debug, PartialEq)]
enum Verdict {
    Same,
    Divergence,
    Gap(&'static str),
    /// A tracked gap that agrees now.
    Closed(&'static str),
    Mismatch,
}

fn verdict(args: &[String], conpty: &str, ours: &str) -> Verdict {
    let is = |listed: &[&str]| listed.iter().copied().eq(args.iter().map(String::as_str));
    let gap = GAPS.iter().find(|(p, ..)| is(p));
    let diverges = || DIVERGENCES.iter().any(|(p, o)| is(p) && *o == ours);
    match (conpty == ours, gap) {
        (true, None) => Verdict::Same,
        (true, Some(&(_, _, issue))) => Verdict::Closed(issue),
        (false, Some(&(_, pinned, issue))) if pinned == ours => Verdict::Gap(issue),
        (false, Some(_)) => Verdict::Mismatch,
        (false, None) if diverges() => Verdict::Divergence,
        (false, None) => Verdict::Mismatch,
    }
}

#[test]
fn parity_suite_agrees_with_conpty() {
    let _g = lock();
    let mut failures = Vec::new();
    for args in parity_probes() {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let conpty = probe_line(Host::ConPty, &argv);
        let ours = probe_line(Host::Own, &argv);
        let v = verdict(&args, &conpty, &ours);
        // The whole matrix, for `--nocapture`.
        eprintln!("{v:?}\t{args:?}\n  conpty {conpty}\n  ours   {ours}");
        if matches!(v, Verdict::Mismatch | Verdict::Closed(_)) {
            failures.push(format!("{v:?} {args:?}: ConPTY {conpty:?}, ours {ours:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_reader_that_stalls_holds_the_writer_back() {
    let _g = lock();
    let total = 4 * 1024 * 1024;
    let (master, mut child) = own(&command(&probe(), &["write", &total.to_string()]));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        child.try_wait().expect("try_wait"),
        None,
        "the writer finished without a reader"
    );
    let (out, _master) = read_all(master).expect("EOF within the budget");
    assert_eq!(out.iter().filter(|b| **b == b'x').count(), total);
    assert_eq!(Box::new(child).wait().expect("wait"), ExitStatus::Code(0));
}

#[test]
fn hang_up_closes_the_console_and_releases_waiters() {
    let _g = lock();
    let (master, child) = own(&command("cmd.exe", &["/c", "ping -n 60 127.0.0.1"]));
    let mut child = Reaped(child);
    let waiter = master.exit_waiter();
    let started = Instant::now();
    master.hang_up();
    master.hang_up();
    let (_, master) = read_all(master).expect("EOF after hang_up");
    waiter.wait();
    assert!(started.elapsed() < Duration::from_secs(5));
    let err = master.write(b"x").expect_err("write after hang_up");
    assert_eq!(err.kind, ErrorKind::BrokenPipe);
    // CTRL_CLOSE ended cmd and ping before the deadline.
    assert_ne!(exit_of(&mut child), ExitStatus::Code(0));
}

/// The grace (§7.3): 2 s from hang-up to the server closing.
const GRACE: Duration = Duration::from_secs(2);

/// Spawn the `ctrl-hold` probe on our host and wait for it to be ready.
fn holding(ms: u64) -> (Arc<OwnPtyMaster>, Stream, Reaped) {
    let (master, child) = own(&command(&probe(), &["ctrl-hold", &ms.to_string()]));
    let master = Arc::new(master);
    let mut stream = Stream::new(Arc::clone(&master));
    stream.until("ready");
    (master, stream, Reaped(child))
}

#[test]
fn hang_up_sends_ctrl_close_and_serves_the_handler_until_it_exits() {
    let _g = lock();
    let (master, mut stream, mut child) = holding(300);
    let started = Instant::now();
    master.hang_up();
    // The handler's own write is served while the console closes.
    stream.until("ctrl-close");
    assert_ne!(exit_of(&mut child), ExitStatus::Code(0));
    let err = master.write(b"x").expect_err("write after hang_up");
    assert_eq!(err.kind, ErrorKind::BrokenPipe);
    master.exit_waiter().wait();
    assert!(started.elapsed() < BUDGET);
}

#[test]
fn a_client_that_holds_ctrl_close_is_cut_off_at_the_deadline() {
    let _g = lock();
    let (master, mut stream, _child) = holding(60_000);
    let started = Instant::now();
    master.hang_up();
    stream.until("ctrl-close");
    // EOF comes at the deadline, not when the handler returns.
    let deadline = Instant::now() + BUDGET;
    while stream.rx.recv_timeout(deadline - Instant::now()).is_ok() {}
    let took = started.elapsed();
    assert!(took >= GRACE, "closed after {took:?}, before the grace");
    assert!(
        took < GRACE + Duration::from_secs(3),
        "closed after {took:?}"
    );
    let mut buf = [0u8; 8];
    assert_eq!(master.read(&mut buf).expect("EOF"), 0);
    assert_eq!(master.read(&mut buf).expect("EOF again"), 0);
}

/// `STATUS_CONTROL_C_EXIT`, the code CTRL_CLOSE ends a process with.
const CONTROL_C_EXIT: i32 = 0xC000_013A_u32 as i32;
/// When the host ends a client still attached at the close: 5 s from
/// CTRL_CLOSE, Windows' own bound on a handler (#1259).
const KILL_AFTER: Duration = Duration::from_secs(5);
/// Slack on the host's and Windows' timing.
const SLACK: Duration = Duration::from_millis(1500);

#[test]
fn a_client_holding_ctrl_close_past_the_grace_is_ended_by_the_kill_bound() {
    let _g = lock();
    let (master, mut stream, mut child) = holding(60_000);
    let started = Instant::now();
    master.hang_up();
    stream.until("ctrl-close");
    let status = exit_of(&mut child);
    let took = started.elapsed();
    assert!(took >= GRACE, "ended after {took:?}, before the grace");
    assert!(took < KILL_AFTER + SLACK, "ended after {took:?}");
    assert_eq!(status, ExitStatus::Code(CONTROL_C_EXIT));
}

/// Every `ready pid=<pid>` line so far.
fn ready_pids(stream: &Stream) -> Vec<u32> {
    plain_lines(&stream.out)
        .iter()
        .filter_map(|l| l.strip_prefix("ready pid=")?.trim().parse().ok())
        .collect()
}

/// A process by pid, opened while it is known to run and ended on drop
/// if a test fails before it exits: adopted into a kill-on-close job. The
/// held handle keeps the pid from being reused, so the drop never reaches
/// another process.
struct ReapedPid {
    pid: u32,
    _group: Box<dyn platform::process::GroupHandle>,
}

impl ReapedPid {
    fn open(pid: u32) -> ReapedPid {
        let group = platform_windows::WindowsSpawner
            .adopt(pid)
            .expect("adopt the probe");
        ReapedPid { pid, _group: group }
    }
}

#[test]
fn grandchildren_still_attached_are_ended_together_at_the_kill_bound() {
    let _g = lock();
    // cmd starts two probes on the same console, waits about a second so
    // their handlers are in place, and exits. CTRL_CLOSE then reaches one
    // probe, whose held handler blocks the delivery to the other until
    // Windows ends it at about 5 s; the other would hold to about 10 s.
    // The host ends both at 5 s (#1259).
    let probe = probe();
    assert!(
        !probe.contains(' '),
        "start needs an unquoted path: {probe}"
    );
    let hold = format!("start /b {probe} ctrl-hold 60000");
    let tail = format!("{hold} & {hold} & ping -n 2 127.0.0.1 >nul");
    let cmd = command("cmd.exe", &["/c", &tail]);
    let (master, child) = own(&cmd);
    let master = Arc::new(master);
    let mut stream = Stream::new(Arc::clone(&master));
    let mut child = Reaped(child);
    let deadline = Instant::now() + BUDGET;
    while ready_pids(&stream).len() < 2 {
        let left = deadline.saturating_duration_since(Instant::now());
        let chunk = stream.rx.recv_timeout(left).expect("two ready probes");
        stream.out.extend(chunk);
    }
    // Opened while the probes wait for CTRL_CLOSE, so they are running.
    let probes: Vec<ReapedPid> = ready_pids(&stream)
        .into_iter()
        .map(ReapedPid::open)
        .collect();
    exit_of(&mut child);
    let exited = Instant::now();
    // What a process that exits right after closing relies on: once the
    // shutdown latch is set, the probes have been ended.
    assert!(master.wait_shut_down(BUDGET), "no shutdown");
    let spawner = platform_windows::WindowsSpawner;
    for p in &probes {
        // TerminateProcess doesn't wait; the exit follows at once.
        let ended = Instant::now();
        while spawner.is_alive(p.pid).expect("is_alive") {
            assert!(
                ended.elapsed() < Duration::from_millis(500),
                "probe {} runs on after the shutdown",
                p.pid
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let took = exited.elapsed();
    assert!(
        took < KILL_AFTER + SLACK,
        "the last ended {took:?} after the child's exit, not at the kill bound"
    );
}

#[test]
fn eof_comes_once_to_readers_on_two_threads() {
    let _g = lock();
    let (master, _stream, _child) = holding(60_000);
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let master = Arc::clone(&master);
            std::thread::spawn(move || {
                let mut buf = [0u8; 64];
                while master.read(&mut buf).expect("read") > 0 {}
                master.read(&mut buf).expect("read after EOF")
            })
        })
        .collect();
    master.hang_up();
    for r in readers {
        assert_eq!(r.join().expect("reader"), 0);
    }
}

#[test]
fn drop_during_the_grace_returns_at_once_and_the_console_closes() {
    let _g = lock();
    let (master, child) = own(&command(&probe(), &["ctrl-hold", "60000"]));
    let _child = Reaped(child);
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    while !plain_lines(&out).iter().any(|l| l.starts_with("ready")) {
        let n = master.read(&mut buf).expect("read");
        assert!(n > 0, "EOF before ready");
        out.extend_from_slice(&buf[..n]);
    }
    let waiter = master.exit_waiter();
    master.hang_up();
    let started = Instant::now();
    // The last master: Drop detaches, as during the grace.
    drop(master);
    assert!(started.elapsed() < Duration::from_millis(500));
    waiter.wait();
    let took = started.elapsed();
    assert!(
        took < GRACE + Duration::from_secs(3),
        "torn down after {took:?}"
    );
}

#[test]
fn resize_checks_the_size() {
    let _g = lock();
    let (master, child) = own(&command("cmd.exe", &["/c", "ping -n 3 127.0.0.1"]));
    master
        .resize(WinSize {
            rows: 40,
            cols: 100,
        })
        .expect("resize");
    let err = master
        .resize(WinSize { rows: 0, cols: 100 })
        .expect_err("0 rows");
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    master.hang_up();
    let _ = child.kill_single(platform::process::Signal::Kill);
}

#[test]
fn spawn_refuses_what_a_fresh_console_cannot_do() {
    let _g = lock();
    let mut cmd = command("cmd.exe", &["/c", "echo no"]);
    let zero = WinSize { rows: 30, cols: 0 };
    let err = OwnPty.spawn_concrete(&cmd, zero).err().expect("0 columns");
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    cmd.group = GroupSpec::JoinGroup(1);
    let err = OwnPty.spawn_concrete(&cmd, SIZE).err().expect("JoinGroup");
    assert_eq!(err.kind, ErrorKind::InvalidInput);
}
