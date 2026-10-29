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
use std::time::Instant;

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
/// and inherits nothing else. It always starts with Ctrl+C enabled: the
/// "ignore Ctrl+C" flag an ancestor's `CREATE_NEW_PROCESS_GROUP` leaves
/// on this process is cleared, process-wide, before the spawn, as for a
/// fresh console. Returns once `CreateProcessW` succeeds, so
/// the child's CONNECT is the first packet [`ConsoleServer::wait`]
/// reports. `GroupSpec::JoinGroup` gives `InvalidInput`. On failure every
/// handle is closed and no child is left running.
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
    let wait = handle::duplicate(&process, false)?;
    Ok(Attached {
        server: ConsoleServer { inner: server },
        child: WindowsChild::from_parts(process, None, pid),
        wait,
    })
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
    /// no longer in flight: `Interrupted` after [`ConsoleServer::cancel_read`],
    /// `BrokenPipe` once the driver has dropped the server. `exit` stays
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
