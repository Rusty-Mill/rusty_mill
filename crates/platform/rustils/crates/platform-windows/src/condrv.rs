//! The console driver (ConDrv) mechanism for our own Windows console host
//! (#1222, PR 1). `rusty_conhost` builds the host on top of it; this
//! module only opens a server, attaches a child, and moves packets. No
//! `unsafe` here; it lives in `sys::condrv`.
//!
//! The contract is the design's §11.2.1
//! (`docs/projects/term/conhost/design.md`):
//! - One owner, the host's engine thread, holds a [`ConsoleServer`]
//!   (`Send`, not `Sync`) and is the only caller of its methods.
//! - At most one READ_IO is in flight. [`ConsoleServer::wait`] returns a
//!   [`Ready`] snapshot with no priority between sources, and takes an
//!   absolute deadline.
//! - [`send_ctrl`] may block, so the host calls it from a worker of its
//!   own, never from the engine thread.
//!
//! Layouts and constants are transcribed from microsoft/terminal (MIT);
//! the notice is at the top of `sys::condrv`.

use std::ffi::{OsStr, OsString};
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Command, GroupSpec, Spawner};

use crate::process::WindowsChild;
use crate::sys::condrv as sys;
use crate::sys::handle::{self, OwnedWinHandle};

/// `CTRL_C_EVENT`, `CTRL_BREAK_EVENT`, `CTRL_CLOSE_EVENT` (wincon.h).
const CTRL_C_EVENT: u32 = 0;
const CTRL_BREAK_EVENT: u32 = 1;
const CTRL_CLOSE_EVENT: u32 = 2;
/// `CONSOLE_CTRL_*_FLAG`, which conhost passes alongside the event code.
const CONSOLE_CTRL_C_FLAG: u32 = 0x1;
const CONSOLE_CTRL_BREAK_FLAG: u32 = 0x2;
const CONSOLE_CTRL_CLOSE_FLAG: u32 = 0x4;
/// How long a failed [`spawn_attached`] waits for the child it terminated.
const REAP_GRACE: Duration = Duration::from_secs(5);

/// A console server: the driver's server object, its client objects, the
/// input-available event, and the storage of the one in-flight READ_IO.
///
/// `Send`, not `Sync`: the host's engine thread owns it. Dropping it, or
/// [`ConsoleServer::close`], cancels and drains an in-flight read, then
/// closes the server, and the driver disconnects every client.
pub struct ConsoleServer {
    inner: sys::Server,
}

/// What [`spawn_attached`] returns.
pub struct Attached {
    pub server: ConsoleServer,
    pub child: WindowsChild,
    /// A duplicate of the child's process handle, for the engine's
    /// [`ConsoleServer::wait`].
    pub wait: OwnedWinHandle,
}

/// One packet the driver delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoDescriptor {
    /// Opaque; echo it back unchanged.
    pub id: [u8; 8],
    /// The driver's per-client handle from CONNECT. On CONNECT itself,
    /// the client's pid.
    pub process: usize,
    /// The driver's per-object handle from CREATE_OBJECT. On CONNECT
    /// itself, the client's thread id.
    pub object: usize,
    /// 1..=8: CONNECT, DISCONNECT, CREATE_OBJECT, CLOSE_OBJECT,
    /// RAW_WRITE, RAW_READ, USER_DEFINED, RAW_FLUSH.
    pub function: u32,
    /// Bytes readable with [`ConsoleServer::read_input`].
    pub input_size: u32,
    /// Bytes writable with [`ConsoleServer::write_output`] and `write_back`.
    pub output_size: u32,
}

/// The completion of one packet.
#[derive(Clone, Copy, Debug)]
pub struct IoComplete<'a> {
    pub id: [u8; 8],
    /// An NTSTATUS.
    pub status: i32,
    pub information: usize,
    /// The fixed reply struct, written at the start of the packet's output.
    pub write_back: &'a [u8],
}

/// A Ctrl event for [`send_ctrl`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlEvent {
    C,
    Break,
    Close,
}

/// Ends a [`ConsoleServer::wait`] from any thread: an auto-reset event.
/// Several wakes before a wait collapse into one. `Send + Sync`.
#[derive(Debug)]
pub struct Waker {
    event: OwnedWinHandle,
}

/// What [`ConsoleServer::wait`] found ready once it stopped waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ready {
    /// The completed read, if it completed.
    pub io: Option<IoDescriptor>,
    /// Whether the `exit` handle is signalled.
    pub exited: bool,
}

/// `Ok(())` when a console server can be opened and `ConsoleControl`
/// resolves. `Unsupported` when the device or the export is missing.
/// Other OS errors are mapped as usual. No other side effects; callable
/// any number of times.
pub fn probe() -> Result<()> {
    match sys::open_server() {
        Ok(server) => drop(server),
        Err(e) if e.kind == ErrorKind::NotFound => return Err(unsupported("ConDrv server")),
        Err(e) => return Err(e),
    }
    if !sys::has_console_control() {
        return Err(unsupported("user32!ConsoleControl"));
    }
    Ok(())
}

fn unsupported(what: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::Unsupported, OsCode::None, what)
}

/// Spawn `cmd` attached to a new console server.
///
/// The program is resolved and quoted exactly as
/// [`crate::WindowsPty::spawn_concrete`] does, under the same spawn lock.
/// The child gets the server's `\Input` and `\Output` as its std handles,
/// and inherits nothing else, except, as with any `CreateProcessW`, this
/// process's "ignore Ctrl+C" flag: a caller that ignores Ctrl+C (one
/// started with `CREATE_NEW_PROCESS_GROUP`, or below such a process) has
/// a child that ignores it too. The caller's own policy is never changed.
/// Returns once `CreateProcessW` succeeds, so
/// the child's CONNECT is the first packet [`ConsoleServer::wait`]
/// reports. `GroupSpec::JoinGroup` gives `InvalidInput`.
///
/// On a failure before the child is created, every handle is closed. On
/// one after, the child is terminated and the error returned once it has
/// exited, every handle closed. If its exit can't be confirmed within 5 s,
/// or `TerminateProcess` fails while it runs, that cleanup error is
/// returned instead (`TimedOut`, or the OS error), and the child's process
/// handle and console stay open on a background thread until it exits.
pub fn spawn_attached(cmd: &Command) -> Result<Attached> {
    if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "condrv::spawn_attached: GroupSpec::JoinGroup is incompatible with a fresh console",
        ));
    }
    let resolved = crate::WindowsSpawner.resolve(&cmd.program)?;
    let args: Vec<&OsStr> = cmd.argv.iter().map(OsString::as_os_str).collect();
    let line = crate::winargv::build_command_line(&resolved, &args)?;

    let _serial = crate::pty::SPAWN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let server = sys::Server::create()?;
    let (process, pid) = sys::spawn(&server, &line, &cmd.cwd, &cmd.env)?;
    finish_spawn(
        server,
        process,
        pid,
        |p| handle::duplicate(p, false),
        |p| terminate_and_reap(p, crate::sys::proc::terminate_process, REAP_GRACE),
    )
}

/// The child is running: finish the remaining fallible setup, or roll the
/// child back. When `rollback` confirms the child has exited, setup's
/// error is returned and every handle is closed here. When it can't, its
/// own error is returned instead, and the child's process handle and
/// console are kept open until the child exits.
fn finish_spawn(
    server: sys::Server,
    process: OwnedWinHandle,
    pid: u32,
    duplicate: impl FnOnce(&OwnedWinHandle) -> Result<OwnedWinHandle>,
    rollback: impl FnOnce(&OwnedWinHandle) -> Result<()>,
) -> Result<Attached> {
    let wait = match duplicate(&process) {
        Ok(wait) => wait,
        Err(setup) => {
            if let Err(cleanup) = rollback(&process) {
                keep_until_exit(process, server);
                return Err(cleanup);
            }
            return Err(setup);
        }
    };
    Ok(Attached {
        server: ConsoleServer { inner: server },
        child: WindowsChild::from_parts(process, None, pid),
        wait,
    })
}

/// Terminate `process` with `terminate` and wait up to `grace` for it to
/// exit. `Ok(())` only once it has exited. A failed `terminate` on a
/// process that has already exited counts as exited; otherwise its error
/// is returned. A process still running after `grace` (termination is
/// asynchronous) gives `TimedOut`.
fn terminate_and_reap(
    process: &OwnedWinHandle,
    terminate: impl FnOnce(&OwnedWinHandle) -> Result<()>,
    grace: Duration,
) -> Result<()> {
    if let Err(e) = terminate(process) {
        if crate::sys::proc::try_wait(process)?.is_some() {
            return Ok(());
        }
        return Err(e);
    }
    match crate::sys::proc::wait_many(&[process.as_raw()], Some(grace))? {
        Some(_) => Ok(()),
        None => Err(PlatformError::new(
            ErrorKind::TimedOut,
            OsCode::None,
            "condrv::spawn_attached: the rolled-back child had not exited",
        )),
    }
}

/// Hold a child's process handle and console on a thread of their own
/// until the child exits, so nothing it may still use is closed under
/// it. Should the thread fail to start, they are closed now.
fn keep_until_exit(process: OwnedWinHandle, server: sys::Server) {
    let _ = std::thread::Builder::new()
        .name("condrv-rollback".into())
        .spawn(move || {
            // An infinite wait on a handle we own fails only if the handle
            // is invalid, and then there is nothing left to wait for.
            let _ = crate::sys::proc::wait(&process);
            drop(server);
        });
}

/// Ask Windows to deliver `event` to `pid` through
/// `user32!ConsoleControl(ConsoleEndTask, …)`, as conhost does.
///
/// `Ok(())` means the call was made, not that the event arrived: the
/// driver reports the same for a pid that isn't attached to us, and
/// Windows then drops it. `Unsupported` when the export is missing. It
/// may block on the target's handler, so call it from a worker thread.
pub fn send_ctrl(pid: u32, event: CtrlEvent) -> Result<()> {
    let (code, flag) = match event {
        CtrlEvent::C => (CTRL_C_EVENT, CONSOLE_CTRL_C_FLAG),
        CtrlEvent::Break => (CTRL_BREAK_EVENT, CONSOLE_CTRL_BREAK_FLAG),
        CtrlEvent::Close => (CTRL_CLOSE_EVENT, CONSOLE_CTRL_CLOSE_FLAG),
    };
    sys::end_task(pid, code, flag)
}

impl Waker {
    pub fn new() -> Result<Waker> {
        Ok(Waker {
            event: sys::auto_reset_event()?,
        })
    }

    /// End the current or next [`ConsoleServer::wait`]. Never blocks.
    pub fn wake(&self) {
        // `SetEvent` fails only for an invalid handle, which this type
        // rules out; there is nothing a caller could do about it anyway.
        let _ = sys::set_event(&self.event);
    }
}

fn completion<'a>(c: &IoComplete<'a>) -> sys::Completion<'a> {
    sys::Completion {
        id: c.id,
        status: c.status,
        information: c.information,
        write_back: c.write_back,
    }
}

impl ConsoleServer {
    /// Start the overlapped READ_IO, completing `reply` in the same IOCTL
    /// when given. Returns at once.
    ///
    /// `WouldBlock` while a read is in flight, and nothing is sent.
    /// `InvalidInput` when `reply.write_back` exceeds its packet's
    /// output size, `NotFound` when its packet isn't pending; nothing is
    /// sent either way. `BrokenPipe` once the driver has dropped the server.
    pub fn submit_read(&mut self, reply: Option<&IoComplete<'_>>) -> Result<()> {
        let reply = reply.map(completion);
        self.inner.submit_read(reply.as_ref())
    }

    /// Block until the in-flight read completes, `waker` fires, `exit` is
    /// signalled, or `deadline` passes, then report a snapshot of what is
    /// ready, with no priority between sources.
    ///
    /// `deadline: None` waits without limit; a deadline already past makes
    /// this a non-blocking poll. The deadline is absolute, so a wait cut
    /// short by a wake never extends it. A [`Ready`] with neither field
    /// set means the waker or the deadline ended the wait. A read that
    /// completed with an error is returned as that error, and the read is
    /// no longer in flight: `Interrupted` when [`ConsoleServer::cancel_read`]
    /// stopped it (a cancel can lose the race, and the read then completes
    /// normally), `BrokenPipe` once the driver has dropped the server. `exit` stays
    /// signalled, so the next call reports it again; stop passing it once
    /// it has been handled.
    pub fn wait(
        &mut self,
        waker: &Waker,
        exit: Option<&OwnedWinHandle>,
        deadline: Option<Instant>,
    ) -> Result<Ready> {
        let s = self.inner.wait(&waker.event, exit, deadline)?;
        Ok(Ready {
            io: s.io.map(|d| IoDescriptor {
                id: d.id,
                process: d.process,
                object: d.object,
                function: d.function,
                input_size: d.input_size,
                output_size: d.output_size,
            }),
            exited: s.exited,
        })
    }

    /// `CancelIoEx` on the in-flight read, or a no-op when none is. It
    /// does not wait: the read's completion, whatever its status, still
    /// arrives through [`ConsoleServer::wait`].
    pub fn cancel_read(&mut self) {
        self.inner.cancel_read();
    }

    /// Complete one held-back packet without reading the next, such as a
    /// ReadConsole that waited for input. Errors as for
    /// [`ConsoleServer::submit_read`]'s `reply`.
    pub fn complete_io(&mut self, c: &IoComplete<'_>) -> Result<()> {
        self.inner.complete_io(&completion(c))
    }

    /// Copy the packet's input, from `offset`, into `buf`. Returns the byte
    /// count, short only at the end of the payload. `InvalidInput` past
    /// `input_size`; `NotFound` once the packet is complete.
    pub fn read_input(&mut self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize> {
        self.inner.read_input(id, offset, buf)
    }

    /// Write `data` into the packet's output at `offset`. `InvalidInput`
    /// past `output_size`; `NotFound` once the packet is complete.
    pub fn write_output(&mut self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()> {
        self.inner.write_output(id, offset, data)
    }

    /// Set or reset the manual-reset input-available event clients wait
    /// on. Idempotent.
    pub fn set_input_available(&mut self, on: bool) -> Result<()> {
        self.inner.set_input_available(on)
    }

    /// Close the server. An in-flight read is cancelled and waited for, up
    /// to 1 s; past that its storage is leaked rather than freed while the
    /// kernel may still write it. Then the driver disconnects every
    /// client. The same happens on drop.
    pub fn close(self) {
        drop(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::proc;
    use platform::process::ExitStatus;

    /// A child attached to a server nobody serves: `ping -n 30` blocks in
    /// its CONNECT, so it runs until terminated. Returns the server, the
    /// child's process handle and pid, and a second handle to watch it.
    fn running_child() -> (sys::Server, OwnedWinHandle, u32, OwnedWinHandle) {
        let cwd = std::env::current_dir().expect("current_dir");
        let mut cmd = Command::new("ping.exe", cwd);
        cmd.argv = ["-n", "30", "127.0.0.1"].iter().map(Into::into).collect();
        let resolved = crate::WindowsSpawner
            .resolve(&cmd.program)
            .expect("resolve");
        let args: Vec<&OsStr> = cmd.argv.iter().map(OsString::as_os_str).collect();
        let line = crate::winargv::build_command_line(&resolved, &args).expect("command line");
        let server = sys::Server::create().expect("Server::create");
        let (process, pid) = sys::spawn(&server, &line, &cmd.cwd, &cmd.env).expect("spawn");
        let probe = handle::duplicate(&process, false).expect("probe handle");
        assert_eq!(proc::try_wait(&probe).expect("try_wait"), None);
        (server, process, pid, probe)
    }

    fn injected(kind: ErrorKind, op: &'static str) -> PlatformError {
        PlatformError::new(kind, OsCode::None, op)
    }

    fn production_rollback(p: &OwnedWinHandle) -> Result<()> {
        terminate_and_reap(p, proc::terminate_process, REAP_GRACE)
    }

    /// Failure injected at the one fallible step after `CreateProcessW`:
    /// the child is terminated and reaped, and setup's error kept.
    #[test]
    fn a_setup_failure_after_create_terminates_the_child() {
        let (server, process, pid, probe) = running_child();
        let Err(e) = finish_spawn(
            server,
            process,
            pid,
            |_| Err(injected(ErrorKind::Other, "setup")),
            production_rollback,
        ) else {
            panic!("finish_spawn succeeded despite the injected failure");
        };
        assert_eq!((e.kind, e.op), (ErrorKind::Other, "setup"));
        // Already reaped: no wait is needed to see the exit.
        assert_eq!(
            proc::try_wait(&probe).expect("try_wait"),
            Some(ExitStatus::Code(1)),
            "the child outlived the failed spawn"
        );
    }

    /// A rollback that can't confirm the exit reports its own error, not
    /// setup's, and the child is left to the keeper thread, not released.
    #[test]
    fn an_unconfirmed_rollback_reports_the_cleanup_error() {
        let (server, process, pid, probe) = running_child();
        let Err(e) = finish_spawn(
            server,
            process,
            pid,
            |_| Err(injected(ErrorKind::Other, "setup")),
            |_| Err(injected(ErrorKind::TimedOut, "cleanup")),
        ) else {
            panic!("finish_spawn succeeded despite the injected failure");
        };
        assert_eq!((e.kind, e.op), (ErrorKind::TimedOut, "cleanup"));
        assert_eq!(proc::try_wait(&probe).expect("try_wait"), None);
        // The keeper thread holds the child until it exits; end it here.
        proc::terminate_process(&probe).expect("terminate");
        proc::wait(&probe).expect("wait");
    }

    /// `terminate_and_reap`'s outcomes, each forced deterministically.
    #[test]
    fn terminate_and_reap_confirms_or_reports() {
        // Terminated, then not exited within the grace: `TimedOut`. The
        // injected terminate does nothing, so the child keeps running.
        let (_server, process, _, probe) = running_child();
        let e = terminate_and_reap(&process, |_| Ok(()), Duration::from_millis(100))
            .expect_err("a running child was reported reaped");
        assert_eq!(e.kind, ErrorKind::TimedOut);

        // A failed terminate while it still runs: that error.
        let e = terminate_and_reap(
            &process,
            |_| Err(injected(ErrorKind::PermissionDenied, "terminate")),
            REAP_GRACE,
        )
        .expect_err("a failed terminate was reported as a reap");
        assert_eq!((e.kind, e.op), (ErrorKind::PermissionDenied, "terminate"));

        // A failed terminate on a child that has already exited: reaped.
        proc::terminate_process(&probe).expect("terminate");
        proc::wait(&probe).expect("wait");
        terminate_and_reap(
            &process,
            |_| Err(injected(ErrorKind::PermissionDenied, "terminate")),
            REAP_GRACE,
        )
        .expect("an exited child counts as reaped");

        // And the real path on an exited child.
        terminate_and_reap(&process, proc::terminate_process, REAP_GRACE)
            .expect("terminating an exited child");
    }
}
