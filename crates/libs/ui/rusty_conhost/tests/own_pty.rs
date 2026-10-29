//! `OwnPty` end to end on Windows: real clients on a console we host,
//! and the parity probes under ConPTY and under our host (design §8.1).
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use platform::error::ErrorKind;
use platform::process::{Child, Command, ExitStatus, GroupSpec};
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
    assert!(
        path.exists(),
        "build the conhost_probe example: {}",
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
    plain_lines(&out)
        .into_iter()
        .find(|l| l.starts_with(args[0]))
        .unwrap_or_else(|| {
            panic!(
                "{host:?}: no {:?} line in {:?}",
                args[0],
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

/// Answers that differ from ConPTY by design (§5.10), as (probe, ours).
/// V4: only 65001 until code pages land, where ConPTY reports the OEM
/// code page.
const DIVERGENCES: &[(&[&str], &str)] = &[(&["cp"], "cp in=65001")];

#[test]
fn probes_agree_with_conpty() {
    let _g = lock();
    let probe_args: &[&[&str]] = &[
        &["modes"],
        &["set-mode", "in", "1e4"],
        &["set-mode", "in", "400"],
        &["set-mode", "out", "20"],
        &["set-mode", "out", "5"],
        &["cp"],
        &["set-cp", "65001"],
    ];
    let mismatches: Vec<String> = probe_args
        .iter()
        .filter_map(|args| {
            let conpty = probe_line(Host::ConPty, args);
            let ours = probe_line(Host::Own, args);
            let diverges = DIVERGENCES.contains(&(args, ours.as_str()));
            (conpty != ours && !diverges)
                .then(|| format!("{args:?}: ConPTY {conpty:?}, ours {ours:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
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
    let waiter = master.exit_waiter();
    let started = Instant::now();
    master.hang_up();
    master.hang_up();
    let (_, master) = read_all(master).expect("EOF after hang_up");
    waiter.wait();
    assert!(started.elapsed() < Duration::from_secs(5));
    let err = master.write(b"x").expect_err("write after hang_up");
    assert_eq!(err.kind, ErrorKind::BrokenPipe);
    let _ = child.kill_single(platform::process::Signal::Kill);
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
