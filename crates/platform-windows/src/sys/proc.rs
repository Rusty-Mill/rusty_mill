//! Process spawning over `CreateProcessW` (RFC v2 §5.4; extraction map
//! step 2, first slice). The command line is built exclusively by
//! [`crate::winargv`] — no other quoting path exists in this crate.
//!
//! Stdio model: when every slot is `Inherit`, the child inherits the
//! parent's std handles the default way (no handle list). When any slot
//! is `Null`, `STARTF_USESTDHANDLES` is used and each slot gets an
//! explicit handle — an inheritable `NUL` device handle for `Null`, or an
//! inheritable duplicate of the parent's own std handle for `Inherit`
//! (duplication because the parent's handle may itself be
//! non-inheritable; the duplicates are closed after the spawn snapshot,
//! mirroring rush's winstdio lesson that `CreateProcessW` snapshots at
//! spawn).

#![allow(unsafe_code)]

use std::ffi::OsStr;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::process::{EnvSpec, ExitStatus, Stdio};

use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::sys::handle::OwnedWinHandle;
use crate::util::wide::to_wide_nul;

/// Build the `CREATE_UNICODE_ENVIRONMENT` block for an explicit env, or
/// `None` to inherit. Each entry is `NAME=value\0`, with a final extra
/// NUL (double-NUL for the empty block).
/// `pub(crate)`: shared with `sys::pty` (rustils#83), which builds its own
/// `CreateProcessW` call for the ConPTY-attached spawn path rather than
/// duplicating this exact env-block boilerplate.
pub(crate) fn env_block(env: &EnvSpec) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let map = match env {
        EnvSpec::Inherit => return None,
        EnvSpec::Explicit(map) => map,
    };
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in map {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Some(block)
}

/// An inheritable handle for one std slot, plus the RAII that closes any
/// handle this module itself opened or duplicated.
enum SlotHandle {
    Owned(OwnedWinHandle),
}

impl SlotHandle {
    fn raw(&self) -> w::HANDLE {
        match self {
            SlotHandle::Owned(h) => h.as_raw(),
        }
    }
}

fn inheritable_nul(read: bool) -> Result<SlotHandle> {
    let access = if read {
        w::FILE_GENERIC_READ
    } else {
        w::FILE_GENERIC_WRITE
    };
    let sa = w::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<w::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let name = to_wide_nul(OsStr::new("NUL"));
    // SAFETY: `name` is a valid NUL-terminated UTF-16 buffer and `sa` a
    // fully initialized SECURITY_ATTRIBUTES, both outliving the call.
    let h = unsafe {
        w::CreateFileW(
            name.as_ptr(),
            access,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE,
            &sa,
            w::OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    OwnedWinHandle::from_raw(h)
        .map(SlotHandle::Owned)
        .ok_or_else(|| errmap::last_win32_err("CreateFileW", OsStr::new("NUL")))
}

/// An anonymous pipe whose *child* end is inheritable and whose *parent*
/// end is explicitly not — the parent end must never leak into any child,
/// or a reader waits for an EOF that cannot come (extraction map D5).
/// Returns (child end, parent end); `stdin_slot` decides which side is
/// which.
///
/// Track W (D-15): `rusty_win32::handle::create_pipe` + `set_inheritable`.
/// The donor's `create_pipe` creates **both** ends non-inheritable by
/// design (its own doc: "pass whichever end a child needs through
/// `set_inheritable` first") — the opposite default from the
/// windows-sys arm's `bInheritHandle = 1` `SECURITY_ATTRIBUTES`, which
/// makes both ends inheritable and then clears the parent end. Same
/// final state either way (child inheritable, parent not), reached from
/// opposite starting points: mark up versus mark down. No race between
/// creation and the inheritability flip in either arm — every handle
/// this function returns is fully configured before `spawn` ever calls
/// `CreateProcessW`, the only point inheritance is actually consulted.
#[cfg(feature = "track-w")]
fn make_pipe(stdin_slot: bool) -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    let (read, write) =
        rusty_win32::handle::create_pipe().map_err(|e| errmap::trackw_err("CreatePipe", e))?;
    let read = OwnedWinHandle::from_raw(read)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let write = OwnedWinHandle::from_raw(write)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let (child, parent) = if stdin_slot {
        (read, write)
    } else {
        (write, read)
    };
    // SAFETY: `child` is the valid handle `create_pipe` just returned,
    // still open and owned by this function's `child` local.
    unsafe { rusty_win32::handle::set_inheritable(child.as_raw(), true) }
        .map_err(|e| errmap::trackw_err("SetHandleInformation", e))?;
    Ok((child, parent))
}

#[cfg(not(feature = "track-w"))]
fn make_pipe(stdin_slot: bool) -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    let sa = w::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<w::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read: w::HANDLE = std::ptr::null_mut();
    let mut write: w::HANDLE = std::ptr::null_mut();
    // SAFETY: `read`/`write` are valid out-pointers and `sa` a fully
    // initialized SECURITY_ATTRIBUTES, all outliving the call.
    let ok = unsafe { w::CreatePipe(&mut read, &mut write, &sa, 0) };
    if ok == 0 {
        return Err(errmap::last_win32_err("CreatePipe", OsStr::new("")));
    }
    let read = OwnedWinHandle::from_raw(read)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let write = OwnedWinHandle::from_raw(write)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreatePipe"))?;
    let (child, parent) = if stdin_slot {
        (read, write)
    } else {
        (write, read)
    };
    // SAFETY: `parent` is a valid open handle; clearing its inherit flag
    // has no other precondition.
    let ok = unsafe { w::SetHandleInformation(parent.as_raw(), w::HANDLE_FLAG_INHERIT, 0) };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "SetHandleInformation",
            OsStr::new(""),
        ));
    }
    Ok((child, parent))
}

/// An inheritable duplicate of a [`Stdio::File`]'s underlying handle
/// (rustils#51, D5) — downcasts to this backend's own `WindowsFile` via
/// [`platform::fs::File::as_any`] to reach the handle
/// [`crate::sys::handle::duplicate`] wraps as inheritable; a `File` from
/// a foreign backend fails `Unsupported` rather than guessing how to
/// extract a handle from it.
fn inheritable_dup_of_file(file: &dyn platform::fs::File) -> Result<OwnedWinHandle> {
    let windows_file = file
        .as_any()
        .downcast_ref::<crate::fs::WindowsFile>()
        .ok_or_else(|| {
            PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::None,
                "CreateProcessW: Stdio::File from a foreign backend",
            )
        })?;
    crate::sys::handle::duplicate(&windows_file.handle, true)
}

/// `GetStdHandle` folded to the exact `NULL`/`INVALID_HANDLE_VALUE`
/// sentinels the raw call itself produces, whichever arm answers.
///
/// Track W (D-15): `rusty_win32::handle::get_std_handle`. Same shape
/// `sys::console::std_handle` already uses for the identical fold, kept
/// as a separate local copy rather than shared: that helper is keyed by
/// `TermStream`, this one by the raw `u32` slot constant `spawn` already
/// has in hand, and neither module has a reason to depend on the other's
/// private primitive for one three-line function.
#[cfg(feature = "track-w")]
fn std_handle_raw(slot: u32) -> w::HANDLE {
    match rusty_win32::handle::get_std_handle(slot) {
        Ok(Some(h)) => h,
        Ok(None) => std::ptr::null_mut(),
        Err(_) => w::INVALID_HANDLE_VALUE,
    }
}

#[cfg(not(feature = "track-w"))]
fn std_handle_raw(slot: u32) -> w::HANDLE {
    // SAFETY: `GetStdHandle` takes a documented slot constant and has no
    // other preconditions.
    unsafe { w::GetStdHandle(slot) }
}

/// Track W (D-15): `rusty_win32::handle::duplicate`, called on the raw
/// `current` value directly rather than through `sys::handle::duplicate`
/// (which takes `&OwnedWinHandle`) — this process doesn't own the
/// original std-slot handle, only the duplicate this mints, exactly
/// mirroring the windows-sys arm below, which never closes `current`
/// either.
#[cfg(feature = "track-w")]
fn inheritable_dup_of_std(slot: u32) -> Result<Option<SlotHandle>> {
    let current = std_handle_raw(slot);
    if current.is_null() || current == w::INVALID_HANDLE_VALUE {
        // No handle in this slot (e.g. a detached process): leave it
        // empty rather than fail the whole spawn.
        return Ok(None);
    }
    // SAFETY: `current` is a live, currently-open handle in this
    // process's own std slot — `duplicate`'s whole safety contract.
    let dup = unsafe { rusty_win32::handle::duplicate(current, true) }
        .map_err(|e| errmap::trackw_err("DuplicateHandle", e))?;
    match OwnedWinHandle::from_raw(dup) {
        Some(h) => Ok(Some(SlotHandle::Owned(h))),
        None => Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "DuplicateHandle",
        )),
    }
}

#[cfg(not(feature = "track-w"))]
fn inheritable_dup_of_std(slot: u32) -> Result<Option<SlotHandle>> {
    let current = std_handle_raw(slot);
    if current.is_null() || current == w::INVALID_HANDLE_VALUE {
        // No handle in this slot (e.g. a detached process): leave it
        // empty rather than fail the whole spawn.
        return Ok(None);
    }
    let mut dup: w::HANDLE = std::ptr::null_mut();
    // SAFETY: source process/handle are this process's own valid handles;
    // `dup` is a valid out-pointer; DUPLICATE_SAME_ACCESS with
    // bInheritHandle=1 is the documented way to mint an inheritable
    // duplicate.
    let ok = unsafe {
        w::DuplicateHandle(
            w::GetCurrentProcess(),
            current,
            w::GetCurrentProcess(),
            &mut dup,
            0,
            1,
            w::DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("DuplicateHandle", OsStr::new("")));
    }
    Ok(OwnedWinHandle::from_raw(dup).map(SlotHandle::Owned))
}

/// Create a kill-on-close Job Object (extraction map D2: the group
/// mechanism — closing the last handle terminates every member, so a
/// dropped-unwaited grouped child cannot leak its tree). `pub(crate)`:
/// shared with `sys::pty` (rustils#83), which always groups a pty-hosted
/// child the same way a `GroupSpec::NewGroup` spawn does here — see that
/// module's own doc comment for why (a pty-hosted child is unconditionally
/// its own session, mirroring the Linux backend's identical reasoning).
///
/// Track W (D-15): `rusty_win32::job::create` + `job::set_kill_on_close`.
/// The donor splits into two calls what the windows-sys arm does inline,
/// but the limit struct it builds is byte-for-byte the one below —
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` in `LimitFlags`, everything else
/// zeroed — so the resulting job object is indistinguishable.
#[cfg(feature = "track-w")]
pub(crate) fn create_kill_on_close_job() -> Result<OwnedWinHandle> {
    let raw = rusty_win32::job::create().map_err(|e| errmap::trackw_err("CreateJobObjectW", e))?;
    let job = OwnedWinHandle::from_raw(raw)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreateJobObjectW"))?;
    // SAFETY: `job` is the valid handle just created and still open (it is
    // dropped no earlier than this function's return) — `set_kill_on_close`'s
    // whole safety contract.
    unsafe { rusty_win32::job::set_kill_on_close(job.as_raw()) }
        .map_err(|e| errmap::trackw_err("SetInformationJobObject", e))?;
    Ok(job)
}

#[cfg(not(feature = "track-w"))]
pub(crate) fn create_kill_on_close_job() -> Result<OwnedWinHandle> {
    // SAFETY: null security attributes and name are documented-valid.
    let job = unsafe { w::CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    let job = OwnedWinHandle::from_raw(job)
        .ok_or_else(|| errmap::last_win32_err("CreateJobObjectW", OsStr::new("")))?;
    // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION is plain-old-data for
    // which all-zeroes is valid; only the limit flag is set before use.
    let mut info: w::JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = w::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `job` is the valid handle just created; `info` is a fully
    // valid struct of exactly the passed size, outliving the call.
    let ok = unsafe {
        w::SetInformationJobObject(
            job.as_raw(),
            w::JobObjectExtendedLimitInformation,
            (&info as *const w::JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<w::JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "SetInformationJobObject",
            OsStr::new(""),
        ));
    }
    Ok(job)
}

/// Open an already-running process by pid and place it into a fresh
/// kill-on-close Job Object (`Spawner::adopt`, rustils#47) — the
/// after-the-fact counterpart to `spawn`'s suspended-spawn → assign →
/// resume sequence, for a pid this backend did not itself create (e.g.
/// `portable-pty::Child::process_id()`). No suspend-before-assign
/// ordering to preserve here (unlike `spawn`'s pre-first-instruction
/// guarantee): the target is already running arbitrary code by the time
/// any caller could have observed its pid, so "before its first
/// instruction" was never a reachable guarantee for an adopted process
/// the way it is for one this crate spawned suspended. Returns (process
/// handle, job handle).
///
/// Track W (D-15): `rusty_win32::process::open_by_pid` + `job::assign`.
/// The access mask stays `w::PROCESS_SET_QUOTA | w::PROCESS_TERMINATE`
/// from windows-sys rather than the donor's own constants — it exports
/// `PROCESS_TERMINATE` but has no `PROCESS_SET_QUOTA`, and inventing a
/// second literal for a value already admitted in `ffi::win32_surface`
/// would be exactly the hand-transcription D-1 exists to prevent.
#[cfg(feature = "track-w")]
pub fn adopt(pid: u32) -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    let raw = rusty_win32::process::open_by_pid(pid, w::PROCESS_SET_QUOTA | w::PROCESS_TERMINATE)
        .map_err(|e| errmap::trackw_err("OpenProcess", e))?;
    let process = OwnedWinHandle::from_raw(raw)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "OpenProcess"))?;
    let job = create_kill_on_close_job()?;
    // SAFETY: `job` and `process` are both valid, currently-open handles,
    // owned here and dropped no earlier than this function's return.
    unsafe { rusty_win32::job::assign(job.as_raw(), process.as_raw()) }
        .map_err(|e| errmap::trackw_err("AssignProcessToJobObject", e))?;
    Ok((process, job))
}

#[cfg(not(feature = "track-w"))]
pub fn adopt(pid: u32) -> Result<(OwnedWinHandle, OwnedWinHandle)> {
    // SAFETY: `pid` is caller-supplied and may not name a live process at
    // all — `OpenProcess` reports that as a `NULL` return, checked by
    // `OwnedWinHandle::from_raw` below, not as UB.
    let handle = unsafe { w::OpenProcess(w::PROCESS_SET_QUOTA | w::PROCESS_TERMINATE, 0, pid) };
    let process = OwnedWinHandle::from_raw(handle)
        .ok_or_else(|| errmap::last_win32_err("OpenProcess", OsStr::new("")))?;
    let job = create_kill_on_close_job()?;
    // SAFETY: `job`/`process` are both valid, currently-open handles for
    // the life of this call.
    let ok = unsafe { w::AssignProcessToJobObject(job.as_raw(), process.as_raw()) };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "AssignProcessToJobObject",
            OsStr::new(""),
        ));
    }
    Ok((process, job))
}

/// Parent-side pipe ends for piped stdio slots: `[stdin write, stdout
/// read, stderr read]`.
pub type ParentPipes = [Option<OwnedWinHandle>; 3];

/// Spawn `command_line` (winargv-built, not yet NUL-terminated) with
/// working directory `cwd`. With `new_group`, the child starts
/// `CREATE_SUSPENDED`, joins a fresh kill-on-close Job Object, and only
/// then resumes — job membership is guaranteed before the child (or
/// anything it later spawns) executes a single instruction (extraction
/// map D2's proven sequence). Returns (process handle, job handle if
/// grouped, pid, parent pipe ends).
///
/// **Stays on windows-sys in both Track W configurations (D-15).** The
/// surrounding job/resume/terminate steps migrated; this `CreateProcessW`
/// call cannot, for three independent reasons, none of them fixable by
/// bumping the pinned rev:
///
/// 1. **No per-spawn std-handle override.** `rusty_win32::process::
///    spawn_suspended` passes a bare zeroed `STARTUPINFOW` and says so in
///    its own docs: "redirect by swapping the parent's std-handle slots
///    before spawning". That is rush's `winstdio` model, and the
///    extraction map (D5, step 4) already records this repo's deliberate
///    decision *against* it — `STARTF_USESTDHANDLES` per spawn, so
///    `Stdio::{Null, Pipe, File}` never mutates process-global state.
///    Migrating here would not be adopting a binding, it would be
///    reversing a recorded architectural decision and deleting the
///    `Stdio` mechanism with it.
/// 2. **No working directory.** `spawn_suspended` passes `NULL` for
///    `lpCurrentDirectory`; `Command::current_dir` needs it.
/// 3. **`&str`, not `&[u16]`.** Its command line is UTF-8, so a
///    winargv-built `&[u16]` would have to round-trip through `String`.
///    winargv is this crate's security boundary (§5.4); routing it
///    through a lossy conversion for unpaired surrogates is not a
///    trade this slice gets to make.
///
/// The honest summary: the donor's spawn is shaped for a shell that owns
/// its own std slots, and this backend deliberately isn't one. See
/// `docs/convergence-roadmap.md` §1d.
pub fn spawn(
    command_line: &[u16],
    cwd: &OsStr,
    env: &EnvSpec,
    stdio: [&Stdio; 3],
    new_group: bool,
) -> Result<(OwnedWinHandle, Option<OwnedWinHandle>, u32, ParentPipes)> {
    let mut line: Vec<u16> = command_line.to_vec();
    line.push(0);
    let cwd_w = to_wide_nul(cwd);
    let block = env_block(env);

    let use_handles = stdio
        .iter()
        .any(|s| matches!(s, Stdio::Null | Stdio::Pipe | Stdio::File(_)));
    let mut parent_pipes: ParentPipes = [None, None, None];
    let mut slot_handles: [Option<SlotHandle>; 3] = [None, None, None];
    // SAFETY: STARTUPINFOW is plain-old-data for which all-zeroes is a
    // valid starting value; `cb` is set before use.
    let mut si: w::STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<w::STARTUPINFOW>() as u32;
    if use_handles {
        let slots = [
            w::STD_INPUT_HANDLE,
            w::STD_OUTPUT_HANDLE,
            w::STD_ERROR_HANDLE,
        ];
        for (i, spec) in stdio.iter().enumerate() {
            slot_handles[i] = match spec {
                Stdio::Null => Some(inheritable_nul(i == 0)?),
                Stdio::Inherit => inheritable_dup_of_std(slots[i])?,
                Stdio::Pipe => {
                    let (child, parent) = make_pipe(i == 0)?;
                    parent_pipes[i] = Some(parent);
                    Some(SlotHandle::Owned(child))
                }
                Stdio::File(file) => {
                    Some(SlotHandle::Owned(inheritable_dup_of_file(file.as_ref())?))
                }
            };
        }
        si.dwFlags |= w::STARTF_USESTDHANDLES;
        si.hStdInput = slot_handles[0]
            .as_ref()
            .map_or(std::ptr::null_mut(), SlotHandle::raw);
        si.hStdOutput = slot_handles[1]
            .as_ref()
            .map_or(std::ptr::null_mut(), SlotHandle::raw);
        si.hStdError = slot_handles[2]
            .as_ref()
            .map_or(std::ptr::null_mut(), SlotHandle::raw);
    }

    let mut flags = if block.is_some() {
        w::CREATE_UNICODE_ENVIRONMENT
    } else {
        0
    };
    if new_group {
        flags |= w::CREATE_SUSPENDED;
    }
    let env_ptr = block
        .as_ref()
        .map_or(std::ptr::null(), |b| b.as_ptr().cast::<core::ffi::c_void>());

    // SAFETY: PROCESS_INFORMATION is plain-old-data; CreateProcessW
    // overwrites it on success before we read it.
    let mut pi: w::PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `line` is an owned, mutable, NUL-terminated UTF-16 buffer
    // (CreateProcessW may write into it); `cwd_w` is NUL-terminated and
    // outlives the call; `env_ptr` is null (inherit) or a well-formed
    // double-NUL block per `env_block`; `si`/`pi` are valid with `cb`
    // set; any slot handles are open, inheritable, and outlive the call
    // (`slot_handles` lives to end of scope); null security attributes
    // and application name are documented-valid.
    let ok = unsafe {
        w::CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            i32::from(use_handles),
            flags,
            env_ptr,
            cwd_w.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("CreateProcessW", cwd));
    }
    // Slot handles (NUL opens and inheritable duplicates) close as
    // `slot_handles` drops here — CreateProcessW has snapshotted them.
    let process = OwnedWinHandle::from_raw(pi.hProcess)
        .ok_or_else(|| PlatformError::new(ErrorKind::Other, OsCode::None, "CreateProcessW"))?;
    let thread = OwnedWinHandle::from_raw(pi.hThread);

    if !new_group {
        // The main thread runs immediately; its handle is not retained.
        drop(thread);
        return Ok((process, None, pi.dwProcessId, parent_pipes));
    }

    // Suspended → assign → resume. On any failure the suspended child
    // must not be leaked: terminate it directly (it never ran an
    // instruction, so this is clean).
    //
    // The assign and resume steps are Track W-migrated (`job::assign`,
    // `process::resume`); the `CreateProcessW` call above is not, and
    // cannot be — see this function's own doc comment.
    let sequence = (|| -> Result<OwnedWinHandle> {
        let job = create_kill_on_close_job()?;
        assign_to_job(&job, &process)?;
        let thread = thread.as_ref().ok_or_else(|| {
            PlatformError::new(ErrorKind::Other, OsCode::None, "CreateProcessW thread")
        })?;
        resume_thread(thread)?;
        Ok(job)
    })();
    match sequence {
        Ok(job) => Ok((process, Some(job), pi.dwProcessId, parent_pipes)),
        Err(e) => {
            // The suspended (or at worst just-assigned) child this call
            // created is terminated exactly once here before the handles
            // drop. The result is deliberately discarded: this is the
            // cleanup path of an already-failing spawn, and `e` — the
            // reason the spawn failed — is the error worth reporting, not
            // whatever a best-effort kill of a process that never ran an
            // instruction has to say.
            let _ = terminate_process(&process);
            Err(e)
        }
    }
}

/// `AssignProcessToJobObject` — Track W (D-15): `rusty_win32::job::assign`.
/// Shared by `spawn`'s suspended→assign→resume sequence and [`adopt`]'s
/// after-the-fact attach, so the migration lands once rather than twice.
#[cfg(feature = "track-w")]
fn assign_to_job(job: &OwnedWinHandle, process: &OwnedWinHandle) -> Result<()> {
    // SAFETY: both are valid, currently-open handles for the life of the
    // `&` borrows — `assign`'s whole safety contract.
    unsafe { rusty_win32::job::assign(job.as_raw(), process.as_raw()) }
        .map_err(|e| errmap::trackw_err("AssignProcessToJobObject", e))
}

#[cfg(not(feature = "track-w"))]
fn assign_to_job(job: &OwnedWinHandle, process: &OwnedWinHandle) -> Result<()> {
    // SAFETY: both handles are valid and open for the life of the `&`
    // borrows; when called from `spawn` the process is still suspended, so
    // membership precedes its first instruction.
    let ok = unsafe { w::AssignProcessToJobObject(job.as_raw(), process.as_raw()) };
    if ok == 0 {
        return Err(errmap::last_win32_err(
            "AssignProcessToJobObject",
            OsStr::new(""),
        ));
    }
    Ok(())
}

/// `ResumeThread` — Track W (D-15): `rusty_win32::process::resume`. The
/// donor performs the identical `prev == u32::MAX` failure check and
/// discards the previous suspend count, which this caller never wanted
/// either.
#[cfg(feature = "track-w")]
fn resume_thread(thread: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `thread` is the valid, suspended main-thread handle, open
    // for the life of the `&` borrow.
    unsafe { rusty_win32::process::resume(thread.as_raw()) }
        .map_err(|e| errmap::trackw_err("ResumeThread", e))
}

#[cfg(not(feature = "track-w"))]
fn resume_thread(thread: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `thread` is the valid, suspended main-thread handle.
    let prev = unsafe { w::ResumeThread(thread.as_raw()) };
    if prev == u32::MAX {
        return Err(errmap::last_win32_err("ResumeThread", OsStr::new("")));
    }
    Ok(())
}

/// `WaitForMultipleObjects`'s own hard cap on one call's handle count.
const MAXIMUM_WAIT_OBJECTS: usize = 64;

/// Multiplexed wait over process handles (RFC v2 §5.6 reactor internals,
/// R3): `Some(position)` of a signaled handle, `None` on timeout. The
/// 64-handle `WaitForMultipleObjects` cap is absorbed here: up to 64
/// handles use one true blocking wait; beyond that, 64-sized chunks are
/// swept with zero-timeout waits on a 10ms tick until the deadline —
/// the documented Win32 limit, not one this crate invents.
/// Raw handles because the caller (the trait impl layer) collects them
/// through `&mut` children it continues to hold across this call — the
/// borrow that guarantees every handle stays open for the duration.
pub fn wait_many(raw: &[w::HANDLE], timeout: Option<std::time::Duration>) -> Result<Option<usize>> {
    if raw.len() <= MAXIMUM_WAIT_OBJECTS {
        let ms = timeout.map_or(w::INFINITE, |t| {
            t.as_millis().min(u128::from(w::INFINITE - 1)) as u32
        });
        return wait_chunk(raw, ms);
    }

    let deadline = timeout.map(|t| std::time::Instant::now() + t);
    loop {
        for (chunk_index, chunk) in raw.chunks(MAXIMUM_WAIT_OBJECTS).enumerate() {
            if let Some(hit) = wait_chunk(chunk, 0)? {
                return Ok(Some(chunk_index * MAXIMUM_WAIT_OBJECTS + hit));
            }
        }
        if let Some(d) = deadline {
            if std::time::Instant::now() >= d {
                return Ok(None);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// One `WaitForMultipleObjects` call over at most [`MAXIMUM_WAIT_OBJECTS`]
/// handles: `Some(index)` of the signaled handle, `None` on timeout. The
/// chunking above is this crate's own and stays shared; only the single
/// call is Track W-migrated.
///
/// Track W (D-15): `rusty_win32::process::wait_any`. That wrapper also
/// fetches the signaled process's exit code via `GetExitCodeProcess`,
/// which this caller discards — `wait_many` reports *which* child is ready
/// and lets the caller do its own `wait`, so the extra query is a wasted
/// call, not a behavior change. The one divergence worth naming: a failing
/// `GetExitCodeProcess` turns a successful wait into an `Err` here, where
/// the windows-sys arm would have returned `Some(index)` and surfaced that
/// failure on the caller's subsequent `wait`. Both report the same
/// `ErrorKind`/`OsCode` for the same OS condition, just one call earlier.
#[cfg(feature = "track-w")]
fn wait_chunk(chunk: &[w::HANDLE], ms: u32) -> Result<Option<usize>> {
    // SAFETY: `chunk` is a valid array of at most 64 open process handles,
    // borrowed for the duration of this call — `wait_any`'s contract.
    match unsafe { rusty_win32::process::wait_any(chunk, Some(ms)) } {
        Ok(Some((idx, _exit_code))) => Ok(Some(idx)),
        Ok(None) => Ok(None),
        Err(e) => Err(errmap::trackw_err("WaitForMultipleObjects", e)),
    }
}

#[cfg(not(feature = "track-w"))]
fn wait_chunk(chunk: &[w::HANDLE], ms: u32) -> Result<Option<usize>> {
    // SAFETY: `chunk` is a valid array of at most 64 open process
    // handles (borrowed for the duration of this call).
    let r = unsafe { w::WaitForMultipleObjects(chunk.len() as u32, chunk.as_ptr(), 0, ms) };
    if r == w::WAIT_TIMEOUT {
        return Ok(None);
    }
    let idx = r.wrapping_sub(w::WAIT_OBJECT_0) as usize;
    if idx < chunk.len() {
        return Ok(Some(idx));
    }
    Err(errmap::last_win32_err(
        "WaitForMultipleObjects",
        OsStr::new(""),
    ))
}

/// Terminate every process in `job` (kill-tree). Exit code 1 — Windows
/// has no signal identity to encode (divergence 001).
/// Track W (D-15): `rusty_win32::job::terminate`, same exit code.
#[cfg(feature = "track-w")]
pub fn terminate_job(job: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `job` is a valid open job handle for the life of `&self`.
    unsafe { rusty_win32::job::terminate(job.as_raw(), 1) }
        .map_err(|e| errmap::trackw_err("TerminateJobObject", e))
}

#[cfg(not(feature = "track-w"))]
pub fn terminate_job(job: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `job` is a valid open job handle for the life of `&self`.
    let ok = unsafe { w::TerminateJobObject(job.as_raw(), 1) };
    if ok == 0 {
        return Err(errmap::last_win32_err("TerminateJobObject", OsStr::new("")));
    }
    Ok(())
}

/// Terminate the single process `process` with exit code 1.
/// Track W (D-15): `rusty_win32::process::terminate`, same exit code.
#[cfg(feature = "track-w")]
pub fn terminate_process(process: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`.
    unsafe { rusty_win32::process::terminate(process.as_raw(), 1) }
        .map_err(|e| errmap::trackw_err("TerminateProcess", e))
}

#[cfg(not(feature = "track-w"))]
pub fn terminate_process(process: &OwnedWinHandle) -> Result<()> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`.
    let ok = unsafe { w::TerminateProcess(process.as_raw(), 1) };
    if ok == 0 {
        return Err(errmap::last_win32_err("TerminateProcess", OsStr::new("")));
    }
    Ok(())
}

/// Non-blocking poll: zero-timeout `WaitForSingleObject`. `Some(code)` if
/// the process has exited, `None` if still running.
///
/// Track W (D-15): `rusty_win32::process::wait` with a `Some(0)` timeout —
/// the donor's own documented `waitpid(WNOHANG)` analog. Its `Ok(None)` is
/// this function's `Ok(None)` (still running). Its `Err` is *also* mapped
/// to `Ok(None)`, deliberately: that is precisely the
/// "any-non-exit-result means still running" contract the windows-sys arm
/// below states in its own comment, and a poll that started reporting
/// errors under one feature flag and not the other would be a behavior
/// difference, not a fidelity improvement. The genuine failure still
/// surfaces on the eventual blocking [`wait`], identically in both arms.
#[cfg(feature = "track-w")]
pub fn try_wait(process: &OwnedWinHandle) -> Result<Option<ExitStatus>> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`; a zero timeout never blocks.
    match unsafe { rusty_win32::process::wait(process.as_raw(), Some(0)) } {
        Ok(Some(code)) => Ok(Some(ExitStatus::Code(code as i32))),
        Ok(None) | Err(_) => Ok(None),
    }
}

#[cfg(not(feature = "track-w"))]
pub fn try_wait(process: &OwnedWinHandle) -> Result<Option<ExitStatus>> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`; a zero timeout never blocks.
    let r = unsafe { w::WaitForSingleObject(process.as_raw(), 0) };
    if r != w::WAIT_OBJECT_0 {
        // WAIT_TIMEOUT — still running. Any genuine failure will surface
        // on the eventual blocking wait; a poll reports liveness only.
        return Ok(None);
    }
    let mut code: u32 = 0;
    // SAFETY: same valid handle; `code` is a valid out-pointer.
    let ok = unsafe { w::GetExitCodeProcess(process.as_raw(), &mut code) };
    if ok == 0 {
        return Err(errmap::last_win32_err("GetExitCodeProcess", OsStr::new("")));
    }
    Ok(Some(ExitStatus::Code(code as i32)))
}

/// Block until `process` exits; decode the code. `Signaled` is never
/// produced on Windows (behavior spec `docs/behavior/process.md`).
///
/// Track W (D-15): `rusty_win32::process::wait` with no timeout. The one
/// divergence in this whole slice, and it is a diagnostic label only: the
/// donor folds `WaitForSingleObject` and `GetExitCodeProcess` into one
/// call, so a failure of the second is reported here under the first's
/// `op` string. `ErrorKind` and `OsCode` — the two fields anything
/// actually branches on — are identical either way, since both arms
/// classify the same `GetLastError` code through the same table. A
/// `WAIT_TIMEOUT` return is impossible with an infinite timeout, so the
/// donor's `Ok(None)` arm is unreachable; it is mapped to the same error
/// the windows-sys arm produces for a non-`WAIT_OBJECT_0` result rather
/// than left to an `unreachable!()` that a future donor change could
/// turn into a panic.
#[cfg(feature = "track-w")]
pub fn wait(process: &OwnedWinHandle) -> Result<ExitStatus> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`.
    match unsafe { rusty_win32::process::wait(process.as_raw(), None) } {
        Ok(Some(code)) => Ok(ExitStatus::Code(code as i32)),
        Ok(None) => Err(PlatformError::new(
            ErrorKind::Other,
            OsCode::None,
            "WaitForSingleObject",
        )),
        Err(e) => Err(errmap::trackw_err("WaitForSingleObject", e)),
    }
}

#[cfg(not(feature = "track-w"))]
pub fn wait(process: &OwnedWinHandle) -> Result<ExitStatus> {
    // SAFETY: `process` is a valid open process handle for the life of
    // `&self`.
    let r = unsafe { w::WaitForSingleObject(process.as_raw(), w::INFINITE) };
    if r != w::WAIT_OBJECT_0 {
        return Err(errmap::last_win32_err(
            "WaitForSingleObject",
            OsStr::new(""),
        ));
    }
    let mut code: u32 = 0;
    // SAFETY: same valid handle; `code` is a valid out-pointer.
    let ok = unsafe { w::GetExitCodeProcess(process.as_raw(), &mut code) };
    if ok == 0 {
        return Err(errmap::last_win32_err("GetExitCodeProcess", OsStr::new("")));
    }
    Ok(ExitStatus::Code(code as i32))
}
