//! Reference tool 3: opens an interactive PTY and spawns the host's
//! default shell in it. Exercises the PTY primitive only — no fs, no
//! captured (non-interactive) process spawn.
//!
//! This is a manual/interactive tool, not something CI runs headlessly:
//! it bridges the host terminal's stdin/stdout to the pty a byte at a
//! time. Exit the spawned shell (`exit` / Ctrl-D) ends the session — the
//! pty read side hits EOF right after, which is this tool's exit signal.

use std::io::{Read, Write};
use std::thread;

use compat::{NativeCapabilities, NativePtySession};
use contract::PtySession;

/// Puts the host terminal into raw/cbreak mode for the lifetime of the
/// guard, restoring cooked mode when dropped — including on an early
/// `?`-return or a panic unwind — so bridging bytes straight through to the
/// inner PTY (arrow keys, Ctrl-C, line editing) doesn't also leave the
/// user's real shell stuck without echo/line-editing if this tool exits
/// abnormally. Mirrors the enable-on-construct/restore-on-drop pattern
/// `rusty_term`'s `RawModeGuard` (`crates/rusty_term/src/render.rs`) uses
/// for the same host-terminal lifecycle.
struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn main() -> anyhow::Result<()> {
    let caps = NativeCapabilities::detect();
    eprintln!(
        "pty-shell: capabilities.pty_win32_input_mode={}",
        caps.pty_win32_input_mode
    );

    let session = NativePtySession;
    let spawn = session.spawn_shell(80, 24)?;
    let mut reader = spawn.reader;
    let mut writer = spawn.writer;
    let mut control = spawn.control;

    // Bridging stdin/stdout a byte at a time only behaves like a real
    // terminal (arrow keys, Ctrl-C passthrough, no local line editing) once
    // the host terminal itself is out of the default cooked/line-buffered
    // mode. `_raw_guard`'s `Drop` restores cooked mode on every exit path
    // out of `main` except the explicit `std::process::exit` below, which
    // is handled by dropping it explicitly first.
    let _raw_guard = RawModeGuard::enable()?;

    // Pump our stdin -> pty input on a detached background thread. It has
    // no clean way to unblock on shell exit (a blocking stdin read has no
    // signal to interrupt it), so the process simply exits out from under
    // it once the output side below observes EOF.
    thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Pump pty output -> our stdout on the main thread until EOF, which
    // the pty master delivers once the shell has exited and hung up.
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                std::io::stdout().write_all(&buf[..n])?;
                std::io::stdout().flush()?;
            }
        }
    }

    let status = control.wait()?;
    // `std::process::exit` skips destructors, so the guard must be dropped
    // by hand here to guarantee cooked mode is restored on this — the
    // normal — exit path too, not just on `?`/panic unwinds.
    drop(_raw_guard);
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::RawModeGuard;

    // A full PTY integration test (spawn this binary, drive a real
    // interactive terminal, and observe the host's console mode from the
    // outside) isn't practical in a headless CI runner — there's no
    // separate observer process here to check the *host's* mode from
    // outside the guard itself. This instead unit-tests the guard's
    // enable/restore logic in isolation: before the fix, `RawModeGuard`
    // didn't exist at all (raw mode was never entered), so this failed to
    // compile; after the fix, it must enable raw mode on construction and
    // restore the prior mode on drop, including when dropped early via an
    // explicit `drop()` rather than falling out of scope.
    #[test]
    fn raw_mode_guard_enables_and_restores_on_drop() {
        let before = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);

        let guard = match RawModeGuard::enable() {
            Ok(guard) => guard,
            // No attached console in this environment (e.g. a headless
            // test runner with redirected stdio) — nothing to assert.
            Err(_) => return,
        };
        assert!(
            crossterm::terminal::is_raw_mode_enabled().unwrap_or(false),
            "guard construction must put the terminal in raw mode"
        );

        drop(guard);
        assert_eq!(
            crossterm::terminal::is_raw_mode_enabled().unwrap_or(false),
            before,
            "dropping the guard must restore the terminal's prior mode"
        );
    }
}
