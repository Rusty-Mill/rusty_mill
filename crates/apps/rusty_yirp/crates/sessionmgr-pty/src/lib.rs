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
//! ConPTY backend on Windows and an `openpty` backend on Linux. On
//! Windows a session can run on our own console host, `rusty_conhost`,
//! instead of ConPTY ([`ConsoleHost`]). This crate adds three things that
//! live above them:
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

use std::ffi::{OsStr, OsString};
use std::io;
use std::sync::Mutex;
#[cfg(windows)]
use std::time::Duration;

use platform::process::{Child, Command, Stdio};
use platform::pty::{Pty, PtyMaster};
use platform::term::WinSize;

/// A sane ceiling on either terminal dimension in [`PtySession::resize`].
/// A `SessionResize` request can carry a client-supplied `u16` straight
/// off the wire, and both PTY backends allocate a screen buffer
/// proportional to it, so an unclamped value is an easy way to demand a
/// gigabyte-scale allocation for one resize. No real terminal is
/// anywhere near this large; it is generous headroom, not a realistic
/// size.
const MAX_TERMINAL_DIM: u16 = 500;

/// The environment variable that picks the Windows console host.
const CONSOLE_HOST_ENV: &str = "RUSTY_CONSOLE_HOST";

/// How long [`PtySession::shut_down`] waits for our own console host:
/// it ends clients still attached 5 s after hanging up, plus a second.
#[cfg(windows)]
const SHUTDOWN: Duration = Duration::from_secs(6);

/// Which console host runs a session's process on Windows.
///
/// Elsewhere there is only the platform's own pty, and this is ignored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleHost {
    /// Microsoft's ConPTY. The default.
    #[default]
    Conpty,
    /// Our own host, `rusty_conhost`. If this Windows can't run it,
    /// [`PtySession::spawn`] fails: there is no silent fallback.
    Own,
}

impl ConsoleHost {
    /// Reads `RUSTY_CONSOLE_HOST`: `conpty` or `own`, and [`Conpty`](
    /// ConsoleHost::Conpty) when unset. Any other value is an
    /// `InvalidInput` error naming the variable. Off Windows the variable
    /// is not read at all, so a Windows-only setting can never fail a
    /// session there.
    pub fn from_env() -> io::Result<Self> {
        if !cfg!(windows) {
            return Ok(ConsoleHost::default());
        }
        Self::parse(std::env::var_os(CONSOLE_HOST_ENV).as_deref())
    }

    fn parse(value: Option<&OsStr>) -> io::Result<Self> {
        let Some(value) = value else {
            return Ok(ConsoleHost::Conpty);
        };
        match value.to_str() {
            Some("conpty") => Ok(ConsoleHost::Conpty),
            Some("own") => Ok(ConsoleHost::Own),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{CONSOLE_HOST_ENV} must be `conpty` or `own`, not {value:?}"),
            )),
        }
    }
}

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
    master: Master,
    /// Taken by [`PtySession::wait`]. `Child::wait` consumes the box, so
    /// it has to be moved out rather than borrowed -- which is also what
    /// makes double-wait unrepresentable.
    child: Mutex<Option<Box<dyn Child>>>,
}

/// The terminal side of a session.
enum Master {
    /// The platform's pty: `openpty` on Linux, ConPTY on Windows.
    Platform(Box<dyn PtyMaster>),
    /// Our own console host. Concrete, because hanging up and shutting
    /// down are not on the `PtyMaster` trait.
    #[cfg(windows)]
    Own(rusty_conhost::OwnPtyMaster),
}

impl Master {
    fn get(&self) -> &dyn PtyMaster {
        match self {
            Master::Platform(master) => master.as_ref(),
            #[cfg(windows)]
            Master::Own(master) => master,
        }
    }
}

// SAFETY: every concrete `PtyMaster`/`Child` this crate can construct is
// one of `rustils`' own backends or our console host's, and each owns
// only handles that are safe to use from any thread: `LinuxPtyMaster` an
// `OwnedFd`, `WindowsPtyMaster` two `OwnedWinHandle`s plus an `HPCON` and
// an `Arc<AtomicBool>`, `OwnPtyMaster` an `Arc` of engine state behind
// locks (it is `Send + Sync` without any help; only the boxed trait
// object of the platform variant needs this bridge), and `WindowsChild`
// a process handle. Windows kernel handles and Unix file descriptors are
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
    /// Which Windows console host to run on; ignored elsewhere. Callers
    /// read it with [`ConsoleHost::from_env`].
    pub console_host: ConsoleHost,
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

        let (master, child) = spawn_on(options.console_host, &command, options.size.into())?;
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
        self.inner.master.get().read(buf).map_err(to_io)
    }

    /// Writes input. **Blocking.**
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.master.get().write(buf).map_err(to_io)
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

    /// Tells the terminal it has been resized. Each dimension is
    /// clamped to [`MAX_TERMINAL_DIM`]: `size` may carry a client-
    /// supplied value straight off the wire, and the PTY backends
    /// allocate a screen buffer proportional to it.
    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        let clamped = TerminalSize {
            rows: size.rows.min(MAX_TERMINAL_DIM),
            cols: size.cols.min(MAX_TERMINAL_DIM),
        };
        self.inner
            .master
            .get()
            .resize(clamped.into())
            .map_err(to_io)
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

    /// Hangs up, then waits for the terminal to finish shutting down.
    /// **Blocking**, for up to 6 s.
    ///
    /// Only our own console host has work here: once hung up, it closes
    /// the console and, 5 s later, tries to end every client still
    /// attached, but only while this process runs. So a worker calls this
    /// before it exits, or those processes are orphaned on a console
    /// nobody serves.
    ///
    /// Returns whether the host finished in time (`OwnPtyMaster::
    /// wait_shut_down`). `true` means its shutdown, including those end
    /// attempts, is complete. It does **not** mean every attached process
    /// has exited: one the host could not open or end, or any after an
    /// engine error closed the console early, may still run. ConPTY and
    /// Linux return `true` at once.
    pub fn shut_down(&self) -> bool {
        match &self.inner.master {
            Master::Platform(_) => true,
            #[cfg(windows)]
            Master::Own(master) => {
                master.hang_up();
                master.wait_shut_down(SHUTDOWN)
            }
        }
    }
}

/// Spawns `command` on the pty for this platform. `host` only matters on
/// Windows.
#[cfg(target_os = "linux")]
fn spawn_on(
    _host: ConsoleHost,
    command: &Command,
    size: WinSize,
) -> io::Result<(Master, Box<dyn Child>)> {
    let (master, child) = platform_linux::LinuxPty
        .spawn(command, size)
        .map_err(to_io)?;
    Ok((Master::Platform(master), child))
}

/// Spawns `command` on the pty for this platform. `host` only matters on
/// Windows.
#[cfg(windows)]
fn spawn_on(
    host: ConsoleHost,
    command: &Command,
    size: WinSize,
) -> io::Result<(Master, Box<dyn Child>)> {
    if host == ConsoleHost::Own {
        return spawn_own(command, size);
    }
    let (master, child) = platform_windows::WindowsPty
        .spawn(command, size)
        .map_err(to_io)?;
    Ok((Master::Platform(master), child))
}

/// Spawns `command` on our own console host.
///
/// When the child exits, the host itself sends CTRL_CLOSE to whatever it
/// left attached and closes within 2 s, so [`PtySession::read`] ends as
/// it does on ConPTY. With something still attached that end comes up to
/// 2 s after the exit, where ConPTY's comes at once.
#[cfg(windows)]
fn spawn_own(command: &Command, size: WinSize) -> io::Result<(Master, Box<dyn Child>)> {
    let (master, child) = rusty_conhost::OwnPty
        .spawn_concrete(command, size)
        .map_err(to_io)?;
    Ok((Master::Own(master), Box::new(child)))
}

/// No PTY backend on this platform.
///
/// A compile error rather than a stub that fails at run time: a build
/// that cannot host a terminal cannot host an agent session, and finding
/// that out at run time would be finding it out from a user.
#[cfg(not(any(target_os = "linux", windows)))]
compile_error!(
    "sessionmgr-pty supports Windows (ConPTY or our own console host) and Linux; \
     macOS/BSD need platform-bsd wiring that has not been done"
);

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
            // Our own host's master is held concretely, but `Inner`'s
            // bridge covers it too, so its premise is asserted as well.
            assert_send::<rusty_conhost::OwnPtyMaster>();
            assert_sync::<rusty_conhost::OwnPtyMaster>();
            assert_send::<platform_windows::WindowsChild>();
        }
    }

    #[test]
    fn the_console_host_defaults_to_conpty_when_unset() {
        assert_eq!(
            ConsoleHost::parse(None).expect("unset"),
            ConsoleHost::Conpty
        );
    }

    #[test]
    fn the_console_host_accepts_exactly_its_two_names() {
        let parse = |v: &str| ConsoleHost::parse(Some(OsStr::new(v)));
        assert_eq!(parse("conpty").expect("conpty"), ConsoleHost::Conpty);
        assert_eq!(parse("own").expect("own"), ConsoleHost::Own);
    }

    #[test]
    #[cfg(not(windows))]
    fn the_console_host_variable_is_ignored_off_windows() {
        // Review of ff3a25b3 (Codex P2): a bad value inherited by a Linux
        // daemon must not fail its sessions. Nothing else in this crate's
        // tests reads the variable, so setting it here races nothing.
        std::env::set_var(CONSOLE_HOST_ENV, "not-a-host");
        let host = ConsoleHost::from_env();
        std::env::remove_var(CONSOLE_HOST_ENV);
        assert_eq!(host.expect("ignored off Windows"), ConsoleHost::Conpty);
    }

    #[test]
    fn any_other_console_host_is_an_error_naming_the_variable() {
        // No silent fallback (design §4.4): a typo must not quietly run
        // the session on the other host.
        for bad in ["", "Own", "OWN", "conhost", " own"] {
            let error = ConsoleHost::parse(Some(OsStr::new(bad))).expect_err(bad);
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
            assert!(error.to_string().contains(CONSOLE_HOST_ENV), "{error}");
        }
    }

    #[test]
    fn the_default_terminal_size_is_usable() {
        // A zero dimension makes a PTY-hosted program lay out to nothing,
        // which looks like a hang rather than a misconfiguration.
        let size = TerminalSize::default();
        assert!(size.rows > 0 && size.cols > 0);
    }

    /// `cmd.exe` with `args` on our own console host.
    #[cfg(windows)]
    fn own_cmd(args: &[&str]) -> PtyOptions {
        PtyOptions {
            program: "cmd.exe".into(),
            args: args.iter().map(Into::into).collect(),
            cwd: std::env::temp_dir().into_os_string(),
            size: TerminalSize::default(),
            console_host: ConsoleHost::Own,
        }
    }

    /// Reads to end of stream on another thread; `None` if that takes
    /// longer than 30 s, so a missing end fails rather than hangs.
    #[cfg(windows)]
    fn read_within_budget(session: &std::sync::Arc<PtySession>) -> Option<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = std::sync::Arc::clone(session);
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = session.read(&mut buf) {
                out.extend_from_slice(&buf[..n]);
            }
            let _ = tx.send(out);
        });
        rx.recv_timeout(std::time::Duration::from_secs(30)).ok()
    }

    #[test]
    #[cfg(windows)]
    fn our_own_host_reports_output_and_exit_code() {
        let session = std::sync::Arc::new(
            PtySession::spawn(own_cmd(&["/c", "echo", "hello"])).expect("spawn on our host"),
        );
        let output = read_within_budget(&session).expect("the stream ends");
        assert!(String::from_utf8_lossy(&output).contains("hello"));
        assert_eq!(session.wait().expect("wait"), Some(0));
        assert!(session.shut_down(), "nothing is left attached");

        let session =
            std::sync::Arc::new(PtySession::spawn(own_cmd(&["/c", "exit", "7"])).expect("spawn"));
        read_within_budget(&session).expect("the stream ends");
        assert_eq!(session.wait().expect("wait"), Some(7));
    }

    #[test]
    #[cfg(windows)]
    fn our_own_host_ends_the_stream_when_the_process_exits() {
        // `start /b` leaves ping attached to the console after cmd exits.
        // ConPTY's backend ends the stream on the child's exit; ours must
        // too, or a worker waits on the left-behind process for a minute.
        let session = std::sync::Arc::new(
            PtySession::spawn(own_cmd(&[
                "/c",
                "start",
                "/b",
                "ping",
                "-n",
                "60",
                "127.0.0.1",
            ]))
            .expect("spawn on our host"),
        );
        assert!(
            read_within_budget(&session).is_some(),
            "the stream must end once cmd exits, not when ping does"
        );
        assert_eq!(session.wait().expect("wait"), Some(0));
        assert!(session.shut_down(), "the host ends ping within its grace");
    }

    #[cfg(unix)]
    fn sh(script: &str) -> PtyOptions {
        PtyOptions {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir().into_os_string(),
            size: TerminalSize::default(),
            console_host: ConsoleHost::default(),
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
            console_host: ConsoleHost::default(),
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
    fn resize_clamps_an_out_of_range_size_instead_of_allocating_it() {
        // A `SessionResize` request carries client-supplied `u16`s
        // straight off the wire; an unclamped 65535x65535 would ask the
        // PTY backend to allocate a screen buffer that large.
        let session = PtySession::spawn(sh("sleep 0.2; stty size")).expect("spawn");
        session
            .resize(TerminalSize {
                rows: 65535,
                cols: 65535,
            })
            .expect("resize");
        let output = String::from_utf8_lossy(&read_to_end(&session, 4096)).into_owned();
        assert!(
            output.contains(&format!("{MAX_TERMINAL_DIM} {MAX_TERMINAL_DIM}")),
            "an out-of-range resize must be clamped to {MAX_TERMINAL_DIM}, got: {output:?}"
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
