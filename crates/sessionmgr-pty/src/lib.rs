//! PTY adapter: runs a session's process on a **real terminal**.
//!
//! # Why this is not optional
//!
//! The Phase 1 spike settled it with evidence rather than preference
//! (`docs/decisions/0002-pty-required-for-agent-sessions.md`): interactive
//! Claude Code under piped stdio does not degrade, it **refuses to run**.
//! It silently falls back to `--print` mode and exits 1. Under a PTY the
//! same command renders its full interface. An agent-session manager
//! whose sessions cannot host an interactive agent is not the product.
//!
//! # What this wraps, and what it does not reimplement
//!
//! `rustils`' `platform::pty::Pty` capability, which already has a
//! ConPTY backend on Windows and an `openpty` backend on Linux. This
//! crate adds three things that live above it:
//!
//! 1. **One owner for the master and the child**, so a session is a
//!    single value with a pid rather than two loose halves.
//! 2. **`std::io` error types**, so the rest of the project does not
//!    thread a platform error type through its own.
//! 3. The `Send`/`Sync` bridge described below.
//!
//! # Threading, and the one `unsafe` here
//!
//! `PtyMaster`'s methods all take `&self` and all **block**. A session
//! needs to read output continuously while still accepting input and
//! resizes, so the master is genuinely shared across threads.
//!
//! `Pty::spawn` returns `Box<dyn PtyMaster>` and `Box<dyn Child>` --
//! trait objects with no `Send`/`Sync` bound, so neither can cross a
//! thread boundary as-is, even though the concrete backends can. That is
//! a gap in the trait object's type, not in the implementations, and it
//! is bridged here with an `unsafe impl` on a private wrapper.
//!
//! The premise is **checked, not assumed**: [`tests::the_backend_master_is_send_and_sync`]
//! asserts `Send + Sync` on the concrete backend type for whichever
//! platform is being compiled. If a future `rustils` makes a backend
//! thread-hostile, that test stops compiling and this bridge stops being
//! justified -- which is the point of keeping it.

use std::ffi::OsString;
use std::io;
use std::sync::Mutex;

use platform::process::{Child, Command, Stdio};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

/// A terminal size. Mirrors `platform::term::WinSize` so callers do not
/// need to depend on the platform crate directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for TerminalSize {
    /// 24x80, the classic default.
    ///
    /// A default is unavoidable: a session can be created by a client
    /// that has no terminal of its own (a script, or the daemon's own
    /// auto-start path), and a PTY must be given *some* size at creation.
    /// Attached clients correct it with [`PtySession::resize`].
    fn default() -> Self {
        TerminalSize { rows: 24, cols: 80 }
    }
}

impl From<TerminalSize> for WinSize {
    fn from(size: TerminalSize) -> WinSize {
        WinSize {
            rows: size.rows,
            cols: size.cols,
        }
    }
}

/// The master and child of one PTY-hosted process.
///
/// See the module docs for why this is `unsafe impl Send`/`Sync`.
struct Inner {
    master: Box<dyn PtyMaster>,
    /// Taken by [`PtySession::wait`]. `Child::wait` consumes the box, so
    /// it has to be moved out rather than borrowed -- which is also what
    /// makes double-wait unrepresentable.
    child: Mutex<Option<Box<dyn Child>>>,
}

// SAFETY: every concrete `PtyMaster`/`Child` this crate can construct is
// one of `rustils`' own backends, and each owns only handles that are
// safe to use from any thread: `LinuxPtyMaster` an `OwnedFd`,
// `WindowsPtyMaster` two `OwnedWinHandle`s plus an `HPCON` and an
// `Arc<AtomicBool>`. Windows kernel handles and Unix file descriptors are
// process-wide values with no thread affinity, and the trait's own API
// takes `&self` for read, write, and resize -- so it is designed for
// concurrent use. The Windows master's read and write sides are two
// *separate* handles, so a concurrent read and write do not even touch
// the same object.
//
// The premise is asserted at compile time for the concrete backend type
// on each platform; see `tests::the_backend_master_is_send_and_sync`.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

/// A process running on its own pseudo-terminal.
///
/// Cloneable handles are not offered: wrap it in an `Arc` if several
/// threads need it, which is what the worker does.
pub struct PtySession {
    inner: Inner,
    pid: u32,
}

/// How to start a PTY-hosted process.
pub struct PtyOptions {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Always explicit -- the underlying capability has no "inherit the
    /// ambient working directory" variant, and for this project that is
    /// exactly right: a worktree session's whole purpose is running
    /// somewhere specific.
    pub cwd: OsString,
    pub size: TerminalSize,
}

impl PtySession {
    /// Starts `options.program` on a fresh pseudo-terminal.
    ///
    /// The child is unconditionally its own session/process group -- that
    /// is inherent to being given a terminal, not a choice made here.
    pub fn spawn(options: PtyOptions) -> io::Result<Self> {
        let mut command = Command::new(options.program, options.cwd);
        for arg in options.args {
            command = command.arg(arg);
        }
        // The child's stdio *is* the terminal; there are no separate
        // pipes to configure. This also means stdout and stderr are
        // merged, which is what a terminal does and what an attached
        // client should see.
        command.stdin = Stdio::Inherit;
        command.stdout = Stdio::Inherit;
        command.stderr = Stdio::Inherit;

        let (master, child) = backend()
            .spawn(&command, options.size.into())
            .map_err(to_io)?;
        let pid = child.id();
        Ok(PtySession {
            inner: Inner {
                master,
                child: Mutex::new(Some(child)),
            },
            pid,
        })
    }

    /// The hosted process's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Reads output. **Blocking.** `Ok(0)` means the process has exited.
    ///
    /// That end-of-stream signal is the trait's documented contract on
    /// both backends, which matters because a PTY does not produce a
    /// clean EOF by itself the way a pipe does -- the Windows backend
    /// arranges it explicitly with an exit watcher.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.master.read(buf).map_err(to_io)
    }

    /// Writes input. **Blocking.**
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.master.write(buf).map_err(to_io)
    }

    /// Writes input until all of it is accepted.
    pub fn write_all(&self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match self.write(buf)? {
                0 => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "the terminal accepted no input",
                    ))
                }
                n => buf = &buf[n..],
            }
        }
        Ok(())
    }

    /// Tells the terminal it has been resized.
    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        self.inner.master.resize(size.into()).map_err(to_io)
    }

    /// Waits for the process to exit and returns its exit code.
    /// **Blocking.**
    ///
    /// `Ok(None)` means it ended without a code -- killed by a signal on
    /// Unix. Callers treat that as a failure rather than a success, since
    /// a killed agent has not finished its work.
    ///
    /// Calling this a second time returns `Ok(None)`: the child has
    /// already been consumed, and there is nothing left to wait on.
    pub fn wait(&self) -> io::Result<Option<i32>> {
        let child = {
            let mut guard = self
                .inner
                .child
                .lock()
                .map_err(|_| io::Error::other("the pty child lock was poisoned"))?;
            guard.take()
        };
        let Some(child) = child else {
            return Ok(None);
        };
        let status = child.wait().map_err(to_io)?;
        Ok(match status {
            platform::process::ExitStatus::Code(code) => Some(code),
            // Killed by a signal. Reported as "no code" rather than
            // mapped to one, because there is no exit code -- and the
            // domain treats that as a failure, which is correct: an agent
            // that was killed has not finished its work.
            platform::process::ExitStatus::Signaled(_) => None,
            // Job-control stops are only ever produced by `wait_job`,
            // which this crate never calls, so these are unreachable in
            // practice. Reported as "no code" rather than panicked on:
            // an unreachable arm that fires should degrade, not crash a
            // worker that is holding a live session.
            _ => None,
        })
    }
}

#[cfg(target_os = "linux")]
fn backend() -> impl Pty {
    platform_linux::LinuxPty
}

#[cfg(windows)]
fn backend() -> impl Pty {
    platform_windows::WindowsPty
}

/// No PTY backend on this platform.
///
/// A compile error rather than a stub that fails at run time: a build
/// that cannot host a terminal cannot host an agent session, and finding
/// that out at run time would be finding it out from a user.
#[cfg(not(any(target_os = "linux", windows)))]
fn backend() -> impl Pty {
    compile_error!(
        "sessionmgr-pty supports Windows (ConPTY) and Linux; \
         macOS/BSD need platform-bsd wiring that has not been done"
    );
}

/// Flattens a platform error into `std::io::Error`, keeping its message.
fn to_io(error: platform::error::PlatformError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn the_backend_master_is_send_and_sync() {
        // The evidence for this crate's `unsafe impl Send`/`Sync` on
        // `Inner`. `Pty::spawn` hands back a `Box<dyn PtyMaster>`, which
        // carries no thread bounds, so the bridge rests on the concrete
        // backend actually being thread-safe. If a future `rustils`
        // changes that, this stops compiling and the `unsafe` stops being
        // justified -- which is exactly the alarm worth having.
        #[cfg(target_os = "linux")]
        {
            assert_send::<platform_linux::LinuxPtyMaster>();
            assert_sync::<platform_linux::LinuxPtyMaster>();
        }
        #[cfg(windows)]
        {
            assert_send::<platform_windows::WindowsPtyMaster>();
            assert_sync::<platform_windows::WindowsPtyMaster>();
        }
    }

    #[test]
    fn the_default_terminal_size_is_usable() {
        // A zero dimension makes a PTY-hosted program lay out to nothing,
        // which looks like a hang rather than a misconfiguration.
        let size = TerminalSize::default();
        assert!(size.rows > 0 && size.cols > 0);
    }

    #[cfg(unix)]
    fn sh(script: &str) -> PtyOptions {
        PtyOptions {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir().into_os_string(),
            size: TerminalSize::default(),
        }
    }

    /// Reads until the process exits, or `limit` bytes.
    #[cfg(unix)]
    fn read_to_end(session: &PtySession, limit: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while out.len() < limit {
            match session.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                // A PTY master reports the child's exit as an I/O error
                // on some platforms rather than a clean zero-length read.
                Err(_) => break,
            }
        }
        out
    }

    #[test]
    #[cfg(unix)]
    fn a_hosted_process_sees_a_real_terminal() {
        // The property the whole crate exists for, asserted from the
        // child's own point of view rather than from ours: under piped
        // stdio this prints "no", and that difference is precisely what
        // makes interactive agent CLIs refuse to run.
        let session = PtySession::spawn(sh("test -t 1 && echo IS_A_TTY || echo NOT_A_TTY"))
            .expect("spawn a pty session");
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        assert!(
            output.contains("IS_A_TTY"),
            "the hosted process must see a terminal on its stdout, got: {output:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn output_and_exit_code_are_both_reported() {
        let session = PtySession::spawn(sh("echo hello; exit 7")).expect("spawn");
        let output = read_to_end(&session, 4096);
        assert!(String::from_utf8_lossy(&output).contains("hello"));
        assert_eq!(session.wait().expect("wait"), Some(7));
    }

    #[test]
    #[cfg(unix)]
    fn waiting_twice_does_not_panic_or_block() {
        let session = PtySession::spawn(sh("exit 0")).expect("spawn");
        let _ = read_to_end(&session, 4096);
        assert_eq!(session.wait().expect("first wait"), Some(0));
        assert_eq!(
            session.wait().expect("second wait"),
            None,
            "a second wait has nothing to wait on and must not block"
        );
    }

    #[test]
    #[cfg(unix)]
    fn input_written_to_the_terminal_reaches_the_process() {
        let session = PtySession::spawn(sh("read line; echo GOT:$line")).expect("spawn");
        session.write_all(b"marker\n").expect("write input");
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        assert!(
            output.contains("GOT:marker"),
            "input must reach the hosted process, got: {output:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn the_terminal_reports_the_size_it_was_given() {
        let session = PtySession::spawn(PtyOptions {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "stty size".into()],
            cwd: std::env::temp_dir().into_os_string(),
            size: TerminalSize {
                rows: 40,
                cols: 132,
            },
        })
        .expect("spawn");
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        assert!(
            output.contains("40 132"),
            "the hosted program should see the size it was given, got: {output:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn resizing_is_visible_to_the_hosted_process() {
        // A session created by a client with no terminal of its own gets
        // a default size, so a UI attaching later has to be able to
        // correct it -- otherwise every such session renders to 24x80
        // forever.
        let session = PtySession::spawn(sh("sleep 0.2; stty size")).expect("spawn");
        session
            .resize(TerminalSize {
                rows: 50,
                cols: 200,
            })
            .expect("resize");
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        assert!(
            output.contains("50 200"),
            "a resize must reach the hosted process, got: {output:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_session_can_be_read_and_written_from_different_threads() {
        // The concurrency the `unsafe impl` exists to allow, exercised
        // rather than only asserted at the type level.
        use std::sync::Arc;
        let session = Arc::new(PtySession::spawn(sh("read line; echo GOT:$line")).expect("spawn"));
        let writer = {
            let session = Arc::clone(&session);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                session.write_all(b"threaded\n")
            })
        };
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        writer.join().expect("writer thread").expect("write");
        assert!(output.contains("GOT:threaded"), "got: {output:?}");
    }
}
