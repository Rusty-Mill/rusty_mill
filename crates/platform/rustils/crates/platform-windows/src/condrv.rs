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

use std::sync::PoisonError;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{Command, GroupSpec};

use crate::process::WindowsChild;
use crate::sys::condrv as sys;
use crate::sys::handle::{self, OwnedWinHandle};

mod cleanup;

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
/// Before `CreateProcessW`, a slot is reserved in a process-wide pool
/// of 8 for a child whose rollback can't be confirmed. With every slot
/// held, this fails with `WouldBlock` and creates nothing; a failure
/// before the child exists closes every handle.
///
/// On a failure after, the child is terminated and the error returned once
/// it has exited, every handle closed. If its exit can't be confirmed
/// within 5 s, or `TerminateProcess` fails while it runs, that cleanup
/// error is returned instead (`TimedOut`, or the OS error), and the
/// reserved slot keeps the child's process handle and console open until
/// it has exited. Nothing here waits longer than that.
pub fn spawn_attached(cmd: &Command) -> Result<Attached> {
    if matches!(cmd.group, GroupSpec::JoinGroup(_)) {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "condrv::spawn_attached: GroupSpec::JoinGroup is incompatible with a fresh console",
        ));
    }
    let line = crate::cmdline::command_line(cmd)?;

    let _serial = crate::pty::SPAWN
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    attach(
        &cleanup::global()?,
        || {
            let server = sys::Server::create()?;
            let (process, pid) = sys::spawn(&server, &line, &cmd.cwd, &cmd.env)?;
            Ok((server, process, pid))
        },
        |p| handle::duplicate(p, false),
        |p| terminate_and_reap(p, crate::sys::proc::terminate_process, REAP_GRACE),
    )
}

/// Reserve a cleanup slot, `create` the child, then finish the remaining
/// fallible setup or roll the child back. Without a slot nothing is
/// created. When `rollback` confirms the child has exited, setup's error
/// is returned and every handle is closed here. When it can't, its own
/// error is returned instead, and the slot takes the child's process
/// handle and console until the child exits.
fn attach(
    pool: &cleanup::Pool,
    create: impl FnOnce() -> Result<(sys::Server, OwnedWinHandle, u32)>,
    duplicate: impl FnOnce(&OwnedWinHandle) -> Result<OwnedWinHandle>,
    rollback: impl FnOnce(&OwnedWinHandle) -> Result<()>,
) -> Result<Attached> {
    let slot = pool.reserve()?;
    let (server, process, pid) = create()?;
    let wait = match duplicate(&process) {
        Ok(wait) => wait,
        Err(setup) => {
            if let Err(cleanup) = rollback(&process) {
                slot.hand_off(process, server);
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

/// `STATUS_CONTROL_C_EXIT`: the code Windows ends a process with once
/// its CTRL_CLOSE handler returns, so a client ended by
/// [`ClientProcess::terminate`] exits as one that CTRL_CLOSE ended.
const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;

/// A console client, held so the host can end it if it is still attached
/// when the console closes (#1259, design §7.3). The handle has
/// `PROCESS_TERMINATE` and `SYNCHRONIZE` access and nothing else.
pub struct ClientProcess {
    handle: OwnedWinHandle,
}

impl ClientProcess {
    /// Open the client `pid` whose CONNECT came from thread `tid` (the
    /// descriptor's `object`), while that CONNECT is unanswered. ConDrv
    /// doesn't keep a waiting client referenced, so a client that died
    /// before this call may have freed its pid for another process
    /// (design §7.3). The thread is opened first and must belong to `pid`;
    /// its handle keeps the process from being freed until the process
    /// handle is open, which then keeps the pid. That the thread is still
    /// the CONNECT's own, not a reuse of its id, the caller proves by
    /// delivering the CONNECT reply afterwards (design §7.3). Fails closed: an
    /// `InvalidInput` error when the thread is another process's, else the
    /// OS error, e.g. when the thread is gone or `PermissionDenied` for a
    /// protected process.
    pub fn open(pid: u32, tid: u32) -> Result<ClientProcess> {
        let thread = sys::open_thread(tid)?;
        if sys::process_of_thread(&thread)? != pid {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "ClientProcess::open: the CONNECT thread is not the client's",
            ));
        }
        let handle = sys::open_process(pid)?;
        drop(thread);
        Ok(ClientProcess { handle })
    }

    /// End the client with `STATUS_CONTROL_C_EXIT` if it is still
    /// running; a client that has exited is left as it is, with `Ok`. It
    /// doesn't wait for the exit. Fails with the OS error.
    pub fn terminate(&self) -> Result<()> {
        if sys::has_exited(&self.handle) {
            return Ok(());
        }
        sys::terminate(&self.handle, STATUS_CONTROL_C_EXIT)
    }
}

/// The key that types `ch` on the active keyboard layout, as conhost
/// synthesizes input records: `(vk, dwControlKeyState)`, the shift state
/// as SHIFT_PRESSED, LEFT_CTRL_PRESSED and LEFT_ALT_PRESSED. `None` when
/// no key types it, or it is outside the BMP. Wraps `VkKeyScanW`.
pub fn key_for(ch: char) -> Option<(u16, u32)> {
    let unit = u16::try_from(u32::from(ch)).ok()?;
    let scan = u16::try_from(sys::key_scan(unit)).ok()?;
    Some((scan & 0xff, key_state(scan >> 8)))
}

/// `VkKeyScanW`'s shift byte (1 Shift, 2 Ctrl, 4 Alt) as key state.
fn key_state(shift: u16) -> u32 {
    const SHIFT_PRESSED: u32 = 0x10;
    const LEFT_CTRL_PRESSED: u32 = 0x08;
    const LEFT_ALT_PRESSED: u32 = 0x02;
    [
        (1, SHIFT_PRESSED),
        (2, LEFT_CTRL_PRESSED),
        (4, LEFT_ALT_PRESSED),
    ]
    .into_iter()
    .filter(|(bit, _)| shift & bit != 0)
    .fold(0, |state, (_, flag)| state | flag)
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
        let line = crate::cmdline::command_line(&cmd).expect("command line");
        let server = sys::Server::create().expect("Server::create");
        let (process, pid) = sys::spawn(&server, &line, &cmd.cwd, &cmd.env).expect("spawn");
        let probe = handle::duplicate(&process, false).expect("probe handle");
        assert_eq!(proc::try_wait(&probe).expect("try_wait"), None);
        (server, process, pid, probe)
    }

    #[test]
    fn key_for_maps_the_shift_byte_and_refuses_what_no_key_types() {
        assert_eq!(key_state(0), 0);
        assert_eq!(key_state(1), 0x10);
        assert_eq!(key_state(2 | 4), 0x08 | 0x02);
        assert_eq!(key_for(' '), Some((0x20, 0)));
        assert_eq!(key_for('\u{1F600}'), None);
    }

    fn injected(kind: ErrorKind, op: &'static str) -> PlatformError {
        PlatformError::new(kind, OsCode::None, op)
    }

    fn production_rollback(p: &OwnedWinHandle) -> Result<()> {
        terminate_and_reap(p, proc::terminate_process, REAP_GRACE)
    }

    /// A pool with a real worker thread, which ends with the pool.
    fn pool(capacity: usize) -> cleanup::Pool {
        cleanup::Pool::start(
            capacity,
            Box::new(|w| std::thread::Builder::new().spawn(w).map(drop)),
        )
        .expect("cleanup pool")
    }

    type Child = Result<(sys::Server, OwnedWinHandle, u32)>;

    /// A `create` that records whether it ran, and must not.
    fn must_not_create(ran: &std::cell::Cell<bool>) -> impl FnOnce() -> Child + '_ {
        move || {
            ran.set(true);
            Err(injected(ErrorKind::Other, "created"))
        }
    }

    fn setup_fails(_: &OwnedWinHandle) -> Result<OwnedWinHandle> {
        Err(injected(ErrorKind::Other, "setup"))
    }

    fn unconfirmed(_: &OwnedWinHandle) -> Result<()> {
        Err(injected(ErrorKind::TimedOut, "cleanup"))
    }

    fn end(probe: &OwnedWinHandle) {
        proc::terminate_process(probe).expect("terminate");
        proc::wait(probe).expect("wait");
    }

    /// Without a slot, from a full pool or a stopped worker, nothing is
    /// created.
    #[test]
    fn no_slot_means_no_child() {
        let full = pool(1);
        let _taken = full.reserve().expect("the only slot");
        let ran = std::cell::Cell::new(false);
        let e = attach(&full, must_not_create(&ran), setup_fails, unconfirmed)
            .err()
            .expect("an attach without a slot");
        assert_eq!(e.kind, ErrorKind::WouldBlock);
        assert!(!ran.get(), "a child was created without a slot");

        let stopped = cleanup::Pool::start(
            1,
            Box::new(|w| {
                drop(w);
                Ok(())
            }),
        )
        .expect("pool");
        let e = attach(&stopped, must_not_create(&ran), setup_fails, unconfirmed)
            .err()
            .expect("an attach with a stopped worker");
        assert_eq!(e.kind, ErrorKind::Other);
        assert!(!ran.get(), "a child was created with a stopped worker");
    }

    /// A successful spawn gives its slot back.
    #[test]
    fn a_successful_attach_releases_its_slot() {
        let slots = pool(1);
        let (server, process, pid, probe) = running_child();
        let attached = attach(
            &slots,
            move || Ok((server, process, pid)),
            |p| handle::duplicate(p, false),
            |_| panic!("rolled back a successful spawn"),
        );
        assert!(attached.is_ok(), "attach failed");
        assert_eq!(slots.reserved(), 0);
        end(&probe);
    }

    /// Failure injected at the one fallible step after `CreateProcessW`:
    /// the child is terminated and reaped, setup's error kept, and the
    /// slot given back.
    #[test]
    fn a_setup_failure_after_create_terminates_the_child() {
        let slots = pool(1);
        let (server, process, pid, probe) = running_child();
        let Err(e) = attach(
            &slots,
            move || Ok((server, process, pid)),
            setup_fails,
            production_rollback,
        ) else {
            panic!("attach succeeded despite the injected failure");
        };
        assert_eq!((e.kind, e.op), (ErrorKind::Other, "setup"));
        // Already reaped: no wait is needed to see the exit.
        assert_eq!(
            proc::try_wait(&probe).expect("try_wait"),
            Some(ExitStatus::Code(1)),
            "the child outlived the failed spawn"
        );
        assert_eq!(slots.reserved(), 0);
    }

    /// A rollback that can't confirm the exit reports its own error, and
    /// the child goes to its reserved slot, which stays taken until the
    /// child exits. A stuck child per slot stops further spawns before
    /// they create anything.
    #[test]
    fn an_unconfirmed_rollback_hands_the_child_to_its_slot() {
        let slots = pool(1);
        let (server, process, pid, probe) = running_child();
        let Err(e) = attach(
            &slots,
            move || Ok((server, process, pid)),
            setup_fails,
            unconfirmed,
        ) else {
            panic!("attach succeeded despite the injected failure");
        };
        assert_eq!((e.kind, e.op), (ErrorKind::TimedOut, "cleanup"));
        assert_eq!((slots.held(), slots.reserved()), (1, 1));
        assert_eq!(proc::try_wait(&probe).expect("try_wait"), None);

        let ran = std::cell::Cell::new(false);
        for _ in 0..3 {
            let e = attach(&slots, must_not_create(&ran), setup_fails, unconfirmed)
                .err()
                .expect("an attach past capacity");
            assert_eq!(e.kind, ErrorKind::WouldBlock);
        }
        assert!(!ran.get(), "a child was created past capacity");

        end(&probe);
        let deadline = Instant::now() + Duration::from_secs(10);
        while slots.reserved() != 0 {
            assert!(Instant::now() < deadline, "the exited child kept its slot");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(slots.held(), 0);
        slots.reserve().expect("the freed slot");
    }

    /// A worker that stops after a hand-off keeps the child's handles:
    /// nothing is released while the child still runs.
    #[test]
    fn a_stopped_worker_keeps_a_handed_off_child() {
        let worker = std::sync::Arc::new(std::sync::Mutex::new(None));
        let park = std::sync::Arc::clone(&worker);
        let slots = cleanup::Pool::start(
            1,
            Box::new(move |w| {
                *park.lock().expect("park") = Some(w);
                Ok(())
            }),
        )
        .expect("pool");
        let (server, process, pid, probe) = running_child();
        let e = attach(
            &slots,
            move || Ok((server, process, pid)),
            setup_fails,
            unconfirmed,
        )
        .err()
        .expect("an unconfirmed rollback");
        assert_eq!(e.kind, ErrorKind::TimedOut);
        drop(worker.lock().expect("park").take());
        assert_eq!((slots.held(), slots.reserved()), (1, 1));
        assert_eq!(proc::try_wait(&probe).expect("try_wait"), None);
        end(&probe);
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
