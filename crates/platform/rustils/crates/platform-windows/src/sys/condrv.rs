//! Console driver (ConDrv) server primitives for our own console host
//! (#1222, PR 1; design `docs/projects/term/conhost/design.md` §4.1 and
//! §11.2.1): open a server, attach a child to it, and serve its packets
//! with overlapped IOCTLs. [`crate::condrv`] is the public face; this
//! module holds the `unsafe`.
//!
//! Layouts and constants are transcribed from microsoft/terminal at commit
//! 09beddc6 (`dep/Console/condrv.h`, `src/server/DeviceHandle.cpp`,
//! `src/server/Entrypoints.cpp`, `src/server/winbasep.h`,
//! `src/interactivity/win32/ConsoleControl.cpp`,
//! `src/interactivity/inc/IConsoleControl.hpp`):
//!
//! ```text
//! Copyright (c) Microsoft Corporation.
//! Licensed under the MIT license.
//! ```
//!
//! **One in-flight READ_IO, owned until completion.** The OVERLAPPED block
//! and the buffers a READ_IO points at live in a [`ReadSlot`] on the heap,
//! at a stable address, from submission until its completion has been
//! observed. `CancelIoEx` only requests cancellation, so neither a cancel
//! nor a drop frees the slot early: [`Drop for Server`] waits a bounded
//! time for the completion and leaks the slot rather than free memory the
//! kernel may still write.
//!
//! **Every IOCTL is overlapped.** The server handle is asynchronous (open
//! options 0, as conhost opens it), so the synchronous-looking calls
//! (COMPLETE_IO, READ_INPUT, WRITE_OUTPUT, SET_SERVER_INFORMATION) issue
//! an overlapped call on a private event and wait for it. ConDrv answers
//! those at once.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{c_void, OsStr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::EnvSpec;

use crate::ffi::nt_surface as nt;
use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::sys::handle::{self, OwnedWinHandle};
use crate::sys::nt as sysnt;
use crate::sys::proc;
use crate::util::wide::to_wide_nul;

/// `CTL_CODE(FILE_DEVICE_CONSOLE, function, method, FILE_ANY_ACCESS)`.
const fn ctl(function: u32, method: u32) -> u32 {
    const FILE_DEVICE_CONSOLE: u32 = 0x50;
    (FILE_DEVICE_CONSOLE << 16) | (function << 2) | method
}
const METHOD_OUT_DIRECT: u32 = 2;
const METHOD_NEITHER: u32 = 3;
const IOCTL_CONDRV_READ_IO: u32 = ctl(1, METHOD_OUT_DIRECT);
const IOCTL_CONDRV_COMPLETE_IO: u32 = ctl(2, METHOD_NEITHER);
const IOCTL_CONDRV_READ_INPUT: u32 = ctl(3, METHOD_NEITHER);
const IOCTL_CONDRV_WRITE_OUTPUT: u32 = ctl(4, METHOD_NEITHER);
const IOCTL_CONDRV_SET_SERVER_INFORMATION: u32 = ctl(7, METHOD_NEITHER);

/// `ProcThreadAttributeValue(10, FALSE, TRUE, FALSE)` (`winbasep.h`):
/// the attribute that attaches a new process to a console server through
/// its `\Reference` handle. windows-sys doesn't ship it.
const PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE: usize = 0x0002_000A;

/// `ControlType::ConsoleEndTask` (`IConsoleControl.hpp`).
const CONSOLE_END_TASK: u32 = 7;

/// Bytes READ_IO may write: the descriptor plus the inline copy of the
/// packet's leading input. The spike's 4 KiB inline area covered every
/// packet it saw; the host reads payloads through READ_INPUT, never from
/// this copy.
const READ_OUT_LEN: usize = std::mem::size_of::<IoDescriptorRaw>() + 4096;

/// How long [`Drop for Server`] waits for a cancelled READ_IO to complete
/// before leaking its slot (design §7.3).
const CANCEL_GRACE_MS: u32 = 1000;

/// `LUID`, kept as the driver's opaque packet identifier.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Luid {
    low: u32,
    high: i32,
}

impl Luid {
    fn to_bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&self.low.to_ne_bytes());
        b[4..].copy_from_slice(&self.high.to_ne_bytes());
        b
    }

    fn from_bytes(b: [u8; 8]) -> Self {
        Self {
            low: u32::from_ne_bytes([b[0], b[1], b[2], b[3]]),
            high: i32::from_ne_bytes([b[4], b[5], b[6], b[7]]),
        }
    }
}

/// `CD_IO_DESCRIPTOR`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoDescriptorRaw {
    identifier: Luid,
    process: usize,
    object: usize,
    function: u32,
    input_size: u32,
    output_size: u32,
    reserved: u32,
}

/// `IO_STATUS_BLOCK`: an NTSTATUS in a pointer-sized union, then the
/// byte count. The union is spelled out here because only the status arm
/// is ever written.
#[repr(C)]
#[derive(Clone, Copy)]
union IoStatusUnion {
    status: i32,
    pointer: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IoStatusBlock {
    u: IoStatusUnion,
    information: usize,
}

/// `CD_IO_BUFFER_DESCRIPTOR`.
#[repr(C)]
#[derive(Clone, Copy)]
struct IoBufferDescriptor {
    data: *mut c_void,
    size: u32,
    offset: u32,
}

/// `CD_IO_COMPLETE`.
#[repr(C)]
#[derive(Clone, Copy)]
struct IoCompleteRaw {
    identifier: Luid,
    io_status: IoStatusBlock,
    write: IoBufferDescriptor,
}

/// `CD_IO_OPERATION`.
#[repr(C)]
struct IoOperation {
    identifier: Luid,
    buffer: IoBufferDescriptor,
}

/// `CD_IO_SERVER_INFORMATION`.
#[repr(C)]
struct ServerInformation {
    input_available_event: w::HANDLE,
}

/// `CONSOLEENDTASK`: the pid travels in a HANDLE-sized field.
#[repr(C)]
struct ConsoleEndTask {
    process_id: usize,
    hwnd: usize,
    console_event_code: u32,
    console_flags: u32,
}

/// A packet READ_IO delivered, as plain values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub id: [u8; 8],
    pub process: usize,
    pub object: usize,
    pub function: u32,
    pub input_size: u32,
    pub output_size: u32,
}

/// One completion to send: the packet, its status and information, and
/// the fixed reply struct written back through `CD_IO_COMPLETE.Write`.
pub struct Completion<'a> {
    pub id: [u8; 8],
    pub status: i32,
    pub information: usize,
    pub write_back: &'a [u8],
}

/// The in-flight READ_IO's storage. Boxed so its address is stable while
/// the kernel holds pointers into it.
struct ReadSlot {
    overlapped: w::OVERLAPPED,
    event: OwnedWinHandle,
    complete: IoCompleteRaw,
    write_back: Vec<u8>,
    out: [u8; READ_OUT_LEN],
}

/// The readiness snapshot [`Server::wait`] returns.
pub struct Snapshot {
    pub io: Option<Descriptor>,
    pub exited: bool,
}

/// An open console server: the driver's server object, its `\Reference`
/// handle, the input-available event, and the READ_IO slot.
pub struct Server {
    server: OwnedWinHandle,
    reference: OwnedWinHandle,
    input_event: OwnedWinHandle,
    sync_event: OwnedWinHandle,
    slot: Option<Box<ReadSlot>>,
    in_flight: bool,
    broken: bool,
    /// Packets delivered and not yet completed: `(input_size, output_size)`.
    pending: HashMap<[u8; 8], (u32, u32)>,
}

// SAFETY: the raw pointers inside `ReadSlot` (the OVERLAPPED block and the
// buffer descriptors) point only into the slot itself or into its own
// `write_back` vector, both owned by this value; the HANDLEs are
// thread-affinity-free (`OwnedWinHandle`'s own `Send` reasoning). Moving
// the whole server to another thread moves the `Box`, not the heap
// storage the kernel writes, so the addresses it holds stay valid. Not
// `Sync`: every operation takes `&mut self`, one owner at a time.
unsafe impl Send for Server {}

fn new_event(manual_reset: bool) -> Result<OwnedWinHandle> {
    // SAFETY: null security attributes and a null name are documented-valid;
    // the call has no other preconditions.
    let raw = unsafe {
        w::CreateEventW(
            std::ptr::null(),
            i32::from(manual_reset),
            0,
            std::ptr::null(),
        )
    };
    // SAFETY: `raw` is a failure sentinel or a fresh handle nothing else owns.
    unsafe { OwnedWinHandle::from_raw(raw) }
        .ok_or_else(|| errmap::last_win32_err("CreateEventW", OsStr::new("")))
}

/// An auto-reset event, for [`crate::condrv::Waker`].
pub fn auto_reset_event() -> Result<OwnedWinHandle> {
    new_event(false)
}

/// `SetEvent`.
pub fn set_event(event: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `event` is a valid, open event handle for the call.
    if unsafe { w::SetEvent(event.as_raw()) } == 0 {
        return Err(errmap::last_win32_err("SetEvent", OsStr::new("")));
    }
    Ok(())
}

fn reset_event(event: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `event` is a valid, open event handle for the call.
    if unsafe { w::ResetEvent(event.as_raw()) } == 0 {
        return Err(errmap::last_win32_err("ResetEvent", OsStr::new("")));
    }
    Ok(())
}

/// Milliseconds left until `deadline`, rounded up and kept below `INFINITE`.
fn timeout_ms(deadline: Instant) -> u32 {
    let left = deadline.saturating_duration_since(Instant::now());
    let ms = left.as_micros().div_ceil(1000);
    ms.min(u128::from(w::INFINITE - 1)) as u32
}

fn is_signalled(h: w::HANDLE) -> bool {
    // SAFETY: callers pass a handle they hold open for the call.
    unsafe { w::WaitForSingleObject(h, 0) == w::WAIT_OBJECT_0 }
}

/// Wait up to `timeout` for any of `raw`, which may mix events and
/// processes: `Some(index)` of a signalled handle, `None` on timeout.
/// Unlike `proc::wait_many`, nothing is queried about the handle that
/// fired. Callers hold every handle open for the call.
pub fn wait_any(raw: &[w::HANDLE], timeout: Duration) -> Result<Option<usize>> {
    if raw.is_empty() || raw.len() > 64 {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "WaitForMultipleObjects: 1 to 64 handles",
        ));
    }
    let ms = timeout.as_millis().min(u128::from(w::INFINITE - 1)) as u32;
    // SAFETY: `raw` holds 1 to 64 handles the caller keeps open for the
    // call.
    let r = unsafe { w::WaitForMultipleObjects(raw.len() as u32, raw.as_ptr(), 0, ms) };
    let index = r.wrapping_sub(w::WAIT_OBJECT_0) as usize;
    match r {
        w::WAIT_TIMEOUT => Ok(None),
        w::WAIT_FAILED => Err(errmap::last_win32_err(
            "WaitForMultipleObjects",
            OsStr::new(""),
        )),
        _ if index < raw.len() => Ok(Some(index)),
        _ => Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::Win32(r),
            "WaitForMultipleObjects: unexpected result",
        )),
    }
}

/// Whether `handle` is signalled now. A failed check is an error, not
/// `false`.
pub fn signalled(handle: &OwnedWinHandle) -> Result<bool> {
    Ok(wait_any(&[handle.as_raw()], Duration::ZERO)?.is_some())
}

/// Classify a failed ConDrv IOCTL: a driver that has dropped the server
/// is `BrokenPipe`, a cancelled read `Interrupted`, the rest by the
/// shared Win32 table.
fn ioctl_err(code: u32, op: &'static str) -> PlatformError {
    let kind = match code {
        w::ERROR_BROKEN_PIPE | w::ERROR_PIPE_NOT_CONNECTED => ErrorKind::BrokenPipe,
        w::ERROR_OPERATION_ABORTED => ErrorKind::Interrupted,
        _ => errmap::kind_of_win32(code),
    };
    PlatformError::new(kind, OsCode::Win32(code), op)
}

fn last_error() -> u32 {
    // SAFETY: `GetLastError` takes no arguments and has no preconditions.
    unsafe { w::GetLastError() }
}

fn invalid(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::InvalidInput, OsCode::None, op)
}

/// Open `\Device\ConDrv\Server`, creating a new console server. No
/// privilege is needed. The handle is asynchronous (options 0).
pub fn open_server() -> Result<OwnedWinHandle> {
    sysnt::open_absolute(
        OsStr::new(r"\Device\ConDrv\Server"),
        w::FILE_ALL_ACCESS,
        nt::FILE_OPEN,
        0,
    )
}

/// Open one of the server's client objects (`\Reference`, `\Input`,
/// `\Output`) relative to the server handle, synchronous, as conhost's
/// `DeviceHandle::CreateClientHandle` does.
fn open_client(server: &OwnedWinHandle, name: &str) -> Result<OwnedWinHandle> {
    sysnt::open_relative(
        server,
        OsStr::new(name),
        w::GENERIC_READ | w::GENERIC_WRITE | w::SYNCHRONIZE,
        nt::FILE_OPEN,
        nt::FILE_SYNCHRONOUS_IO_NONALERT,
    )
}

impl Server {
    /// Open a server and its `\Reference`, and register the
    /// input-available event with SET_SERVER_INFORMATION.
    pub fn create() -> Result<Self> {
        let server = open_server()?;
        let reference = open_client(&server, r"\Reference")?;
        let input_event = new_event(true)?;
        let mut this = Self {
            server,
            reference,
            input_event,
            sync_event: new_event(true)?,
            slot: None,
            in_flight: false,
            broken: false,
            pending: HashMap::new(),
        };
        let mut info = ServerInformation {
            input_available_event: this.input_event.as_raw(),
        };
        this.ioctl_sync(
            IOCTL_CONDRV_SET_SERVER_INFORMATION,
            (&mut info as *mut ServerInformation).cast(),
            std::mem::size_of::<ServerInformation>(),
            "SET_SERVER_INFORMATION",
        )?;
        Ok(this)
    }

    /// An overlapped IOCTL with no output buffer, waited for on the
    /// private event.
    fn ioctl_sync(
        &mut self,
        code: u32,
        input: *mut c_void,
        input_len: usize,
        op: &'static str,
    ) -> Result<()> {
        if self.broken {
            return Err(PlatformError::new(ErrorKind::BrokenPipe, OsCode::None, op));
        }
        // SAFETY: OVERLAPPED is plain-old-data for which all-zeroes is valid.
        let mut ov: w::OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.sync_event.as_raw();
        let mut n: u32 = 0;
        // SAFETY: `input` points at `input_len` readable bytes the caller
        // keeps alive until this function returns, which is after the
        // operation has completed (`GetOverlappedResult` with bWait);
        // `ov` and `n` outlive the call for the same reason; the server
        // handle is open.
        let ok = unsafe {
            w::DeviceIoControl(
                self.server.as_raw(),
                code,
                input,
                input_len as u32,
                std::ptr::null_mut(),
                0,
                &mut n,
                &mut ov,
            )
        };
        if ok == 0 {
            let code = last_error();
            if code != w::ERROR_IO_PENDING {
                return Err(self.note(ioctl_err(code, op)));
            }
            // SAFETY: `ov` is the OVERLAPPED of the operation just started
            // on this handle; waiting completes it before `ov` goes away.
            if unsafe { w::GetOverlappedResult(self.server.as_raw(), &ov, &mut n, 1) } == 0 {
                return Err(self.note(ioctl_err(last_error(), op)));
            }
        }
        Ok(())
    }

    /// Remember a dropped server so later calls fail fast.
    fn note(&mut self, e: PlatformError) -> PlatformError {
        if e.kind == ErrorKind::BrokenPipe {
            self.broken = true;
        }
        e
    }

    /// The `\Reference` handle a child is attached through.
    pub fn reference(&self) -> &OwnedWinHandle {
        &self.reference
    }

    /// Open the client's `\Input` and `\Output` objects.
    pub fn open_std(&self) -> Result<(OwnedWinHandle, OwnedWinHandle)> {
        Ok((
            open_client(&self.server, r"\Input")?,
            open_client(&self.server, r"\Output")?,
        ))
    }

    /// Start the one overlapped READ_IO, completing `reply` in the same
    /// IOCTL when given.
    pub fn submit_read(&mut self, reply: Option<&Completion<'_>>) -> Result<()> {
        if self.in_flight {
            return Err(PlatformError::new(
                ErrorKind::WouldBlock,
                OsCode::None,
                "READ_IO: a read is already in flight",
            ));
        }
        if self.broken {
            return Err(PlatformError::new(
                ErrorKind::BrokenPipe,
                OsCode::None,
                "READ_IO",
            ));
        }
        if let Some(c) = reply {
            self.check_write_back(c)?;
        }
        if self.slot.is_none() {
            self.slot = Some(Box::new(ReadSlot {
                // SAFETY: OVERLAPPED is plain-old-data; all-zeroes is valid.
                overlapped: unsafe { std::mem::zeroed() },
                event: new_event(true)?,
                complete: completion_raw(&Completion {
                    id: [0; 8],
                    status: 0,
                    information: 0,
                    write_back: &[],
                }),
                write_back: Vec::new(),
                out: [0; READ_OUT_LEN],
            }));
        }
        let server = self.server.as_raw();
        let Some(slot) = self.slot.as_deref_mut() else {
            return Err(invalid("READ_IO"));
        };
        reset_event(&slot.event)?;
        // SAFETY: as above.
        slot.overlapped = unsafe { std::mem::zeroed() };
        slot.overlapped.hEvent = slot.event.as_raw();
        let (input, input_len) = match reply {
            Some(c) => {
                slot.write_back.clear();
                slot.write_back.extend_from_slice(c.write_back);
                slot.complete = completion_raw(c);
                slot.complete.write.data = slot.write_back.as_mut_ptr().cast();
                (
                    (&mut slot.complete as *mut IoCompleteRaw).cast::<c_void>(),
                    std::mem::size_of::<IoCompleteRaw>(),
                )
            }
            None => (std::ptr::null_mut(), 0),
        };
        let mut n: u32 = 0;
        // SAFETY: every pointer refers into `slot`, which is heap-allocated
        // at a stable address and stays allocated until this read's
        // completion is observed (`wait`) or the slot is leaked (`Drop`);
        // the server handle is open.
        let ok = unsafe {
            w::DeviceIoControl(
                server,
                IOCTL_CONDRV_READ_IO,
                input,
                input_len as u32,
                slot.out.as_mut_ptr().cast(),
                READ_OUT_LEN as u32,
                &mut n,
                &mut slot.overlapped,
            )
        };
        if ok == 0 {
            let code = last_error();
            if code != w::ERROR_IO_PENDING {
                return Err(self.note(ioctl_err(code, "READ_IO")));
            }
        }
        // Started, or already complete with the event set: either way the
        // completion is collected by `wait`.
        self.in_flight = true;
        if let Some(c) = reply {
            self.pending.remove(&c.id);
        }
        Ok(())
    }

    /// Wait for the in-flight read, `waker`, or `exit`, until `deadline`,
    /// then report everything that is ready.
    pub fn wait(
        &mut self,
        waker: &OwnedWinHandle,
        exit: Option<&OwnedWinHandle>,
        deadline: Option<Instant>,
    ) -> Result<Snapshot> {
        let mut handles: Vec<w::HANDLE> = vec![waker.as_raw()];
        if self.in_flight {
            if let Some(slot) = &self.slot {
                handles.push(slot.event.as_raw());
            }
        }
        if let Some(h) = exit {
            handles.push(h.as_raw());
        }
        // The wait's timer is coarser than `Instant` and can expire a tick
        // early, so wait again until the deadline has really passed.
        loop {
            let ms = deadline.map_or(w::INFINITE, timeout_ms);
            // SAFETY: `handles` holds at most three open handles, borrowed
            // from `waker`, the slot and `exit` for the duration of the call.
            let r =
                unsafe { w::WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, ms) };
            if r == w::WAIT_FAILED {
                return Err(errmap::last_win32_err(
                    "WaitForMultipleObjects",
                    OsStr::new(""),
                ));
            }
            if r != w::WAIT_TIMEOUT || deadline.map_or(true, |d| Instant::now() >= d) {
                break;
            }
        }
        let io = self.take_completion()?;
        let exited = exit.is_some_and(|h| is_signalled(h.as_raw()));
        Ok(Snapshot { io, exited })
    }

    /// Collect the in-flight read's completion if it has completed.
    fn take_completion(&mut self) -> Result<Option<Descriptor>> {
        if !self.in_flight {
            return Ok(None);
        }
        let server = self.server.as_raw();
        let Some(slot) = self.slot.as_deref_mut() else {
            return Ok(None);
        };
        if !is_signalled(slot.event.as_raw()) {
            return Ok(None);
        }
        let mut n: u32 = 0;
        // SAFETY: the slot's OVERLAPPED belongs to the read on this handle,
        // whose event is signalled, so the call doesn't block.
        let ok = unsafe { w::GetOverlappedResult(server, &slot.overlapped, &mut n, 0) };
        self.in_flight = false;
        if ok == 0 {
            return Err(self.note(ioctl_err(last_error(), "READ_IO")));
        }
        if (n as usize) < std::mem::size_of::<IoDescriptorRaw>() {
            return Err(PlatformError::new(
                ErrorKind::Other,
                OsCode::None,
                "READ_IO: short descriptor",
            ));
        }
        // SAFETY: `out` holds at least one descriptor's worth of bytes the
        // driver wrote; `read_unaligned` copes with the array's alignment.
        let raw: IoDescriptorRaw = unsafe { std::ptr::read_unaligned(slot.out.as_ptr().cast()) };
        let d = Descriptor {
            id: raw.identifier.to_bytes(),
            process: raw.process,
            object: raw.object,
            function: raw.function,
            input_size: raw.input_size,
            output_size: raw.output_size,
        };
        self.pending.insert(d.id, (d.input_size, d.output_size));
        Ok(Some(d))
    }

    /// `CancelIoEx` on the in-flight read; a no-op when none is. The
    /// completion still arrives through [`Server::wait`].
    pub fn cancel_read(&mut self) {
        if !self.in_flight {
            return;
        }
        if let Some(slot) = &self.slot {
            // SAFETY: the OVERLAPPED identifies this handle's in-flight read;
            // cancelling a read that already completed is harmless.
            unsafe {
                w::CancelIoEx(self.server.as_raw(), &slot.overlapped);
            }
        }
    }

    fn check_write_back(&self, c: &Completion<'_>) -> Result<()> {
        let Some(&(_, output)) = self.pending.get(&c.id) else {
            return Err(PlatformError::new(
                ErrorKind::NotFound,
                OsCode::None,
                "COMPLETE_IO: no such pending packet",
            ));
        };
        if c.write_back.len() > output as usize {
            return Err(invalid(
                "COMPLETE_IO: write_back exceeds the packet's output size",
            ));
        }
        Ok(())
    }

    /// COMPLETE_IO for one held-back packet.
    pub fn complete_io(&mut self, c: &Completion<'_>) -> Result<()> {
        self.check_write_back(c)?;
        let mut write_back = c.write_back.to_vec();
        let mut raw = completion_raw(c);
        raw.write.data = write_back.as_mut_ptr().cast();
        self.ioctl_sync(
            IOCTL_CONDRV_COMPLETE_IO,
            (&mut raw as *mut IoCompleteRaw).cast(),
            std::mem::size_of::<IoCompleteRaw>(),
            "COMPLETE_IO",
        )?;
        self.pending.remove(&c.id);
        Ok(())
    }

    fn sizes(&self, id: [u8; 8], op: &'static str) -> Result<(u32, u32)> {
        self.pending
            .get(&id)
            .copied()
            .ok_or_else(|| PlatformError::new(ErrorKind::NotFound, OsCode::None, op))
    }

    /// READ_INPUT: copy the packet's input from `offset` into `buf`,
    /// short only at the end of the payload.
    pub fn read_input(&mut self, id: [u8; 8], offset: u32, buf: &mut [u8]) -> Result<usize> {
        let (input, _) = self.sizes(id, "READ_INPUT: no such pending packet")?;
        if offset > input {
            return Err(invalid("READ_INPUT: offset past the packet's input"));
        }
        let len = buf.len().min((input - offset) as usize);
        if len == 0 {
            return Ok(0);
        }
        self.operation(
            IOCTL_CONDRV_READ_INPUT,
            id,
            offset,
            buf.as_mut_ptr(),
            len,
            "READ_INPUT",
        )?;
        Ok(len)
    }

    /// WRITE_OUTPUT: write `data` into the packet's output at `offset`.
    pub fn write_output(&mut self, id: [u8; 8], offset: u32, data: &[u8]) -> Result<()> {
        let (_, output) = self.sizes(id, "WRITE_OUTPUT: no such pending packet")?;
        let end = u64::from(offset) + data.len() as u64;
        if end > u64::from(output) {
            return Err(invalid("WRITE_OUTPUT: data past the packet's output"));
        }
        if data.is_empty() {
            return Ok(());
        }
        // The driver only reads from the buffer; METHOD_NEITHER takes it as
        // a mutable pointer.
        self.operation(
            IOCTL_CONDRV_WRITE_OUTPUT,
            id,
            offset,
            data.as_ptr().cast_mut(),
            data.len(),
            "WRITE_OUTPUT",
        )
    }

    fn operation(
        &mut self,
        code: u32,
        id: [u8; 8],
        offset: u32,
        data: *mut u8,
        len: usize,
        op: &'static str,
    ) -> Result<()> {
        let mut operation = IoOperation {
            identifier: Luid::from_bytes(id),
            buffer: IoBufferDescriptor {
                data: data.cast(),
                size: len as u32,
                offset,
            },
        };
        self.ioctl_sync(
            code,
            (&mut operation as *mut IoOperation).cast(),
            std::mem::size_of::<IoOperation>(),
            op,
        )
    }

    /// Set or reset the input-available event clients wait on.
    pub fn set_input_available(&mut self, on: bool) -> Result<()> {
        if on {
            set_event(&self.input_event)
        } else {
            reset_event(&self.input_event)
        }
    }
}

fn completion_raw(c: &Completion<'_>) -> IoCompleteRaw {
    // Zero the whole pointer-sized union first, so no byte the driver
    // reads is left uninitialized, then store the NTSTATUS.
    let mut u = IoStatusUnion {
        pointer: std::ptr::null_mut(),
    };
    u.status = c.status;
    IoCompleteRaw {
        identifier: Luid::from_bytes(c.id),
        io_status: IoStatusBlock {
            u,
            information: c.information,
        },
        write: IoBufferDescriptor {
            data: std::ptr::null_mut(),
            size: c.write_back.len() as u32,
            offset: 0,
        },
    }
}

impl Drop for Server {
    /// Cancel and drain the in-flight read before the handles close. If the
    /// completion doesn't arrive within [`CANCEL_GRACE_MS`], the slot is
    /// leaked: the kernel may still write into it. Closing the server
    /// handle then disconnects every client.
    fn drop(&mut self) {
        if !self.in_flight {
            return;
        }
        self.cancel_read();
        let Some(slot) = self.slot.take() else {
            return;
        };
        // SAFETY: the slot's event is open; waiting on it has no other
        // precondition.
        let done = unsafe { w::WaitForSingleObject(slot.event.as_raw(), CANCEL_GRACE_MS) }
            == w::WAIT_OBJECT_0;
        if done {
            let mut n: u32 = 0;
            // SAFETY: the read's event is signalled, so this collects the
            // finished operation without blocking; its result is moot.
            unsafe {
                w::GetOverlappedResult(self.server.as_raw(), &slot.overlapped, &mut n, 0);
            }
            drop(slot);
        } else {
            // There is no log facility in this crate; the consumer's
            // runtime records teardown timeouts (design §7.3).
            std::mem::forget(slot);
        }
    }
}

/// Spawn `command_line` attached to `server`: the console reference
/// attribute, plus a handle list holding only the three std handles,
/// which are inheritable duplicates of `\Input`, `\Output`, `\Output`
/// closed again once `CreateProcessW` returns. Returns the process handle
/// and pid.
pub fn spawn(
    server: &Server,
    command_line: &[u16],
    cwd: &OsStr,
    env: &EnvSpec,
) -> Result<(OwnedWinHandle, u32)> {
    let (input, output) = server.open_std()?;
    let std_in = handle::duplicate(&input, true)?;
    let std_out = handle::duplicate(&output, true)?;
    let std_err = handle::duplicate(&output, true)?;
    let mut list = [std_in.as_raw(), std_out.as_raw(), std_err.as_raw()];
    let mut reference = server.reference().as_raw();

    let mut line: Vec<u16> = command_line.to_vec();
    line.push(0);
    let cwd_w = to_wide_nul(cwd);
    let block = proc::env_block(env);

    let attrs = AttrList::new(2)?;
    // CONSOLE_REFERENCE's value is a pointer to a HANDLE holding the
    // reference handle, as `Entrypoints::StartConsoleForCmdLine`
    // passes it; `reference` outlives `CreateProcessW` below.
    attrs.update(
        PROC_THREAD_ATTRIBUTE_CONSOLE_REFERENCE,
        (&mut reference as *mut w::HANDLE).cast(),
        std::mem::size_of::<w::HANDLE>(),
    )?;
    // HANDLE_LIST's value is the array itself; `list` outlives
    // `CreateProcessW` below.
    attrs.update(
        w::PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        list.as_mut_ptr().cast(),
        std::mem::size_of_val(&list),
    )?;

    // SAFETY: STARTUPINFOEXW is plain-old-data; all-zeroes is a valid start.
    let mut siex: w::STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    siex.StartupInfo.cb = std::mem::size_of::<w::STARTUPINFOEXW>() as u32;
    siex.StartupInfo.dwFlags = w::STARTF_USESTDHANDLES;
    siex.StartupInfo.hStdInput = list[0];
    siex.StartupInfo.hStdOutput = list[1];
    siex.StartupInfo.hStdError = list[2];
    siex.lpAttributeList = attrs.raw();

    let mut flags = w::EXTENDED_STARTUPINFO_PRESENT;
    if block.is_some() {
        flags |= w::CREATE_UNICODE_ENVIRONMENT;
    }
    let env_ptr = block
        .as_ref()
        .map_or(std::ptr::null(), |b| b.as_ptr().cast::<c_void>());
    // SAFETY: PROCESS_INFORMATION is plain-old-data, overwritten on success.
    let mut pi: w::PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let si_ptr = (&siex as *const w::STARTUPINFOEXW).cast::<w::STARTUPINFOW>();
    // SAFETY: `line` is an owned, mutable, NUL-terminated buffer; `cwd_w`
    // is NUL-terminated; `env_ptr` is null or a double-NUL block from
    // `proc::env_block`; `si_ptr` is a fully initialized STARTUPINFOEXW
    // whose `cb` is the extended size, with an attribute list whose
    // values all outlive this call; `bInheritHandles = TRUE` passes only
    // the handle list's three handles, which are inheritable.
    let ok = unsafe {
        w::CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            flags,
            env_ptr,
            cwd_w.as_ptr(),
            si_ptr,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("CreateProcessW", cwd));
    }
    // SAFETY: fresh handles from the successful call above, owned by nobody else.
    drop(unsafe { OwnedWinHandle::from_raw(pi.hThread) });
    // SAFETY: as above.
    let process = unsafe { OwnedWinHandle::from_raw(pi.hProcess) }
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreateProcessW"))?;
    Ok((process, pi.dwProcessId))
}

/// A `PROC_THREAD_ATTRIBUTE_LIST`, deleted on drop.
struct AttrList {
    buf: Vec<u8>,
}

impl AttrList {
    fn new(count: u32) -> Result<Self> {
        let mut size: usize = 0;
        // SAFETY: a null list is the documented size query; it reports
        // failure by design and writes only `size`.
        unsafe {
            w::InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size);
        }
        if size == 0 {
            return Err(errmap::last_win32_err(
                "InitializeProcThreadAttributeList",
                OsStr::new(""),
            ));
        }
        let mut buf = vec![0u8; size];
        // SAFETY: `buf` is exactly the size the query reported.
        let ok = unsafe {
            w::InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size)
        };
        if ok == 0 {
            return Err(errmap::last_win32_err(
                "InitializeProcThreadAttributeList",
                OsStr::new(""),
            ));
        }
        Ok(Self { buf })
    }

    fn raw(&self) -> w::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_ptr().cast_mut().cast()
    }

    /// The caller keeps `value` alive until the list's last use.
    fn update(&self, attribute: usize, value: *mut c_void, size: usize) -> Result<()> {
        // SAFETY: the list was initialized in `new`; `value` points at
        // `size` bytes the caller keeps alive (see above).
        let ok = unsafe {
            w::UpdateProcThreadAttribute(
                self.raw(),
                0,
                attribute,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(errmap::last_win32_err(
                "UpdateProcThreadAttribute",
                OsStr::new(""),
            ));
        }
        Ok(())
    }
}

impl Drop for AttrList {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`, deleted exactly once here.
        unsafe { w::DeleteProcThreadAttributeList(self.raw()) };
    }
}

type ConsoleControlFn = unsafe extern "system" fn(u32, *mut c_void, u32) -> i32;

/// `user32!ConsoleControl`, loaded from System32 once and cached. `None`
/// when user32 or the export is missing. The module is never freed.
fn console_control() -> Option<ConsoleControlFn> {
    static CACHE: OnceLock<Option<ConsoleControlFn>> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let name = to_wide_nul(OsStr::new("user32.dll"));
        // SAFETY: `name` is NUL-terminated; a null file handle and the
        // System32-only search flag are documented-valid.
        let module = unsafe {
            w::LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                w::LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return None;
        }
        // SAFETY: `module` is a loaded module; the name is NUL-terminated.
        let proc_addr: w::FARPROC =
            unsafe { w::GetProcAddress(module, b"ConsoleControl\0".as_ptr()) };
        // SAFETY: `ConsoleControl`'s signature is
        // `NTSTATUS (CONSOLECONTROL, PVOID, DWORD)` (microsoft/terminal
        // `ConsoleControl.cpp`); both are `extern "system"` fn pointers.
        proc_addr.map(|f| unsafe { std::mem::transmute::<_, ConsoleControlFn>(f) })
    })
}

/// Whether `ConsoleControl` resolves.
pub fn has_console_control() -> bool {
    console_control().is_some()
}

/// `ConsoleControl(ConsoleEndTask, …)`: ask Windows to deliver a Ctrl
/// event to `pid`. The NTSTATUS says nothing about delivery (conhost's
/// `input.cpp` notes it fails for dead processes and succeeds for
/// refusals), so only a missing export is an error.
pub fn end_task(pid: u32, event_code: u32, flags: u32) -> Result<()> {
    let Some(f) = console_control() else {
        return Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "user32!ConsoleControl is not available",
        ));
    };
    let mut task = ConsoleEndTask {
        process_id: pid as usize,
        hwnd: 0,
        console_event_code: event_code,
        console_flags: flags,
    };
    // SAFETY: `task` is a valid CONSOLEENDTASK for the call's duration and
    // the length is its size.
    unsafe {
        f(
            CONSOLE_END_TASK,
            (&mut task as *mut ConsoleEndTask).cast(),
            std::mem::size_of::<ConsoleEndTask>() as u32,
        );
    }
    Ok(())
}
