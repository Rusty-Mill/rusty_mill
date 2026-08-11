//! Live, cross-process regression test for the `unix_listen` stale-reclaim
//! race documented in `sys::net::is_stale_bind_candidate` and
//! `docs/decision-request-af-unix-stale-reclaim-race.md`: a listener whose
//! owning process is force-killed (`TerminateProcess`, not a graceful
//! close) while it also holds a second, live `AF_UNIX` connection open to
//! an *unrelated* path must still be cleanly reclaimable by a fresh
//! `unix_listen` on its own path — traced from a real failure in the
//! `rusty_prime_agent` daemon-restart harness (bogus `os error 0` on
//! rebind), reproducible only with that second live connection present.
//!
//! Only actually executes on CI's `windows-latest` leg, same discipline as
//! `pty.rs`/`console_acquisition.rs`: this crate's whole backend is
//! developed from a Linux host against
//! `cargo check --target x86_64-pc-windows-gnu`, so nothing here has run
//! outside CI as of this writing. Every blocking call is bounded for the
//! same reason `pty.rs`'s own module doc gives — an unbounded wait turning
//! a real hang into an opaque multi-minute CI stall is worse than a clear
//! test failure.
//!
//! This binary re-execs itself (`std::env::current_exe`) to play the
//! "about to be force-killed" role in a genuinely separate process —
//! anything short of that (a second thread, a second socket closed within
//! this same process) doesn't exercise the actual mechanism under test,
//! which is specifically about how Windows tears down a *terminated
//! process's* whole socket table, not about closing one socket.

#![cfg(windows)]
#![allow(unsafe_code)] // force_kill below: one raw TerminateProcess call, its own SAFETY comment.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use platform_windows::{WindowsUnixListener, WindowsUnixStream};

/// Set in the helper's own environment so the re-exec'd process knows to
/// run [`run_as_helper`] instead of the parent-side assertions.
const HELPER_ENV: &str = "RUSTILS_STALE_RECLAIM_HELPER";
const PATH_ENV: &str = "RUSTILS_STALE_RECLAIM_PATH";
const PEER_ENV: &str = "RUSTILS_STALE_RECLAIM_PEER";

const HELPER_READY_BUDGET: Duration = Duration::from_secs(15);
const ACCEPT_BUDGET: Duration = Duration::from_secs(15);
const REBIND_BUDGET: Duration = Duration::from_secs(15);

#[test]
fn rebind_after_forced_kill_of_a_listener_that_also_held_an_outbound_connection() {
    if std::env::var_os(HELPER_ENV).is_some() {
        run_as_helper();
    }

    let dir = std::env::temp_dir().join(format!("rustils-stale-reclaim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test temp dir");
    let path = dir.join("owner.sock");
    let peer_path = dir.join("peer.sock");

    // The peer the dying process connects out to — owned by *this*
    // process throughout, so its lifetime never confounds the thing under
    // test (the owner-side listener's own reclaim).
    let peer_listener = WindowsUnixListener::bind(&peer_path).expect("bind peer listener");

    let mut helper = spawn_helper(&path, &peer_path);
    let mut helper_stdout =
        std::io::BufReader::new(helper.stdout.take().expect("helper stdout piped"));
    wait_for_ready_line(&mut helper_stdout, &mut helper);

    // Accept the helper's outbound connection so it's genuinely
    // established (not just sitting in the listen backlog) at the moment
    // of the kill — the repro's own precondition.
    accept_with_budget(&peer_listener, ACCEPT_BUDGET);

    force_kill(&helper);
    let _ = helper.wait();

    // The actual assertion: a fresh bind at `path` must reclaim it within
    // a bounded window, not surface a spurious failure (the harness saw
    // `os error 0`, but any error here is a regression).
    let deadline = Instant::now() + REBIND_BUDGET;
    let outcome = loop {
        match WindowsUnixListener::bind(&path) {
            Ok(listener) => break Ok(listener),
            Err(e) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
                let _ = e;
            }
            Err(e) => break Err(e),
        }
    };
    match outcome {
        Ok(listener) => drop(listener),
        Err(e) => panic!(
            "rebind after forced kill of a listener with a live outbound connection must \
             reclaim the stale path, got: {e:?}"
        ),
    }

    drop(peer_listener);
    let _ = std::fs::remove_dir_all(&dir);
}

fn spawn_helper(path: &Path, peer_path: &Path) -> Child {
    let exe = std::env::current_exe().expect("current_exe");
    Command::new(exe)
        .arg("rebind_after_forced_kill_of_a_listener_that_also_held_an_outbound_connection")
        .arg("--exact")
        .arg("--nocapture")
        .env(HELPER_ENV, "1")
        .env(PATH_ENV, path)
        .env(PEER_ENV, peer_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn helper process")
}

fn wait_for_ready_line(stdout: &mut impl BufRead, helper: &mut Child) {
    let deadline = Instant::now() + HELPER_READY_BUDGET;
    let mut line = String::new();
    loop {
        if let Some(status) = helper.try_wait().expect("try_wait helper") {
            panic!("helper exited before signaling ready: {status:?}");
        }
        if Instant::now() >= deadline {
            panic!("helper did not signal ready within {HELPER_READY_BUDGET:?}");
        }
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => continue,
            Ok(_) if line.trim() == "ready" => return,
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
}

fn accept_with_budget(listener: &WindowsUnixListener, budget: Duration) {
    // `accept` is a blocking call on this crate's own (non-reactor-driven)
    // sockets, same as every other `sys::net` primitive here — bounded by
    // spinning a short poll loop isn't available since there's no
    // non-blocking toggle exercised in this test; the helper is already
    // guaranteed to be mid-`connect` by the time its ready line printed,
    // so this should return immediately. `budget` only documents the
    // expectation, matching this file's own bounded-call discipline.
    let started = Instant::now();
    let result = listener.accept();
    assert!(
        started.elapsed() < budget,
        "accept took longer than the {budget:?} budget"
    );
    result.expect("accept the helper's outbound connection");
}

/// `TerminateProcess`, matching the harness's own `force_kill` — an
/// external, unowned-pid kill, not this crate's own graceful shutdown
/// path. The whole point of this test is the process-death teardown
/// timing, so nothing softer stands in for it.
fn force_kill(child: &Child) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: plain Win32 calls on a pid this process just spawned and
    // still owns; the handle is checked before use and closed on every
    // path that opened one.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, child.id());
        assert!(
            !handle.is_null(),
            "OpenProcess({}) for force_kill failed: {:?}",
            child.id(),
            std::io::Error::last_os_error()
        );
        let ok = TerminateProcess(handle, 1);
        let err = std::io::Error::last_os_error();
        CloseHandle(handle);
        assert!(ok != 0, "TerminateProcess({}) failed: {err:?}", child.id());
    }
}

/// The re-exec'd helper role: bind `path`, connect out to `peer_path`,
/// announce readiness, then park until the parent kills this process.
/// Never returns normally — the parent's `force_kill` is the only exit.
fn run_as_helper() -> ! {
    let path = PathBuf::from(std::env::var_os(PATH_ENV).expect("helper path env"));
    let peer_path = PathBuf::from(std::env::var_os(PEER_ENV).expect("helper peer env"));

    let _owner_listener = WindowsUnixListener::bind(&path).expect("helper bind owner listener");
    let _outbound =
        WindowsUnixStream::connect(&peer_path).expect("helper connect out to peer listener");

    println!("ready");
    std::io::stdout().flush().ok();

    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
