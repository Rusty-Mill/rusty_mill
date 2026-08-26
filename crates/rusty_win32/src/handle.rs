//! `DuplicateHandle`/`CreatePipe`/`SetHandleInformation`/`CloseHandle` — the
//! Windows counterpart of Unix `dup`/`pipe2`/`close`, and the primitive rush
//! needs to close its fd-3-and-up gap (see rush's
//! `docs/WINDOWS_BACKEND_ANALYSIS.md` §4.2). Windows has no kernel-level
//! small-integer fd table the way Unix does — only the three std-handle
//! slots [`get_std_handle`]/[`set_std_handle`] read and swap, which this
//! crate's [`crate::console`] sibling doesn't otherwise touch. This module
//! provides the raw handle primitives; a rush-owned integer-to-`HANDLE` map
//! to give fd 3+ and `{name}>` varfd redirects any meaning is a follow-up in
//! rush itself, not this crate (see the analysis doc's §4.2 for why that
//! split is deliberate).
//!
//! A Windows `HANDLE` is non-inheritable by default — the inverse of Unix's
//! `CLOEXEC`-by-default-absent convention, where a descriptor is inherited
//! unless explicitly marked `FD_CLOEXEC`. [`set_inheritable`] is the
//! `SetHandleInformation`/`HANDLE_FLAG_INHERIT` call that flips a specific
//! handle the other way, for the one end of a pipe (or duplicated handle) a
//! spawned child should actually see.

use crate::error::Win32Error;

/// A raw Win32 `HANDLE` (same representation `std::os::windows::io` uses).
pub type RawHandle = *mut core::ffi::c_void;

/// `SetHandleInformation`'s `dwMask`/`dwFlags` bit for handle inheritance.
const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
/// `DuplicateHandle`'s `dwOptions` bit: ignore `dwDesiredAccess` and
/// duplicate with the source handle's own access rights.
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> RawHandle;
    fn CreatePipe(
        read_pipe: *mut RawHandle,
        write_pipe: *mut RawHandle,
        pipe_attributes: *const core::ffi::c_void,
        size: u32,
    ) -> i32;
    fn DuplicateHandle(
        source_process: RawHandle,
        source_handle: RawHandle,
        target_process: RawHandle,
        target_handle: *mut RawHandle,
        desired_access: u32,
        inherit_handle: i32,
        options: u32,
    ) -> i32;
    fn SetHandleInformation(object: RawHandle, mask: u32, flags: u32) -> i32;
    fn GetHandleInformation(object: RawHandle, flags: *mut u32) -> i32;
    fn CloseHandle(object: RawHandle) -> i32;
    fn PeekNamedPipe(
        named_pipe: RawHandle,
        buffer: *mut u8,
        buffer_size: u32,
        bytes_read: *mut u32,
        total_bytes_avail: *mut u32,
        bytes_left_this_message: *mut u32,
    ) -> i32;
    fn GetStdHandle(std_handle: u32) -> RawHandle;
    fn SetStdHandle(std_handle: u32, handle: RawHandle) -> i32;
    fn CreateMutexW(
        mutex_attributes: *const core::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> RawHandle;
    fn ReleaseMutex(mutex: RawHandle) -> i32;
    fn CreateSemaphoreW(
        semaphore_attributes: *const core::ffi::c_void,
        initial_count: i32,
        maximum_count: i32,
        name: *const u16,
    ) -> RawHandle;
    fn ReleaseSemaphore(semaphore: RawHandle, release_count: i32, previous_count: *mut i32) -> i32;
    fn WaitForSingleObjectEx(handle: RawHandle, milliseconds: u32, alertable: i32) -> u32;
    fn WaitForMultipleObjectsEx(
        count: u32,
        handles: *const RawHandle,
        wait_all: i32,
        milliseconds: u32,
        alertable: i32,
    ) -> u32;
    fn SignalObjectAndWait(
        object_to_signal: RawHandle,
        object_to_wait_on: RawHandle,
        milliseconds: u32,
        alertable: i32,
    ) -> u32;
    fn GetModuleHandleW(module_name: *const u16) -> RawHandle;
    fn GetProcAddress(module: RawHandle, proc_name: *const u8) -> *mut core::ffi::c_void;
}

/// `INFINITE` — `WaitForSingleObjectEx`/`WaitForMultipleObjectsEx`'s
/// sentinel `dwMilliseconds` value meaning "never time out."
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_ABANDONED_0: u32 = 0x0000_0080;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_IO_COMPLETION: u32 = 0x0000_00C0;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;

/// `GetStdHandle`/`SetStdHandle`'s `nStdHandle` slot selector for the
/// process's standard input. Defined as `(DWORD)-10` — a negative index
/// cast to an unsigned parameter type, not a real handle table offset.
pub const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;
/// `GetStdHandle`/`SetStdHandle`'s `nStdHandle` slot selector for the
/// process's standard output. Defined as `(DWORD)-11`.
pub const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
/// `GetStdHandle`/`SetStdHandle`'s `nStdHandle` slot selector for the
/// process's standard error. Defined as `(DWORD)-12`.
pub const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;

/// `GetStdHandle`'s own sentinel for "this call itself failed" — distinct
/// from a `NULL` return, which means the slot has no handle assigned rather
/// than that the call failed.
const INVALID_HANDLE_VALUE: isize = -1;

/// Create an anonymous pipe, returning `(read_handle, write_handle)`.
/// Neither end is inheritable by a spawned child yet — pass whichever end a
/// child needs through [`set_inheritable`] first, matching Windows'
/// non-inheritable-by-default convention rather than plumbing a
/// `SECURITY_ATTRIBUTES` struct through this call.
pub fn create_pipe() -> Result<(RawHandle, RawHandle), Win32Error> {
    let mut read_handle: RawHandle = core::ptr::null_mut();
    let mut write_handle: RawHandle = core::ptr::null_mut();
    // SAFETY: both out-pointers are valid, non-null, and point at
    // appropriately-sized `RawHandle` locals; `pipe_attributes = NULL`
    // requests default (non-inheritable) security attributes, a documented
    // valid input, not a dereferenced null.
    let ok = unsafe { CreatePipe(&mut read_handle, &mut write_handle, core::ptr::null(), 0) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok((read_handle, write_handle))
    }
}

/// Duplicate `handle` within the current process — the Windows analog of
/// Unix `dup`. The duplicate has the same access rights as `handle` and is
/// independently closeable; closing one does not affect the other. Set
/// `inheritable` to `true` if the duplicate should be visible to a
/// subsequently spawned child (e.g. the pipe end handed to a coprocess).
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle owned by the caller (or
/// otherwise known to be safe to pass here) — Windows doesn't guarantee a
/// stale or reused handle value fails cleanly the way a Unix `dup` on a
/// closed fd does in every case (pseudo-handles and handle-value reuse are
/// both real edge cases).
pub unsafe fn duplicate(handle: RawHandle, inheritable: bool) -> Result<RawHandle, Win32Error> {
    let mut target: RawHandle = core::ptr::null_mut();
    // SAFETY: `handle` is a caller-supplied, presumed-valid handle (the
    // caller's responsibility, same as every other function here); source
    // and target process handles are both the current process's pseudo
    // handle, a documented, always-valid constant; `target` is a valid
    // out-pointer.
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut target,
            0,
            i32::from(inheritable),
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(target)
    }
}

/// Mark `handle` inheritable (visible to a child spawned after this call
/// with handle inheritance enabled) or not — `SetHandleInformation` with
/// `HANDLE_FLAG_INHERIT`, the inverse of Unix's `FD_CLOEXEC`: a Windows
/// handle starts non-inheritable, so this is the "opt in" call, not an
/// "opt out" one.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle owned by the caller.
pub unsafe fn set_inheritable(handle: RawHandle, inheritable: bool) -> Result<(), Win32Error> {
    // SAFETY: `handle` is caller-supplied; the flags are a plain bitmask,
    // not a pointer.
    let ok = unsafe {
        SetHandleInformation(
            handle,
            HANDLE_FLAG_INHERIT,
            if inheritable { HANDLE_FLAG_INHERIT } else { 0 },
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Read `handle`'s current handle-information flags — `GetHandleInformation`,
/// the read-side counterpart to [`set_inheritable`]'s write-only
/// `SetHandleInformation` wrapper. Returns the raw flags bitmask
/// unmodified (the `HANDLE_FLAG_INHERIT` bit [`set_inheritable`] itself
/// toggles, plus `HANDLE_FLAG_PROTECT_FROM_CLOSE`, which this crate doesn't
/// otherwise expose) — deciding what to do with it is the caller's policy,
/// the same way this crate exposes other raw bitmask fields
/// (`FILE_ATTRIBUTE_*`, `ENABLE_*`) without deciding what they mean.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle.
pub unsafe fn handle_information(handle: RawHandle) -> Result<u32, Win32Error> {
    let mut flags: u32 = 0;
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `flags` is a valid out-pointer.
    let ok = unsafe { GetHandleInformation(handle, &mut flags) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(flags)
    }
}

/// Close `handle` — the Windows analog of Unix `close`. Closing an
/// already-closed or otherwise invalid handle fails with
/// [`Win32Error::ERROR_INVALID_HANDLE`] rather than being a silent no-op.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle owned by the caller, not
/// used again (by this crate or anything else) after this call returns —
/// the same "don't use it again" obligation Unix `close` places on a raw fd.
pub unsafe fn close(handle: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `handle` is caller-supplied; `CloseHandle` has no precondition
    // beyond "a handle value", and reports an invalid one as a normal
    // `FALSE`/`GetLastError` failure rather than undefined behavior.
    let ok = unsafe { CloseHandle(handle) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// How many bytes are currently available to read from `pipe_read_handle`
/// without blocking — `PeekNamedPipe`, the pipe-specific analog of
/// [`crate::console::wait_readable`]'s `WaitForSingleObject`-based check.
/// An anonymous pipe read end from [`create_pipe`] isn't usable the same way
/// `wait_readable` uses a console input handle — Windows' answer for "is
/// there data yet, don't block" on a pipe is this call instead. Does not
/// consume any data: a subsequent real read still sees every byte this
/// reports as available.
///
/// # Safety
///
/// `pipe_read_handle` must be a currently-open, valid handle to the read end
/// of a pipe.
pub unsafe fn pipe_bytes_available(pipe_read_handle: RawHandle) -> Result<u32, Win32Error> {
    let mut total_avail: u32 = 0;
    // SAFETY: `pipe_read_handle` is caller-supplied per this function's own
    // safety contract; passing NULL for the buffer/bytes-read/
    // bytes-left-this-message out-parameters is documented valid when the
    // caller only wants the total-available count (`buffer_size = 0`);
    // `total_avail` is a valid out-pointer.
    let ok = unsafe {
        PeekNamedPipe(
            pipe_read_handle,
            core::ptr::null_mut(),
            0,
            core::ptr::null_mut(),
            &mut total_avail,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(total_avail)
    }
}

/// Read one of the calling process's standard handle slots (`slot`: one of
/// [`STD_INPUT_HANDLE`]/[`STD_OUTPUT_HANDLE`]/[`STD_ERROR_HANDLE`]) —
/// `GetStdHandle`, the primitive `spawn_suspended`'s own doc comment
/// describes redirection as built on ("swapping the parent's std-handle
/// slots before spawning"). `Ok(None)` means the slot has no handle
/// assigned (`GetStdHandle`'s documented `NULL`-without-`GetLastError`-
/// failure outcome, e.g. a GUI process with no console), distinct from
/// `Err`, which means the call itself failed
/// (`INVALID_HANDLE_VALUE`).
pub fn get_std_handle(slot: u32) -> Result<Option<RawHandle>, Win32Error> {
    // SAFETY: `slot` is a plain `DWORD` selector, not a pointer; any `u32`
    // value is a valid (if possibly unrecognized) argument to
    // `GetStdHandle`.
    let handle = unsafe { GetStdHandle(slot) };
    if handle as isize == INVALID_HANDLE_VALUE {
        Err(Win32Error::last())
    } else if handle.is_null() {
        Ok(None)
    } else {
        Ok(Some(handle))
    }
}

/// Replace one of the calling process's standard handle slots — `SetStdHandle`,
/// the other half of the "swap the parent's std-handle slots before
/// spawning" redirection model [`get_std_handle`]'s own doc references.
/// Affects only this process's own view (and anything a subsequent
/// `CreateProcessW`-style spawn inherits from it); it does not duplicate or
/// close the handle previously in that slot.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle (or a documented pseudo-
/// handle) that outlives its use as a standard handle — this call does not
/// take ownership of it, matching `SetStdHandle`'s own documented contract.
pub unsafe fn set_std_handle(slot: u32, handle: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `slot` is a plain `DWORD` selector; `handle` is caller-supplied
    // per this function's own safety contract.
    let ok = unsafe { SetStdHandle(slot, handle) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Create (or open, if `name` already names an existing one) a named or
/// unnamed mutex — `CreateMutexW`, the Windows analog of `flock`'s
/// cross-process locking, but as a standalone kernel object rather than a
/// file-descriptor operation. `name`, if given, makes the mutex visible to
/// any process that names the same string (e.g. guarding concurrent writes
/// to a shared history file from multiple shell instances); `None` creates
/// an unnamed mutex only this process (and anything it hands the returned
/// handle to) can reach. `initial_owner` requests immediate ownership,
/// skipping a separate wait — the same shortcut `CreateMutexW`'s own
/// `bInitialOwner` parameter offers. Acquiring an *existing* mutex is
/// already covered by this crate's `WaitForSingleObject`-shaped wait
/// primitives (e.g. [`crate::process::wait`]'s underlying call, or
/// [`crate::console::wait_readable`]) once a handle is in hand — this
/// function only creates/opens the object itself.
pub fn create_mutex(name: Option<&str>, initial_owner: bool) -> Result<RawHandle, Win32Error> {
    extern crate alloc;
    let wide: Option<alloc::vec::Vec<u16>> =
        name.map(|n| n.encode_utf16().chain(core::iter::once(0)).collect());
    let name_ptr = wide.as_ref().map_or(core::ptr::null(), |w| w.as_ptr());
    // SAFETY: `mutex_attributes = NULL` requests default (non-inheritable)
    // security attributes, a documented valid input; `name_ptr` is either
    // NULL (documented as "create an unnamed mutex") or a valid,
    // NUL-terminated UTF-16 string kept alive for the duration of this call.
    let handle = unsafe { CreateMutexW(core::ptr::null(), i32::from(initial_owner), name_ptr) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Release ownership of `mutex`, previously acquired via a
/// `WaitForSingleObject`-shaped wait (or [`create_mutex`]'s own
/// `initial_owner: true`) — `ReleaseMutex`.
///
/// # Safety
///
/// `mutex` must be a currently-open, valid mutex handle currently owned by
/// the calling thread.
pub unsafe fn release_mutex(mutex: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `mutex` is caller-supplied per this function's own safety
    // contract.
    let ok = unsafe { ReleaseMutex(mutex) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Create (or open, if `name` already names an existing one) a named or
/// unnamed counting semaphore — `CreateSemaphoreW`, alongside
/// [`create_mutex`] the other standard Win32 synchronization primitive.
/// `initial_count` is the semaphore's starting count (must be between `0`
/// and `maximum_count` inclusive); `maximum_count` is the upper bound
/// [`release_semaphore`] can raise it back to. `name`, if given, makes the
/// semaphore visible to any process naming the same string, the same as
/// [`create_mutex`]'s `name` parameter. No current `rush` feature asks for
/// this; filed for Win32 parity.
pub fn create_semaphore(
    name: Option<&str>,
    initial_count: i32,
    maximum_count: i32,
) -> Result<RawHandle, Win32Error> {
    extern crate alloc;
    let wide: Option<alloc::vec::Vec<u16>> =
        name.map(|n| n.encode_utf16().chain(core::iter::once(0)).collect());
    let name_ptr = wide.as_ref().map_or(core::ptr::null(), |w| w.as_ptr());
    // SAFETY: `semaphore_attributes = NULL` requests default (non-inheritable)
    // security attributes, a documented valid input; `name_ptr` is either
    // NULL (documented as "create an unnamed semaphore") or a valid,
    // NUL-terminated UTF-16 string kept alive for the duration of this call.
    let handle =
        unsafe { CreateSemaphoreW(core::ptr::null(), initial_count, maximum_count, name_ptr) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Increase `semaphore`'s count by `release_count`, returning the count
/// just before the release — `ReleaseSemaphore`. Acquiring is already
/// covered by this crate's `WaitForSingleObject`-shaped wait primitives
/// (e.g. [`crate::process::wait`]'s underlying call) once a handle is in
/// hand — this function only releases.
///
/// # Safety
///
/// `semaphore` must be a currently-open, valid semaphore handle.
pub unsafe fn release_semaphore(
    semaphore: RawHandle,
    release_count: i32,
) -> Result<i32, Win32Error> {
    let mut previous_count: i32 = 0;
    // SAFETY: `semaphore` is caller-supplied per this function's own safety
    // contract; `previous_count` is a valid, distinct local out-pointer.
    let ok = unsafe { ReleaseSemaphore(semaphore, release_count, &mut previous_count) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(previous_count)
    }
}

/// [`wait_single_ex`]/[`wait_multiple_ex`]'s outcome. `Signaled`/
/// `Abandoned` carry the index of the handle that woke the wait (always
/// `0` for [`wait_single_ex`]; the position within the passed slice for
/// [`wait_multiple_ex`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    /// The handle at this index became signaled normally.
    Signaled(usize),
    /// The handle at this index was a mutex whose previous owner
    /// terminated without releasing it — still counts as acquired, but
    /// the caller should treat any state it protected as possibly
    /// inconsistent, same as Windows' own documented `WAIT_ABANDONED`
    /// semantics.
    Abandoned(usize),
    /// No handle became signaled before `timeout_ms` elapsed.
    TimedOut,
    /// The wait returned early because an asynchronous procedure call
    /// (APC) was queued to this thread and `alertable` was `true` — no
    /// handle was signaled.
    IoCompletion,
}

/// Alertable-wait variant of [`crate::console::wait_readable`]'s
/// `WaitForSingleObject` — `WaitForSingleObjectEx`. Adds `alertable`,
/// which lets the wait return early (reporting
/// [`WaitResult::IoCompletion`]) if an APC is queued to this thread while
/// waiting; pass `false` to behave identically to the non-`Ex` wait. No
/// current `rush` feature uses APCs, so `alertable: true` has no realistic
/// use yet — filed for Win32 parity.
///
/// # Safety
///
/// `handle` must be a currently-open, valid, waitable handle.
pub unsafe fn wait_single_ex(
    handle: RawHandle,
    timeout_ms: Option<u32>,
    alertable: bool,
) -> Result<WaitResult, Win32Error> {
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; the other two parameters are plain values, not pointers.
    let result = unsafe {
        WaitForSingleObjectEx(handle, timeout_ms.unwrap_or(INFINITE), i32::from(alertable))
    };
    match result {
        WAIT_OBJECT_0 => Ok(WaitResult::Signaled(0)),
        WAIT_ABANDONED_0 => Ok(WaitResult::Abandoned(0)),
        WAIT_TIMEOUT => Ok(WaitResult::TimedOut),
        WAIT_IO_COMPLETION => Ok(WaitResult::IoCompletion),
        _ => Err(Win32Error::last()),
    }
}

/// Alertable-wait variant of [`crate::process::wait_any`]'s
/// `WaitForMultipleObjects` — `WaitForMultipleObjectsEx`. `wait_all`
/// selects between "any one of `handles`" (`false`) and "every handle in
/// `handles`" (`true`) becoming signaled; `alertable` is the same
/// APC-wakeup opt-in as [`wait_single_ex`]. Unlike `wait_any` (scoped to
/// process handles, which are never abandoned), this generic wait also
/// reports [`WaitResult::Abandoned`] for a mutex whose owner terminated
/// without releasing it.
///
/// `handles` must be non-empty and no longer than
/// [`crate::process::MAXIMUM_WAIT_OBJECTS`] — `WaitForMultipleObjectsEx`'s
/// own documented limit; passing more (or zero) reports
/// [`Win32Error::ERROR_INVALID_PARAMETER`], the same failure the raw call
/// itself would report.
///
/// # Safety
///
/// Every handle in `handles` must be currently-open and valid.
pub unsafe fn wait_multiple_ex(
    handles: &[RawHandle],
    wait_all: bool,
    timeout_ms: Option<u32>,
    alertable: bool,
) -> Result<WaitResult, Win32Error> {
    // SAFETY: `handles` is a caller-supplied slice of valid handles per
    // this function's own safety contract; `handles.as_ptr()`/`.len()`
    // describe that same slice, a valid input `WaitForMultipleObjectsEx`
    // documents (including reporting `ERROR_INVALID_PARAMETER` itself for
    // an empty or oversized one, rather than this wrapper pre-checking).
    let result = unsafe {
        WaitForMultipleObjectsEx(
            handles.len() as u32,
            handles.as_ptr(),
            i32::from(wait_all),
            timeout_ms.unwrap_or(INFINITE),
            i32::from(alertable),
        )
    };
    match result {
        WAIT_TIMEOUT => Ok(WaitResult::TimedOut),
        WAIT_IO_COMPLETION => Ok(WaitResult::IoCompletion),
        WAIT_FAILED => Err(Win32Error::last()),
        index if (index as usize) < handles.len() => Ok(WaitResult::Signaled(index as usize)),
        index
            if index >= WAIT_ABANDONED_0
                && ((index - WAIT_ABANDONED_0) as usize) < handles.len() =>
        {
            Ok(WaitResult::Abandoned((index - WAIT_ABANDONED_0) as usize))
        }
        _ => Err(Win32Error::last()),
    }
}

/// Atomically signal `to_signal` and wait on `to_wait_on` —
/// `SignalObjectAndWait`, avoiding the race a caller would otherwise
/// accept by making two separate calls (a signal on one object followed
/// by a separate wait on another leaves a window where another thread
/// could act between them). `to_signal` must be a mutex, semaphore, or
/// event — the same object kinds [`release_mutex`]/[`release_semaphore`]/
/// a manual-or-auto-reset event accept. `alertable` is the same APC-wakeup
/// opt-in as [`wait_single_ex`]. No current `rush` feature asks for this;
/// filed for Win32 parity.
///
/// # Safety
///
/// `to_signal` and `to_wait_on` must each be a currently-open, valid
/// handle of a kind `SignalObjectAndWait` accepts.
pub unsafe fn signal_and_wait(
    to_signal: RawHandle,
    to_wait_on: RawHandle,
    timeout_ms: Option<u32>,
    alertable: bool,
) -> Result<WaitResult, Win32Error> {
    // SAFETY: both handles are caller-supplied per this function's own
    // safety contract; the remaining parameters are plain values, not
    // pointers.
    let result = unsafe {
        SignalObjectAndWait(
            to_signal,
            to_wait_on,
            timeout_ms.unwrap_or(INFINITE),
            i32::from(alertable),
        )
    };
    match result {
        WAIT_OBJECT_0 => Ok(WaitResult::Signaled(0)),
        WAIT_ABANDONED_0 => Ok(WaitResult::Abandoned(0)),
        WAIT_TIMEOUT => Ok(WaitResult::TimedOut),
        WAIT_IO_COMPLETION => Ok(WaitResult::IoCompletion),
        _ => Err(Win32Error::last()),
    }
}

type CompareObjectHandlesFn = unsafe extern "system" fn(RawHandle, RawHandle) -> i32;

/// Ask Windows directly whether `first` and `second` refer to the same
/// kernel object — `CompareObjectHandles`, the documented-correct
/// alternative to comparing raw handle values, which isn't guaranteed
/// reliable (a handle value can be reused after its object is closed, and
/// two distinct handle values can validly refer to the same object, e.g.
/// via [`duplicate`]). Resolved via `GetProcAddress` at call time rather
/// than this crate's usual static `#[link]` import: despite being a real,
/// always-present `kernel32.dll` export (forwarded from `KernelBase.dll`),
/// some Windows SDK versions' `kernel32.lib` import library omits a static
/// stub for it, which fails to *link* (not just run) on those toolchains —
/// confirmed by a real CI failure on this crate's own `windows-latest`
/// runner. Kernel32 is never unloaded for the lifetime of a process, so
/// this lookup can't outlive its module.
///
/// # Safety
///
/// `first` and `second` must each be a currently-open, valid handle.
pub unsafe fn same_object(first: RawHandle, second: RawHandle) -> Result<bool, Win32Error> {
    const KERNEL32: &[u16] = &[
        b'k' as u16,
        b'e' as u16,
        b'r' as u16,
        b'n' as u16,
        b'e' as u16,
        b'l' as u16,
        b'3' as u16,
        b'2' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    // `CompareObjectHandles` is genuinely implemented in `KernelBase.dll`;
    // `kernel32.dll` normally just forwards to it, but this crate's own CI
    // (`windows-latest`) confirmed a real machine where `GetProcAddress`
    // against `kernel32.dll` alone doesn't resolve it — so this module is
    // tried second, as a fallback, rather than relied on as the primary.
    const KERNELBASE: &[u16] = &[
        b'k' as u16,
        b'e' as u16,
        b'r' as u16,
        b'n' as u16,
        b'e' as u16,
        b'l' as u16,
        b'b' as u16,
        b'a' as u16,
        b's' as u16,
        b'e' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    const PROC_NAME: &[u8] = b"CompareObjectHandles\0";

    // SAFETY: both name arrays are valid, NUL-terminated UTF-16 strings
    // naming modules that are always already loaded in any Win32 process
    // (`kernelbase.dll` is `kernel32.dll`'s own dependency).
    let proc = [KERNEL32, KERNELBASE].into_iter().find_map(|name| {
        let module = unsafe { GetModuleHandleW(name.as_ptr()) };
        if module.is_null() {
            return None;
        }
        // SAFETY: `module` is a valid module handle just obtained above;
        // `PROC_NAME` is a valid, NUL-terminated ASCII string.
        let proc = unsafe { GetProcAddress(module, PROC_NAME.as_ptr()) };
        if proc.is_null() { None } else { Some(proc) }
    });
    let Some(proc) = proc else {
        // ERROR_PROC_NOT_FOUND — Windows' own documented code for this,
        // not a distinct error this crate invents.
        return Err(Win32Error::from_raw(127));
    };
    // SAFETY: `proc` is a non-null pointer `GetProcAddress` reported for
    // `"CompareObjectHandles"`, which has this exact signature per its
    // documented prototype; both handles are caller-supplied per this
    // function's own safety contract.
    let compare: CompareObjectHandlesFn = unsafe { core::mem::transmute(proc) };
    Ok(unsafe { compare(first, second) != 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pipe_returns_two_distinct_handles() {
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        assert!(!read_handle.is_null());
        assert!(!write_handle.is_null());
        assert_ne!(read_handle, write_handle);
        // SAFETY: both handles are freshly created, valid, and not used
        // again after this.
        unsafe {
            close(read_handle).unwrap();
            close(write_handle).unwrap();
        }
    }

    #[test]
    fn pipe_carries_bytes_end_to_end() {
        use std::io::{Read, Write};
        use std::os::windows::io::{FromRawHandle, OwnedHandle};

        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: both handles are freshly created, valid, and uniquely
        // owned by this test — nothing else holds or will close them.
        let mut reader =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(read_handle as _) });
        let mut writer =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(write_handle as _) });

        writer.write_all(b"rusty_win32").unwrap();
        drop(writer); // close the write end so the read below sees EOF

        let mut got = std::string::String::new();
        reader.read_to_string(&mut got).unwrap();
        assert_eq!(got, "rusty_win32");
    }

    #[test]
    fn duplicate_produces_an_independently_closeable_handle() {
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: `write_handle` is freshly created and valid.
        let dup =
            unsafe { duplicate(write_handle, false) }.expect("DuplicateHandle should succeed");
        assert_ne!(dup, write_handle);

        // SAFETY: all three handles are valid and each is closed exactly
        // once; `dup` and `write_handle` are independent handles to the
        // same object, so closing one doesn't invalidate the other.
        unsafe {
            close(write_handle).unwrap();
            close(dup).unwrap();
            close(read_handle).unwrap();
        }
    }

    #[test]
    fn set_inheritable_round_trips() {
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: `write_handle`/`read_handle` are freshly created and valid
        // for the duration of this test.
        unsafe {
            set_inheritable(write_handle, true).expect("marking inheritable should succeed");
            set_inheritable(write_handle, false).expect("clearing inheritable should succeed");
            close(read_handle).unwrap();
            close(write_handle).unwrap();
        }
    }

    #[test]
    fn handle_information_reflects_set_inheritable() {
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: `write_handle` is freshly created and valid for the
        // duration of this test.
        unsafe {
            set_inheritable(write_handle, true).expect("marking inheritable should succeed");
            let flags_when_set =
                handle_information(write_handle).expect("GetHandleInformation should succeed");
            assert_ne!(
                flags_when_set & HANDLE_FLAG_INHERIT,
                0,
                "the inherit bit should be set after set_inheritable(true)"
            );

            set_inheritable(write_handle, false).expect("clearing inheritable should succeed");
            let flags_when_cleared =
                handle_information(write_handle).expect("GetHandleInformation should succeed");
            assert_eq!(
                flags_when_cleared & HANDLE_FLAG_INHERIT,
                0,
                "the inherit bit should be clear after set_inheritable(false)"
            );

            close(read_handle).unwrap();
            close(write_handle).unwrap();
        }
    }

    #[test]
    fn closing_an_already_closed_handle_fails() {
        // Exact error code isn't part of `CloseHandle`'s documented
        // contract (a reused handle value is possible in principle), so
        // this only pins "fails", not a specific `Win32Error`.
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: the double-close on `write_handle` is the specific
        // documented-failure case under test, not a real use-after-close;
        // nothing else touches this handle value in between.
        unsafe {
            close(write_handle).unwrap();
            assert!(close(write_handle).is_err());
            close(read_handle).unwrap();
        }
    }

    #[test]
    fn pipe_bytes_available_reports_zero_for_an_empty_pipe() {
        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: `read_handle` is freshly created and valid.
        let avail =
            unsafe { pipe_bytes_available(read_handle) }.expect("PeekNamedPipe should succeed");
        assert_eq!(avail, 0);
        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            close(read_handle).unwrap();
            close(write_handle).unwrap();
        }
    }

    #[test]
    fn pipe_bytes_available_reports_written_data_without_consuming_it() {
        use std::io::{Read, Write};
        use std::os::windows::io::{FromRawHandle, OwnedHandle};

        let (read_handle, write_handle) = create_pipe().expect("CreatePipe should succeed");
        // SAFETY: `write_handle` is freshly created, valid, and uniquely
        // owned by this test.
        let mut writer =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(write_handle as _) });
        writer.write_all(b"rusty_win32").unwrap();

        // SAFETY: `read_handle` is freshly created, valid, and still open
        // (not yet wrapped/moved into an owning `File` below).
        let avail =
            unsafe { pipe_bytes_available(read_handle) }.expect("PeekNamedPipe should succeed");
        assert_eq!(avail, "rusty_win32".len() as u32);

        drop(writer); // close the write end so the read below sees EOF after the data

        // SAFETY: `read_handle` is still the same valid handle; nothing else
        // holds or will close it.
        let mut reader =
            std::fs::File::from(unsafe { OwnedHandle::from_raw_handle(read_handle as _) });
        let mut got = std::string::String::new();
        reader.read_to_string(&mut got).unwrap();
        assert_eq!(
            got, "rusty_win32",
            "peeking must not have consumed any bytes"
        );
    }

    #[test]
    fn get_std_handle_succeeds_for_every_standard_slot() {
        for slot in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            get_std_handle(slot).expect("GetStdHandle should succeed for a well-known slot");
        }
    }

    #[test]
    fn set_std_handle_then_get_std_handle_round_trips() {
        let original = get_std_handle(STD_INPUT_HANDLE).expect("GetStdHandle should succeed");
        let (read_end, write_end) = create_pipe().expect("CreatePipe should succeed");

        // SAFETY: `read_end` is a just-created, valid handle that outlives
        // its use as the standard-input slot for the duration of this test.
        unsafe { set_std_handle(STD_INPUT_HANDLE, read_end) }.expect("SetStdHandle should succeed");
        let swapped = get_std_handle(STD_INPUT_HANDLE).expect("GetStdHandle should succeed");
        assert_eq!(swapped, Some(read_end));

        // SAFETY: `original` (if any) is the process's own handle from
        // before this test began, or NULL to restore "no handle assigned" —
        // either way a safe value to hand back to `SetStdHandle`.
        unsafe { set_std_handle(STD_INPUT_HANDLE, original.unwrap_or(core::ptr::null_mut())) }
            .expect("SetStdHandle should succeed restoring the original handle");

        // SAFETY: both pipe ends are still open and not used again.
        unsafe {
            let _ = close(read_end);
            let _ = close(write_end);
        }
    }

    #[test]
    fn create_mutex_then_release_mutex_round_trips() {
        let mutex = create_mutex(None, false).expect("CreateMutexW should succeed");

        // SAFETY: `mutex` is a freshly created, valid, waitable handle.
        let acquired = unsafe { crate::console::wait_readable(mutex, 0) }
            .expect("WaitForSingleObject should succeed acquiring an unowned mutex");
        assert!(
            acquired,
            "an unowned mutex should be immediately acquirable"
        );

        // SAFETY: `mutex` is a valid mutex handle currently owned by this
        // thread (just acquired above); this is the operation under test.
        unsafe { release_mutex(mutex) }.expect("ReleaseMutex should succeed");

        // SAFETY: `mutex` is still a valid, currently-open handle, closed
        // exactly once and not used again after this.
        unsafe { close(mutex).unwrap() };
    }

    #[test]
    fn create_mutex_with_initial_owner_starts_already_owned() {
        let mutex = create_mutex(Some("rusty_win32_test_mutex_initial_owner"), true)
            .expect("CreateMutexW should succeed");

        // SAFETY: `mutex` is valid and currently owned by this thread
        // (`initial_owner: true` above).
        unsafe { release_mutex(mutex) }
            .expect("ReleaseMutex should succeed releasing initial ownership");

        // SAFETY: `mutex` is still a valid, currently-open handle, closed
        // exactly once and not used again after this.
        unsafe { close(mutex).unwrap() };
    }

    #[test]
    fn create_semaphore_then_release_semaphore_round_trips() {
        let semaphore = create_semaphore(None, 1, 1).expect("CreateSemaphoreW should succeed");

        // SAFETY: `semaphore` is a freshly created, valid, waitable handle
        // with an initial count of 1.
        let acquired = unsafe { crate::console::wait_readable(semaphore, 0) }
            .expect("WaitForSingleObject should succeed acquiring a non-zero-count semaphore");
        assert!(
            acquired,
            "a semaphore with initial_count 1 should be immediately acquirable once"
        );

        // The count is now 0 — a second immediate wait should time out.
        // SAFETY: same handle.
        let acquired_again = unsafe { crate::console::wait_readable(semaphore, 0) }
            .expect("WaitForSingleObject should succeed (report a timeout, not fail)");
        assert!(
            !acquired_again,
            "a semaphore just drained to 0 should not be immediately acquirable again"
        );

        // SAFETY: `semaphore` is a valid, currently-open semaphore handle;
        // this is the operation under test.
        let previous =
            unsafe { release_semaphore(semaphore, 1) }.expect("ReleaseSemaphore should succeed");
        assert_eq!(previous, 0, "count just before this release should be 0");

        // SAFETY: same handle, now released back to a non-zero count.
        let acquired_after_release = unsafe { crate::console::wait_readable(semaphore, 0) }
            .expect("WaitForSingleObject should succeed after the semaphore was released");
        assert!(
            acquired_after_release,
            "the semaphore should be acquirable again after release_semaphore"
        );

        // SAFETY: `semaphore` is still a valid, currently-open handle,
        // closed exactly once and not used again after this.
        unsafe { close(semaphore).unwrap() };
    }

    #[test]
    fn wait_single_ex_reports_signaled_then_timed_out() {
        let semaphore = create_semaphore(None, 1, 1).expect("CreateSemaphoreW should succeed");

        // SAFETY: `semaphore` is a freshly created, valid, waitable handle
        // with an initial count of 1.
        let result = unsafe { wait_single_ex(semaphore, Some(0), false) }
            .expect("WaitForSingleObjectEx should succeed acquiring a non-zero-count semaphore");
        assert_eq!(result, WaitResult::Signaled(0));

        // The count is now 0 — a second immediate wait should time out.
        // SAFETY: same handle.
        let result = unsafe { wait_single_ex(semaphore, Some(0), false) }
            .expect("WaitForSingleObjectEx should succeed (report a timeout, not fail)");
        assert_eq!(result, WaitResult::TimedOut);

        // SAFETY: `semaphore` is still a valid, currently-open handle,
        // closed exactly once and not used again after this.
        unsafe { close(semaphore).unwrap() };
    }

    #[test]
    fn wait_multiple_ex_reports_the_signaled_index() {
        let first = create_semaphore(None, 0, 1).expect("CreateSemaphoreW should succeed");
        let second = create_semaphore(None, 1, 1).expect("CreateSemaphoreW should succeed");

        // SAFETY: both handles are freshly created and valid; only
        // `second` starts with a non-zero count.
        let result = unsafe { wait_multiple_ex(&[first, second], false, Some(0), false) }
            .expect("WaitForMultipleObjectsEx should succeed");
        assert_eq!(
            result,
            WaitResult::Signaled(1),
            "only the second handle (index 1) has a non-zero count"
        );

        // SAFETY: both handles are still valid, each closed exactly once.
        unsafe {
            close(first).unwrap();
            close(second).unwrap();
        }
    }

    #[test]
    fn signal_and_wait_signals_one_semaphore_and_acquires_another() {
        let to_signal = create_semaphore(None, 0, 1).expect("CreateSemaphoreW should succeed");
        let to_wait_on = create_semaphore(None, 1, 1).expect("CreateSemaphoreW should succeed");

        // SAFETY: both handles are freshly created and valid; `to_signal`
        // is a semaphore (a valid `SignalObjectAndWait` signal-object
        // kind), `to_wait_on` currently has a non-zero count.
        let result = unsafe { signal_and_wait(to_signal, to_wait_on, Some(0), false) }
            .expect("SignalObjectAndWait should succeed");
        assert_eq!(result, WaitResult::Signaled(0));

        // `to_signal`'s count should now be 1 (released by the call above).
        // SAFETY: `to_signal` is still a valid, currently-open handle.
        let signaled_result = unsafe { wait_single_ex(to_signal, Some(0), false) }
            .expect("WaitForSingleObjectEx should succeed");
        assert_eq!(
            signaled_result,
            WaitResult::Signaled(0),
            "to_signal should have been released to a non-zero count"
        );

        // `to_wait_on`'s count should now be 0 (acquired by the call above).
        // SAFETY: `to_wait_on` is still a valid, currently-open handle.
        let drained_result = unsafe { wait_single_ex(to_wait_on, Some(0), false) }
            .expect("WaitForSingleObjectEx should succeed (report a timeout, not fail)");
        assert_eq!(
            drained_result,
            WaitResult::TimedOut,
            "to_wait_on should have been drained to 0 by the wait side of the call"
        );

        // SAFETY: both handles are still valid, each closed exactly once.
        unsafe {
            close(to_signal).unwrap();
            close(to_wait_on).unwrap();
        }
    }

    #[test]
    fn same_object_distinguishes_a_duplicate_from_an_unrelated_handle() {
        let original = create_mutex(None, false).expect("CreateMutexW should succeed");
        // SAFETY: `original` is a freshly created, valid handle.
        let duplicated =
            unsafe { duplicate(original, false) }.expect("DuplicateHandle should succeed");
        let unrelated = create_mutex(None, false).expect("CreateMutexW should succeed");

        // SAFETY: `original`/`duplicated` are both valid, currently-open
        // handles; this is the operation under test.
        assert!(
            unsafe { same_object(original, duplicated) }
                .expect("CompareObjectHandles should succeed"),
            "a duplicated handle should refer to the same object as the original"
        );
        // SAFETY: `original`/`unrelated` are both valid, currently-open
        // handles.
        assert!(
            !unsafe { same_object(original, unrelated) }
                .expect("CompareObjectHandles should succeed"),
            "two independently created mutexes should not be the same object"
        );

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            close(original).unwrap();
            close(duplicated).unwrap();
            close(unrelated).unwrap();
        }
    }
}
