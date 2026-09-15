//! Live PTY test (rustils#83): spawns real children attached to a real
//! ConPTY pseudo console, no mock — the same live-verification bar #82's
//! Linux backend was held to. Only actually executes on CI's
//! `windows-latest` leg; this crate's whole backend is developed from a
//! Linux host against `cargo check --target x86_64-pc-windows-gnu`
//! (`crates/platform-windows/src/lib.rs`'s own module doc), so nothing
//! here has run outside CI.
//!
//! **Every blocking call in this file is bounded.** A first version of
//! this test used the portable `Pty`/`PtyMaster`/`Child` trait objects
//! directly, with unbounded `master.read()`/`child.wait()` calls — and a
//! genuine hang in that first ConPTY implementation attempt (caught
//! *because* of this) turned a ~1-minute CI job into a 45+ minute stuck
//! run that had to be cancelled by hand, with no diagnostic signal at
//! all about which call was stuck. Testing at the `sys::pty` level
//! instead (rather than through the type-erased `Box<dyn PtyMaster>`,
//! which has no way to reach the raw handles a bounded read needs)
//! gives every read a `wait_readable` budget and every wait a
//! `try_wait` poll budget — a real hang now fails the specific test
//! with a clear message inside `READ_BUDGET`/`WAIT_BUDGET`, not an
//! opaque multi-minute CI stall.

#![cfg(windows)]

use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use platform::process::{EnvSpec, ExitStatus, GroupSpec};
use platform::pty::Pty;
use platform::term::WinSize;
use platform_windows::ffi::win32_surface as w;
use platform_windows::sys::handle::OwnedWinHandle;
use platform_windows::sys::{fileio, proc as sysproc, pty as syspty};
use platform_windows::{winargv, WindowsPty};

const READ_BUDGET: Duration = Duration::from_secs(15);
const WAIT_BUDGET: Duration = Duration::from_secs(15);

/// Serializes every test that creates a real pseudo console.
/// `cargo test`'s default parallelism ran all of this file's tests
/// concurrently, each creating its own independent `HPCON` — and CI
/// showed exactly one test's *entire* child output nondeterministically
/// leaking to the job's own ambient console on each run (a different
/// test each time), while every test's own dedicated pipe only ever
/// received conhost's initial VT-mode negotiation and nothing past it.
/// That pattern — a race, not a fixed logic bug, since neither which
/// test "won" nor the earlier `CREATE_SUSPENDED` removal changed the
/// shape of the failure — points at concurrent pseudo-console
/// creation/attachment from one process not being safe on this runner,
/// not at anything wrong with a single spawn's own sequence. Serializing
/// is the direct test, not a guess: if it fixes the failure, the theory
/// was right; if not, the real cause is still elsewhere.
static PTY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_pty_tests() -> std::sync::MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn size() -> WinSize {
    WinSize { rows: 24, cols: 80 }
}

fn command_line(program: &str, args: &[&str]) -> Vec<u16> {
    let program = OsStr::new(program);
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    winargv::build_command_line(program, &args).expect("build_command_line")
}

/// Owns a pseudo console + its two master pipe handles, mirroring
/// `WindowsPtyMaster`'s own shape but at the `sys::pty` level so tests
/// can reach the raw handles `wait_readable` needs. `closed` is an `Arc`
/// (not a bare value) for the same reason `WindowsPtyMaster` uses one:
/// [`TestPty::watch_exit`] installs the same background exit-watcher
/// `WindowsPty::spawn` always installs in production
/// (`syspty::spawn_exit_watcher` — see its own doc comment), so this
/// helper stays a faithful mirror rather than silently not exercising
/// that fix. `output` stays a bare handle — the watcher never touches it
/// (deliberately, to avoid racing a caller's own concurrent reads; see
/// `spawn_exit_watcher`'s own doc comment), so there's nothing to share.
/// `Drop` races the same `closed` compare-exchange the watcher thread
/// does, calling `syspty::close` at most once either way.
struct TestPty {
    hpc: w::HPCON,
    input: OwnedWinHandle,
    output: OwnedWinHandle,
    closed: Arc<AtomicBool>,
}

impl TestPty {
    fn create() -> Self {
        let (hpc, input, output) = syspty::create_pty(size()).expect("create_pty");
        Self {
            hpc,
            input,
            output,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Install the same exit-watcher production spawns always install —
    /// call once, right after `syspty::spawn_attached` returns a process
    /// handle, in every test that actually spawns a child.
    fn watch_exit(&self, process: &OwnedWinHandle) {
        syspty::spawn_exit_watcher(process, self.hpc, Arc::clone(&self.closed));
    }
}

impl Drop for TestPty {
    fn drop(&mut self) {
        if self
            .closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            syspty::close(self.hpc, &self.output);
        }
    }
}

/// Bounded, non-blocking poll instead of `sys::proc::wait`'s unbounded
/// `WaitForSingleObject(..., INFINITE)` — see this file's own doc
/// comment for why every wait here is bounded.
fn wait_bounded(process: &OwnedWinHandle) -> ExitStatus {
    let deadline = Instant::now() + WAIT_BUDGET;
    loop {
        if let Some(status) = sysproc::try_wait(process).expect("try_wait") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "child did not exit within {WAIT_BUDGET:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Read from `output` in a loop until `needle` appears in the
/// accumulated output or `attempts` reads have happened —
/// `syspty::wait_readable` gates every `ReadFile` with `READ_BUDGET` so
/// a stuck read fails this assertion cleanly instead of hanging (see
/// this file's own doc comment).
fn read_until(output: &OwnedWinHandle, needle: &str, attempts: usize) -> String {
    let mut acc = String::new();
    let mut buf = [0u8; 256];
    for _ in 0..attempts {
        assert!(
            syspty::wait_readable(output, READ_BUDGET),
            "timed out after {READ_BUDGET:?} waiting for pty output; saw so far: {acc:?}"
        );
        let n = fileio::read(output, &mut buf).expect("read");
        if n == 0 {
            break;
        }
        acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        if acc.contains(needle) {
            return acc;
        }
    }
    acc
}

#[test]
fn spawn_attaches_a_real_child_to_the_pseudo_console() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    let line = command_line("cmd", &["/c", "echo hello-from-conpty"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    pty.watch_exit(&process);

    let output = read_until(&pty.output, "hello-from-conpty", 32);
    assert!(
        output.contains("hello-from-conpty"),
        "expected 'hello-from-conpty' in master output, saw: {output:?}"
    );

    let status = wait_bounded(&process);
    assert!(status.success());
}

/// Isolates whether `cmd.exe /c` itself (as opposed to this crate's
/// spawn sequence) is the source of the still-open bug where a
/// pty-hosted child's real console output never reaches the master pipe
/// (only conhost's own initial VT-mode negotiation does). Microsoft's
/// own ConPTY sample (`samples/ConPTY/EchoCon`, verified against the
/// primary source rather than memory) spawns `ping localhost` directly
/// — never a shell — so this mirrors that exactly, with no `cmd.exe`
/// anywhere in the process tree, as the most isolated comparison point
/// available. If this passes while every `cmd`-spawning test still
/// fails, `cmd.exe` specifically is implicated; if this fails too, the
/// bug is somewhere more fundamental and `cmd.exe` was never the
/// differentiator.
#[test]
fn spawn_a_plain_executable_with_no_shell_in_the_tree() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    let line = command_line("ping", &["-n", "1", "127.0.0.1"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    pty.watch_exit(&process);

    let output = read_until(&pty.output, "Pinging", 32);
    assert!(
        output.contains("Pinging"),
        "expected 'Pinging' in master output, saw: {output:?}"
    );

    let status = wait_bounded(&process);
    assert!(status.success());
}

#[test]
fn master_io_round_trips_with_the_spawned_child() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    // `/v:on` (delayed expansion, `!REPLY!` not `%REPLY%`) is required
    // here, not a ConPTY concern: cmd.exe expands `%VAR%` tokens once at
    // *parse* time for the whole `&`-joined command line, before `set
    // /p` has run — so `%REPLY%` would always read back whatever REPLY
    // held *before* this line started, empty here, regardless of
    // platform or pty correctness. This surfaced as this test's own
    // failure once the real bug (see `sys::pty::spawn_attached`'s
    // `STARTF_USESTDHANDLES` comment) was fixed and real child output
    // started reaching the pipe: the master output showed "hello" (our
    // written input, echoed) genuinely being received, immediately
    // followed by the literal, unexpanded "got:%REPLY%" — confirming
    // this was always a test-command bug, not a spawn/attach bug.
    let line = command_line("cmd", &["/v:on", "/c", "set /p REPLY= & echo got:!REPLY!"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    pty.watch_exit(&process);

    fileio::write(&pty.input, b"hello\r\n").expect("write");
    let output = read_until(&pty.output, "got:hello", 32);
    assert!(
        output.contains("got:hello"),
        "expected 'got:hello' in master output, saw: {output:?}"
    );

    let status = wait_bounded(&process);
    assert!(status.success());
}

/// Confirmed live (before `TestPty::watch_exit` existed) that ConPTY does
/// *not* spontaneously EOF its output pipe when the last attached
/// process exits — this test's own read blocked the full `READ_BUDGET`
/// even after `wait_bounded` had already confirmed the child was gone.
/// `syspty::spawn_exit_watcher` (installed via `watch_exit` below,
/// exactly like `WindowsPty::spawn` always installs it in production)
/// is what makes this pass: it forces `ClosePseudoConsole` once the
/// child exits, which is what actually breaks the pipe. See that
/// function's own doc comment for the full reasoning and the read-race
/// trade-off it accepts.
#[test]
fn eof_is_reported_as_ok_zero_after_the_child_exits() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    let line = command_line("cmd", &["/c", "exit 0"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    pty.watch_exit(&process);
    wait_bounded(&process);

    let deadline = Instant::now() + READ_BUDGET;
    loop {
        assert!(
            syspty::wait_readable(&pty.output, READ_BUDGET),
            "timed out after {READ_BUDGET:?} waiting for EOF"
        );
        let mut buf = [0u8; 64];
        let n = fileio::read(&pty.output, &mut buf)
            .expect("read must translate ERROR_BROKEN_PIPE to Ok(0)");
        if n == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "never reached EOF within {READ_BUDGET:?}"
        );
    }
}

#[test]
fn join_group_is_rejected() {
    // The one test that stays at the portable-trait level: it returns
    // before any OS call happens at all (no spawn, no read, no wait —
    // nothing that could hang), so the type-erased `Box<dyn PtyMaster>`
    // this returns is never touched.
    let pty = WindowsPty;
    let cmd = platform::process::Command::new("cmd", ".").group(GroupSpec::JoinGroup(1));
    let err = pty
        .spawn(&cmd, size())
        .err()
        .expect("JoinGroup must be rejected before attempting a real spawn");
    assert_eq!(err.kind, platform::error::ErrorKind::InvalidInput);
}

#[test]
fn resize_succeeds_against_a_real_pseudo_console() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    let line = command_line("cmd", &["/c", "exit 0"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    pty.watch_exit(&process);

    syspty::resize(
        pty.hpc,
        WinSize {
            rows: 40,
            cols: 120,
        },
    )
    .expect("resize");

    wait_bounded(&process);
}

/// The load-bearing test for the teardown ordering
/// (`docs/design-discussion-pty.md`'s EOF-vs-exit lesson,
/// `sys::pty::close`'s drain-before-`ClosePseudoConsole` fix): spawn a
/// child that writes far more output than a pipe's default buffer
/// holds, then drop the `TestPty` (calling `syspty::close`, exactly what
/// `WindowsPtyMaster::drop` does) **without ever reading any of it**.
/// This call itself is what needs to be bounded — `close`'s own internal
/// drain has a fixed budget (`docs/design-discussion-pty.md`), so unlike
/// every other test here this one needs no extra timeout wrapper; a
/// regression in that internal budget is what this test exists to catch.
#[test]
fn dropping_an_undrained_master_does_not_deadlock() {
    let _guard = lock_pty_tests();
    let pty = TestPty::create();
    let line = command_line("cmd", &["/c", "for /L %i in (1,1,20000) do @echo line %i"]);
    let (process, _pid) =
        syspty::spawn_attached(pty.hpc, &line, OsStr::new("."), &EnvSpec::Inherit)
            .expect("spawn_attached");
    // Mirrors production for consistency, though it's not this test's
    // own point: with the pipe never read and default buffer sizes far
    // smaller than 20000 lines, the child is essentially certain to
    // still be running (and the watcher thread still blocked waiting for
    // it) when `drop(pty)` below runs, so `Drop` wins the close race the
    // same way it always did before this thread existed.
    pty.watch_exit(&process);

    // Give the child a moment to actually produce output and fill the
    // pipe before tearing down undrained.
    std::thread::sleep(Duration::from_millis(200));
    drop(pty);

    // The child itself may still be mid-write when its pseudo console
    // tears down; reap it with the same bounded poll every other test
    // uses rather than assuming it has already exited.
    wait_bounded(&process);
}
