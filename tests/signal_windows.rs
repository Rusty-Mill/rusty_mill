#![cfg(windows)]
// `signal`'s Windows arm -- see `src/signal.rs`'s own "Windows" docs and
// `docs/decision-request-windows-process-signal-ipc.md` for the design.
//
// `signal::ctrl_c()` itself isn't exercised here: sending `CTRL_C_EVENT`
// via `GenerateConsoleCtrlEvent` can only target "every process sharing
// this console" (`dwProcessGroupId` must be `0` for that event
// specifically) -- there is no way to aim it at just a child process
// without also hitting the test harness's own process (and any other
// test binary sharing the same console), which would be a genuinely
// unsafe thing to do from an automated test. `CTRL_BREAK_EVENT` has no
// such restriction -- it can be targeted at one specific process group
// -- so this file spawns `ctrl_break_probe` (`src/bin/ctrl_break_probe.rs`)
// with `CREATE_NEW_PROCESS_GROUP` and fires `CTRL_BREAK_EVENT` at
// exactly that child's group, verifying real, end-to-end
// `SetConsoleCtrlHandler` → self-pipe → reactor → listener delivery
// through this crate's actual code, not a mock of any part of it. The
// same underlying `console_ctrl_handler`/self-pipe/`dispatch` machinery
// serves `ctrl_c`/`windows::ctrl_close`/`windows::ctrl_logoff`/
// `windows::ctrl_shutdown` too -- this is the one event in the family
// that can be exercised for real without the same targeting hazard.

use rusty_tokio::io::AsyncReadExt;
use rusty_tokio::process::{Command, Stdio};
use rusty_tokio::Runtime;
use std::os::windows::process::CommandExt;
use std::time::Duration;
use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

#[test]
fn ctrl_break_event_reaches_a_targeted_child_process_group() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let probe = env!("CARGO_BIN_EXE_ctrl_break_probe");
        let mut command = Command::new(probe);
        // `CREATE_NEW_PROCESS_GROUP`: makes the child the leader of its
        // own process group (its pid doubles as the group id), the
        // prerequisite for `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, ..)`
        // to target it specifically instead of this test's own console.
        // `Command::as_std_mut` is this crate's own documented escape
        // hatch for exactly this kind of builder option it doesn't wrap
        // itself.
        command
            .as_std_mut()
            .creation_flags(CREATE_NEW_PROCESS_GROUP);
        command.stdout(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let pid = child.id();

        let mut stdout = child.stdout.take().unwrap();

        // Read the `READY\n` line the probe prints once its listener is
        // actually registered -- without this handshake, the event could
        // fire before `SetConsoleCtrlHandler` has even been installed.
        let ready = read_line(&mut stdout).await;
        assert_eq!(ready, "READY", "probe did not report ready in time");

        // SAFETY: `pid` is a live child process, the leader of its own
        // process group (via `CREATE_NEW_PROCESS_GROUP` above) -- exactly
        // the precondition `GenerateConsoleCtrlEvent` documents for
        // targeting a `dwProcessGroupId` other than `0`.
        let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
        assert_ne!(
            ok,
            0,
            "GenerateConsoleCtrlEvent failed: {:?}",
            std::io::Error::last_os_error()
        );

        let received = read_line(&mut stdout).await;
        assert_eq!(
            received, "CTRL_BREAK_RECEIVED",
            "probe did not observe CTRL_BREAK_EVENT through rusty_tokio::signal"
        );

        let status = rusty_tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("probe should exit promptly after handling the event")
            .unwrap();
        assert!(status.success());
    });
}

#[test]
fn ctrl_close_logoff_shutdown_listeners_register_without_error() {
    // `CTRL_CLOSE_EVENT`/`CTRL_LOGOFF_EVENT`/`CTRL_SHUTDOWN_EVENT` have no
    // synthetic-delivery API the way `GenerateConsoleCtrlEvent` covers
    // `CTRL_C_EVENT`/`CTRL_BREAK_EVENT` -- they only ever fire for a real
    // window close/logoff/shutdown, which an automated test can't safely
    // (or sanely) trigger. This confirms the one thing that *can* be
    // verified without that: each event's listener constructor actually
    // reaches `SetConsoleCtrlHandler`/the self-pipe global setup and
    // succeeds, sharing the exact same `listen_for`/`WINDOWS_GLOBAL`
    // machinery `ctrl_break_event_reaches_a_targeted_child_process_group`
    // already proved delivers for real.
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let _close = rusty_tokio::signal::windows::ctrl_close().unwrap();
        let _logoff = rusty_tokio::signal::windows::ctrl_logoff().unwrap();
        let _shutdown = rusty_tokio::signal::windows::ctrl_shutdown().unwrap();
    });
}

/// Reads one `\n`-terminated line (with a generous timeout -- this is
/// real inter-process signal delivery, not a tight loop) from an async
/// `ChildStdout`, trimming the trailing newline.
async fn read_line(stdout: &mut rusty_tokio::process::ChildStdout) -> String {
    rusty_tokio::time::timeout(Duration::from_secs(10), async {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = stdout.read(&mut byte).await.unwrap();
            assert_ne!(n, 0, "probe exited before printing a full line");
            if byte[0] == b'\n' {
                break;
            }
            buf.push(byte[0]);
        }
        String::from_utf8(buf).unwrap()
    })
    .await
    .expect("timed out waiting for a line from the probe")
}
