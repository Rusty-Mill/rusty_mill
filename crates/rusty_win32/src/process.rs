//! Raw `CreateProcessW` spawn + wait — narrowly scoped to what Job-Object-
//! integrated background jobs need (rush's
//! `docs/WINDOWS_BACKEND_ANALYSIS.md` §4.1), not a replacement for
//! `std::process::Command`. Ordinary foreground spawn/wait already works
//! via `std::process::Command`, which resolves to the same underlying
//! `CreateProcessW`/`WaitForSingleObject` calls — keep using it for that.
//! The one thing it can't do is hand back the child's *thread* handle,
//! which starting a process suspended (`CREATE_SUSPENDED`) requires for
//! `resume` afterward: a process must be assigned to a
//! [`crate::job`] object *before* its main thread runs, and there is no
//! stable way to reach a `std::process::Child`'s thread handle.
//!
//! Command-line construction (Windows argv quoting) is the caller's
//! responsibility. `std::process::Command` already solves that correctly
//! and there's no public API to reuse its escaping logic here, so
//! [`spawn_suspended`] takes an already-built command-line string rather
//! than reimplementing argument quoting.

use crate::error::Win32Error;
use crate::handle::RawHandle;
use crate::time::Timespec;
use crate::wide::to_wide;

extern crate alloc;
use alloc::vec::Vec;

const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;

// Layout verified against mingw-w64's processthreadsapi.h
// (`_STARTUPINFOW`): `size_of` 104, `align_of` 8 on x86_64, checked by
// compiling a `_Static_assert`-based probe against the real header rather
// than hand-computed padding.
#[repr(C)]
#[derive(Default)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: RawHandle,
    h_std_output: RawHandle,
    h_std_error: RawHandle,
}

const _: () = assert!(core::mem::size_of::<StartupInfoW>() == 104);
const _: () = assert!(core::mem::align_of::<StartupInfoW>() == 8);

// Layout verified the same way against `_PROCESS_INFORMATION`: `size_of`
// 24, `align_of` 8 on x86_64.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessInformationRaw {
    h_process: RawHandle,
    h_thread: RawHandle,
    dw_process_id: u32,
    dw_thread_id: u32,
}

const _: () = assert!(core::mem::size_of::<ProcessInformationRaw>() == 24);
const _: () = assert!(core::mem::align_of::<ProcessInformationRaw>() == 8);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const core::ffi::c_void,
        thread_attributes: *const core::ffi::c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const core::ffi::c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformationRaw,
    ) -> i32;
    fn ResumeThread(thread: RawHandle) -> u32;
    fn WaitForSingleObject(handle: RawHandle, milliseconds: u32) -> u32;
    fn WaitForMultipleObjects(
        count: u32,
        handles: *const RawHandle,
        wait_all: i32,
        milliseconds: u32,
    ) -> u32;
    fn GetExitCodeProcess(process: RawHandle, exit_code: *mut u32) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> RawHandle;
    fn TerminateProcess(process: RawHandle, exit_code: u32) -> i32;
    fn GetProcessId(process: RawHandle) -> u32;
    fn Sleep(milliseconds: u32);
    fn SleepEx(milliseconds: u32, alertable: i32) -> u32;
    fn GetSystemInfo(system_info: *mut SystemInfo);
    fn GetTickCount64() -> u64;
    fn GetLogicalProcessorInformation(
        buffer: *mut SystemLogicalProcessorInformationRaw,
        return_length: *mut u32,
    ) -> i32;
    fn AddVectoredExceptionHandler(
        first_handler: u32,
        handler: VectoredExceptionHandler,
    ) -> *mut core::ffi::c_void;
    fn RemoveVectoredExceptionHandler(handle: *mut core::ffi::c_void) -> u32;
    fn SetUnhandledExceptionFilter(
        filter: Option<TopLevelExceptionFilter>,
    ) -> Option<TopLevelExceptionFilter>;
    fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    fn SetErrorMode(mode: u32) -> u32;
    fn GetPriorityClass(process: RawHandle) -> u32;
    fn SetPriorityClass(process: RawHandle, priority_class: u32) -> i32;
    fn GetProcessAffinityMask(
        process: RawHandle,
        process_affinity_mask: *mut usize,
        system_affinity_mask: *mut usize,
    ) -> i32;
    fn SetProcessAffinityMask(process: RawHandle, process_affinity_mask: usize) -> i32;
    fn GetExitCodeThread(thread: RawHandle, exit_code: *mut u32) -> i32;
    fn GetThreadTimes(
        thread: RawHandle,
        creation_time: *mut FileTime,
        exit_time: *mut FileTime,
        kernel_time: *mut FileTime,
        user_time: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        process: RawHandle,
        flags: u32,
        exe_name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: RawHandle,
        creation_time: *mut FileTime,
        exit_time: *mut FileTime,
        kernel_time: *mut FileTime,
        user_time: *mut FileTime,
    ) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(penv: *mut u16) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> RawHandle;
    fn Process32FirstW(snapshot: RawHandle, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: RawHandle, entry: *mut ProcessEntry32W) -> i32;
    fn Thread32First(snapshot: RawHandle, entry: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snapshot: RawHandle, entry: *mut ThreadEntry32) -> i32;
    fn OpenThread(desired_access: u32, inherit_handle: i32, thread_id: u32) -> RawHandle;
    fn SuspendThread(thread: RawHandle) -> u32;
}

/// `CreateToolhelp32Snapshot`'s `dwFlags`: include every thread currently
/// running on the system in the snapshot (there's no per-process filter at
/// the `CreateToolhelp32Snapshot` level — [`list_threads`] filters by
/// `th32OwnerProcessID` itself after the fact).
const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;

// THREADENTRY32: `size_of` 28, `align_of` 4 on x86_64. Verified against
// mingw-w64's `tlhelp32.h` the same way as `ProcessEntry32W` (a
// `_Static_assert` probe compiled with `x86_64-w64-mingw32-gcc` against the
// real header).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ThreadEntry32 {
    dw_size: u32,
    cnt_usage: u32,
    th32_thread_id: u32,
    th32_owner_process_id: u32,
    tp_base_pri: i32,
    tp_delta_pri: i32,
    dw_flags: u32,
}
const _: () = assert!(core::mem::size_of::<ThreadEntry32>() == 28);
const _: () = assert!(core::mem::align_of::<ThreadEntry32>() == 4);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, cnt_usage) == 4);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, th32_thread_id) == 8);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, th32_owner_process_id) == 12);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, tp_base_pri) == 16);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, tp_delta_pri) == 20);
const _: () = assert!(core::mem::offset_of!(ThreadEntry32, dw_flags) == 24);

/// `OpenThread`'s access-rights bit letting the returned handle be passed
/// to [`suspend_thread`]/[`resume`].
pub const THREAD_SUSPEND_RESUME: u32 = 0x0002;

/// `OpenThread`'s access-rights bit letting the returned handle be passed
/// to [`thread_exit_code`].
pub const THREAD_QUERY_INFORMATION: u32 = 0x0040;

/// `CreateToolhelp32Snapshot`'s `dwFlags`: include every process currently
/// running in the snapshot.
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

// PROCESSENTRY32W: `size_of` 568, `align_of` 8 on x86_64. Verified against
// mingw-w64's `tlhelp32.h` the same way as every other struct in this crate
// (an offset-extraction probe compiled with `x86_64-w64-mingw32-gcc`
// against the real header, reading each field's compile-time-constant
// `offsetof`/`sizeof` back out of the generated object code — the same
// technique as this crate's usual `_Static_assert` probes, needed here only
// because the exact total size wasn't obvious to guess up front, unlike
// this crate's other structs).
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: u64,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

// `[u16; 260]` doesn't implement `Default` (std only special-cases arrays
// up to length 32), so this is written by hand rather than derived.
impl Default for ProcessEntry32W {
    fn default() -> Self {
        ProcessEntry32W {
            dw_size: 0,
            cnt_usage: 0,
            th32_process_id: 0,
            th32_default_heap_id: 0,
            th32_module_id: 0,
            cnt_threads: 0,
            th32_parent_process_id: 0,
            pc_pri_class_base: 0,
            dw_flags: 0,
            sz_exe_file: [0u16; 260],
        }
    }
}

const _: () = assert!(core::mem::size_of::<ProcessEntry32W>() == 568);
const _: () = assert!(core::mem::align_of::<ProcessEntry32W>() == 8);
const _: () = assert!(core::mem::offset_of!(ProcessEntry32W, th32_default_heap_id) == 16);
const _: () = assert!(core::mem::offset_of!(ProcessEntry32W, th32_parent_process_id) == 32);
const _: () = assert!(core::mem::offset_of!(ProcessEntry32W, sz_exe_file) == 44);

/// `OpenProcess`'s access-rights bit letting the returned handle be passed to
/// [`terminate`].
pub const PROCESS_TERMINATE: u32 = 0x0001;
/// `OpenProcess`'s access-rights bit letting the returned handle be passed to
/// [`wait`]/[`wait_any`] (a process handle must carry this right to be
/// waitable at all).
pub const SYNCHRONIZE: u32 = 0x0010_0000;
/// `OpenProcess`'s access-rights bit letting the returned handle be passed to
/// [`times`].
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

// FILETIME: `size_of` 8, `align_of` 4 on x86_64 — mirrors `time.rs`'s
// private struct of the same shape; duplicated locally rather than shared,
// matching this crate's existing per-module-locality convention for tiny
// FFI-mirror structs (`fs.rs` does the same).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}
const _: () = assert!(core::mem::size_of::<FileTime>() == 8);
const _: () = assert!(core::mem::align_of::<FileTime>() == 4);

// SYSTEM_INFO: `size_of` 48, `align_of` 8 on x86_64. Verified against
// mingw-w64's `sysinfoapi.h` the same way as this crate's other structs (a
// `_Static_assert` probe compiled with `x86_64-w64-mingw32-gcc` against the
// real header). `dwOemId`'s union collapses to its `wProcessorArchitecture`/
// `wReserved` members here since [`logical_processor_count`] doesn't read
// either — only [`SystemInfo::number_of_processors`] is exposed.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemInfo {
    processor_architecture: u16,
    reserved: u16,
    page_size: u32,
    minimum_application_address: *mut core::ffi::c_void,
    maximum_application_address: *mut core::ffi::c_void,
    active_processor_mask: u64,
    number_of_processors: u32,
    processor_type: u32,
    allocation_granularity: u32,
    processor_level: u16,
    processor_revision: u16,
}
const _: () = assert!(core::mem::size_of::<SystemInfo>() == 48);
const _: () = assert!(core::mem::align_of::<SystemInfo>() == 8);
const _: () = assert!(core::mem::offset_of!(SystemInfo, page_size) == 4);
const _: () = assert!(core::mem::offset_of!(SystemInfo, minimum_application_address) == 8);
const _: () = assert!(core::mem::offset_of!(SystemInfo, maximum_application_address) == 16);
const _: () = assert!(core::mem::offset_of!(SystemInfo, active_processor_mask) == 24);
const _: () = assert!(core::mem::offset_of!(SystemInfo, number_of_processors) == 32);
const _: () = assert!(core::mem::offset_of!(SystemInfo, processor_type) == 36);
const _: () = assert!(core::mem::offset_of!(SystemInfo, allocation_granularity) == 40);
const _: () = assert!(core::mem::offset_of!(SystemInfo, processor_level) == 44);
const _: () = assert!(core::mem::offset_of!(SystemInfo, processor_revision) == 46);

// SYSTEM_LOGICAL_PROCESSOR_INFORMATION: `size_of` 32, `align_of` 8 on
// x86_64. Verified against mingw-w64's `winnt.h` the same way as this
// crate's other structs (a `_Static_assert` probe compiled with
// `x86_64-w64-mingw32-gcc` against the real header). The real struct's
// trailing member is a union (`ProcessorCore.Flags`/`NumaNode.NodeNumber`/
// `Cache: CACHE_DESCRIPTOR`/`Reserved: [u64; 2]`) — this crate only
// exposes `ProcessorMask`/`Relationship`, the two fields meaningful across
// every relationship kind, so the union is mirrored as opaque padding
// bytes rather than a Rust `union` this crate has no other use for yet.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemLogicalProcessorInformationRaw {
    processor_mask: usize,
    relationship: u32,
    _union: [u8; 20],
}
const _: () = assert!(core::mem::size_of::<SystemLogicalProcessorInformationRaw>() == 32);
const _: () = assert!(core::mem::align_of::<SystemLogicalProcessorInformationRaw>() == 8);
const _: () =
    assert!(core::mem::offset_of!(SystemLogicalProcessorInformationRaw, relationship) == 8);

// MEMORYSTATUSEX: `size_of` 64, `align_of` 8 on x86_64. Verified against
// mingw-w64's `sysinfoapi.h`/`winbase.h` the same way as this crate's other
// structs (a `_Static_assert` probe compiled with `x86_64-w64-mingw32-gcc`
// against the real header).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}
const _: () = assert!(core::mem::size_of::<MemoryStatusEx>() == 64);
const _: () = assert!(core::mem::align_of::<MemoryStatusEx>() == 8);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, memory_load) == 4);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, total_phys) == 8);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, avail_phys) == 16);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, total_page_file) == 24);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, avail_page_file) == 32);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, total_virtual) == 40);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, avail_virtual) == 48);
const _: () = assert!(core::mem::offset_of!(MemoryStatusEx, avail_extended_virtual) == 56);

/// 100ns ticks between the FILETIME epoch (1601-01-01) and the Unix epoch
/// (1970-01-01) — the same standard conversion constant `time.rs`/`fs.rs` use.
const FILETIME_UNIX_EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const HUNDRED_NS_PER_SEC: i64 = 10_000_000;
const NANOS_PER_HUNDRED_NS: i64 = 100;

/// `creation_time`/`exit_time` are real wall-clock `FILETIME`s (an absolute
/// point in time, like `time.rs::now_realtime`'s result) — this conversion
/// subtracts the epoch difference the same way that one does.
fn filetime_to_timespec(ft: FileTime) -> Timespec {
    let ticks_100ns =
        (i64::from(ft.high) << 32 | i64::from(ft.low)) - FILETIME_UNIX_EPOCH_DIFF_100NS;
    let secs = ticks_100ns.div_euclid(HUNDRED_NS_PER_SEC);
    let remainder_100ns = ticks_100ns.rem_euclid(HUNDRED_NS_PER_SEC);
    Timespec {
        secs,
        nanos: (remainder_100ns * NANOS_PER_HUNDRED_NS) as u32,
    }
}

/// `kernel_time`/`user_time` are documented as an *elapsed duration* (total
/// CPU time accumulated in that mode), not a point in FILETIME's own epoch —
/// there is no epoch to subtract, only a raw 100ns tick count to convert to
/// seconds/nanoseconds. Reuses [`Timespec`]'s shape the same way
/// `time.rs::now_monotonic`'s result already does for a non-wall-clock
/// duration.
fn filetime_to_duration(ft: FileTime) -> Timespec {
    let ticks_100ns = i64::from(ft.high) << 32 | i64::from(ft.low);
    let secs = ticks_100ns / HUNDRED_NS_PER_SEC;
    let remainder_100ns = ticks_100ns % HUNDRED_NS_PER_SEC;
    Timespec {
        secs,
        nanos: (remainder_100ns * NANOS_PER_HUNDRED_NS) as u32,
    }
}

/// [`times`]'s result — `GetProcessTimes`, the Windows analog of
/// `getrusage`/a `wait4`-reported `rusage` for CPU-time accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessTimes {
    /// When the process was created — a real wall-clock timestamp.
    pub creation: Timespec,
    /// When the process exited — a real wall-clock timestamp, but only
    /// meaningful once the process actually has; Windows reports this as
    /// zero for a still-running process, not an error.
    pub exit: Timespec,
    /// Total time spent executing in kernel mode, as an elapsed *duration*
    /// since process creation — not a wall-clock timestamp.
    pub kernel_time: Timespec,
    /// Total time spent executing in user mode, as an elapsed *duration*
    /// since process creation — not a wall-clock timestamp.
    pub user_time: Timespec,
}

/// [`thread_times`]'s result — `GetThreadTimes`, the thread-level
/// counterpart to [`ProcessTimes`]/[`times`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadTimes {
    /// When the thread was created — a real wall-clock timestamp.
    pub creation: Timespec,
    /// When the thread exited — a real wall-clock timestamp, but only
    /// meaningful once the thread actually has; Windows reports this as
    /// zero for a still-running thread, not an error.
    pub exit: Timespec,
    /// Total time spent executing in kernel mode, as an elapsed *duration*
    /// since thread creation — not a wall-clock timestamp.
    pub kernel_time: Timespec,
    /// Total time spent executing in user mode, as an elapsed *duration*
    /// since thread creation — not a wall-clock timestamp.
    pub user_time: Timespec,
}

/// `WaitForMultipleObjects`'s own hard cap on `nCount` — a documented Win32
/// limit, not one this crate invents. [`wait_any`] passing more than this
/// reports [`Win32Error::ERROR_INVALID_PARAMETER`], the same code the real
/// call fails with, rather than this crate pre-validating and inventing its
/// own error for the same condition.
pub const MAXIMUM_WAIT_OBJECTS: usize = 64;

/// A process started by [`spawn_suspended`], still suspended until
/// [`resume`] is called on `thread`.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedProcess {
    pub process: RawHandle,
    pub thread: RawHandle,
    pub process_id: u32,
    pub thread_id: u32,
}

/// Start `command_line` suspended (`CREATE_SUSPENDED`) — its main thread
/// does not run until [`resume`] is called on the returned
/// [`SpawnedProcess::thread`]. The gap between this call and `resume` is
/// exactly the window in which a caller assigns the process to a
/// [`crate::job`] object, so job membership is guaranteed before the
/// process (or any child it later spawns) executes a single instruction.
///
/// `inherit_handles` controls whether the child inherits the calling
/// process's currently-inheritable standard handles (see
/// [`crate::handle::set_inheritable`]) — there is no `STARTUPINFOW`
/// std-handle override here; redirect by swapping the parent's std-handle
/// slots before spawning, matching `winstdio`'s existing model in rush.
///
/// `new_process_group` requests `CREATE_NEW_PROCESS_GROUP`, putting the
/// child (and any descendants it spawns) in a console process group of its
/// own rather than the caller's. This is what makes it possible to later
/// interrupt *just* that child via
/// [`crate::console::generate_ctrl_event`]`(CTRL_BREAK_EVENT, group_id)` —
/// without it, a console control event has no way to target one child
/// process group instead of every process attached to the console at once.
/// `group_id` for that later call is this same [`SpawnedProcess::process_id`]
/// — Windows defines a new process group's id as its creating process's pid.
///
/// `environment` overrides the child's environment block; `None` inherits
/// the calling process's own environment unchanged (`CreateProcessW`'s
/// default when its `lpEnvironment` argument is null). A caller tracking
/// its own variable table separately from the real OS environment (rush's
/// `vars` module never calls `std::env::set_var`, so the two can diverge
/// after `export`/`unset`) needs `Some(..)` — build the block with
/// [`environment_block`] rather than hand-rolling it, since `CreateProcessW`
/// walks it by embedded NULs, not by a length this function could otherwise
/// validate.
///
/// # Safety
///
/// `command_line` must already be a valid, correctly-quoted Windows
/// command line for the target program (this function does no argument
/// quoting or escaping of its own). `environment`, if `Some`, must be a
/// well-formed Windows environment block — a sequence of NUL-terminated
/// `"NAME=value"` UTF-16 strings followed by one additional NUL — since
/// `CreateProcessW` reads it by scanning for that terminator, not by the
/// slice's own length; [`environment_block`] always produces one.
pub unsafe fn spawn_suspended(
    command_line: &str,
    inherit_handles: bool,
    new_process_group: bool,
    environment: Option<&[u16]>,
) -> Result<SpawnedProcess, Win32Error> {
    // `CreateProcessW` is documented as possibly writing into this buffer
    // (e.g. inserting a terminating NUL if `lpApplicationName` is NULL and
    // `lpCommandLine`'s first token exceeds `MAX_PATH`), so a `&str`'s
    // read-only pointer isn't sufficient — this must be an owned, mutable
    // buffer.
    let mut wide: Vec<u16> = crate::wide::to_wide(command_line);

    let mut startup_info = StartupInfoW {
        cb: core::mem::size_of::<StartupInfoW>() as u32,
        ..Default::default()
    };
    let mut process_info = ProcessInformationRaw::default();

    // `CREATE_UNICODE_ENVIRONMENT` is required whenever an explicit
    // environment block is passed — without it, `CreateProcessW` expects
    // an ANSI (8-bit) block instead and misreads ours.
    let mut creation_flags = CREATE_SUSPENDED;
    if environment.is_some() {
        creation_flags |= CREATE_UNICODE_ENVIRONMENT;
    }
    if new_process_group {
        creation_flags |= CREATE_NEW_PROCESS_GROUP;
    }
    let env_ptr = environment.map_or(core::ptr::null(), |e| {
        e.as_ptr().cast::<core::ffi::c_void>()
    });

    // SAFETY: `wide` is a valid, mutable, NUL-terminated UTF-16 buffer;
    // `startup_info`/`process_info` are valid, correctly-sized out
    // pointers with `cb` set as `CreateProcessW` requires; `env_ptr` is
    // either null (inherit) or a well-formed double-NUL-terminated block
    // per this function's own safety contract; every other pointer
    // argument is a documented-valid NULL (default security attributes,
    // no explicit application name/current directory override).
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            wide.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            i32::from(inherit_handles),
            creation_flags,
            env_ptr,
            core::ptr::null(),
            &mut startup_info,
            &mut process_info,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(SpawnedProcess {
        process: process_info.h_process,
        thread: process_info.h_thread,
        process_id: process_info.dw_process_id,
        thread_id: process_info.dw_thread_id,
    })
}

/// `EXTENDED_STARTUPINFO_PRESENT` — the `CreateProcessW` creation flag
/// declaring that `lpStartupInfo` points at a `STARTUPINFOEXW` (with a
/// live `lpAttributeList`), not a bare `STARTUPINFOW`. Verified against
/// mingw-w64's own `winbase.h` with a compiled `_Static_assert` probe.
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;

// STARTUPINFOEXW: `size_of` 112, `align_of` 8 on x86_64, `lpAttributeList`
// at offset 104 — exactly `StartupInfoW` (104 bytes) plus one trailing
// pointer, verified against mingw-w64's own `winbase.h` with a compiled
// `_Static_assert` probe (including the full `CreateProcessW` call it
// would go into, cast from `LPSTARTUPINFOW` the same way this module's
// own `spawn_suspended_with_pseudoconsole` does).
#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    lp_attribute_list: *mut u8,
}

const _: () = assert!(core::mem::size_of::<StartupInfoExW>() == 112);
const _: () = assert!(core::mem::align_of::<StartupInfoExW>() == 8);
const _: () = assert!(core::mem::offset_of!(StartupInfoExW, lp_attribute_list) == 104);

/// Start `command_line` suspended, hosted by an already-created
/// pseudoconsole — a wholly new function alongside [`spawn_suspended`]
/// (not a breaking change to it), for a fully-interactive child rather
/// than one with redirected stdio pipes. `hpc` supplies the child's
/// console I/O, so unlike `spawn_suspended` there is no `inherit_handles`
/// parameter — irrelevant once a pseudoconsole is in the picture.
///
/// Builds a one-attribute [`crate::conpty::AttributeList`], binds `hpc`
/// into it via `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, then calls
/// `CreateProcessW` with a `STARTUPINFOEXW` (`EXTENDED_STARTUPINFO_PRESENT`
/// set, `cb` sized for the *extended* struct — `CreateProcessW`'s own
/// documented requirement once that flag is set) rather than a bare
/// `StartupInfoW`. The attribute list is torn down (`Drop`) before this
/// function returns either way; only `hpc` itself outlives the call, for
/// the caller to eventually [`crate::conpty::resize`]/
/// [`crate::conpty::close`] once the child exits.
///
/// The full picture this function is one piece of: two anonymous pipe
/// pairs ([`crate::handle::create_pipe`]) → [`crate::conpty::create`]
/// bound to one pipe's read end and the other's write end (closing the
/// caller's own copies of those two ends right after, per `create`'s own
/// documented pattern) → **this function** → [`crate::conpty::resize`]
/// any time afterward → (after the child exits)
/// [`crate::conpty::close`].
///
/// # Safety
///
/// `command_line` must already be a valid, correctly-quoted Windows
/// command line for the target program, same contract as
/// `spawn_suspended`. `hpc` must be a currently-open, valid pseudoconsole
/// handle from [`crate::conpty::create`], not yet
/// [`crate::conpty::close`]-d.
pub unsafe fn spawn_suspended_with_pseudoconsole(
    command_line: &str,
    hpc: crate::conpty::Hpcon,
) -> Result<SpawnedProcess, Win32Error> {
    let mut wide: Vec<u16> = crate::wide::to_wide(command_line);

    let mut attribute_list = crate::conpty::AttributeList::init(1)?;
    // SAFETY: `hpc` is caller-supplied per this function's own safety
    // contract. For `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` specifically,
    // Windows' own documented usage (and its official ConPTY sample)
    // passes the `HPCON` handle's own pointer-sized *value* directly as
    // `lpValue` -- not `&hpc`, a pointer to a local holding that value.
    // Passing `&hpc` compiles and type-checks identically (both are
    // `*const c_void`-shaped) but is a real, silent bug: it binds the
    // address of this function's own stack slot instead of the handle
    // itself, so the spawned child gets a bogus pseudoconsole reference
    // and fails during its own console-subsystem startup
    // (`STATUS_DLL_INIT_FAILED`, confirmed by this function's own test
    // failing exactly that way before this was fixed).
    unsafe {
        attribute_list.update(
            crate::conpty::PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            hpc.cast_const(),
            core::mem::size_of::<crate::conpty::Hpcon>(),
        )
    }?;

    let mut startup_info_ex = StartupInfoExW {
        startup_info: StartupInfoW {
            cb: core::mem::size_of::<StartupInfoExW>() as u32,
            ..Default::default()
        },
        lp_attribute_list: attribute_list.as_mut_ptr(),
    };
    let mut process_info = ProcessInformationRaw::default();

    // SAFETY: `wide` is a valid, mutable, NUL-terminated UTF-16 buffer;
    // `startup_info_ex` is a valid, correctly-sized `STARTUPINFOEXW` with
    // `cb` set to its own (extended) `size_of` as `CreateProcessW`
    // requires once `EXTENDED_STARTUPINFO_PRESENT` is set; passing the
    // address of the embedded `startup_info` field is valid as a
    // `STARTUPINFOEXW*` since it's `StartupInfoExW`'s first field at
    // offset 0, the same address as the whole extended struct;
    // `process_info` is a valid, correctly-sized out pointer;
    // `inherit_handles = FALSE` since the pseudoconsole (not inherited
    // handles) supplies the child's console I/O; every other pointer
    // argument is a documented-valid NULL.
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            wide.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            core::ptr::null(),
            core::ptr::null(),
            &mut startup_info_ex.startup_info,
            &mut process_info,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(SpawnedProcess {
        process: process_info.h_process,
        thread: process_info.h_thread,
        process_id: process_info.dw_process_id,
        thread_id: process_info.dw_thread_id,
    })
}

/// Build a Windows environment block for [`spawn_suspended`]'s
/// `environment` parameter: each `("NAME", "value")` pair encoded as a
/// NUL-terminated UTF-16 `"NAME=value"` string, back to back, with one
/// additional NUL terminating the whole block — the exact format
/// `CreateProcessW` requires when `CREATE_UNICODE_ENVIRONMENT` is set.
///
/// `vars` order is preserved as given; callers with a name appearing more
/// than once should dedupe first (`CreateProcessW`'s own behavior on a
/// block with a duplicate name is unspecified by its docs).
pub fn environment_block<'a>(vars: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    for (name, value) in vars {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        // Zero variables: still terminate with two NULs total, not one —
        // documented Win32 behavior for an empty environment block.
        block.push(0);
    }
    block
}

/// Snapshot the calling process's real environment as
/// `(name, value)` pairs — `GetEnvironmentStringsW`, the read-back
/// counterpart to [`environment_block`]. Exists for a caller that needs to
/// *seed* its own variable table from the real inherited environment at
/// startup (unlike `spawn_suspended`'s `environment` parameter, which only
/// ever *writes* a block for a child) — a caller tracking its own table
/// separately from the OS environment (as `rush`'s `vars` module does)
/// needs exactly this once, before it starts tracking exports/unsets
/// itself.
///
/// Includes Windows' own `=C:=C:\path`-style hidden per-drive
/// current-directory entries (name `=C:`, `=D:`, ...) exactly as
/// `GetEnvironmentStringsW` reports them, rather than this wrapper
/// silently filtering them — deciding whether a caller's variable table
/// should carry these is the caller's policy, the same way this crate
/// exposes raw `FILE_ATTRIBUTE_*`/`ENABLE_*` bits without deciding what
/// they mean.
pub fn environment_snapshot()
-> Result<Vec<(alloc::string::String, alloc::string::String)>, Win32Error> {
    // SAFETY: `GetEnvironmentStringsW` takes no arguments; a `NULL` return
    // is its own documented (if practically unreachable) failure mode,
    // handled below rather than assumed away.
    let ptr = unsafe { GetEnvironmentStringsW() };
    if ptr.is_null() {
        return Err(Win32Error::last());
    }

    let mut pairs = Vec::new();
    let mut entry_start = 0usize;
    let mut i = 0usize;
    // SAFETY: `ptr` is the just-returned, valid pointer to a block
    // documented as a sequence of NUL-terminated UTF-16 strings ending in
    // one additional NUL; this walk reads only up through that terminator.
    unsafe {
        loop {
            if *ptr.add(i) == 0 {
                if i == entry_start {
                    // An empty entry marks the block's own end (the
                    // additional NUL after the last real "NAME=value").
                    break;
                }
                let entry = core::slice::from_raw_parts(ptr.add(entry_start), i - entry_start);
                if let Some(pair) = parse_environment_entry(entry) {
                    pairs.push(pair);
                }
                entry_start = i + 1;
            }
            i += 1;
        }
    }
    // SAFETY: `ptr` was returned by `GetEnvironmentStringsW` above and not
    // used again after this.
    unsafe { FreeEnvironmentStringsW(ptr) };
    Ok(pairs)
}

/// Split one `GetEnvironmentStringsW` entry (already known non-empty, no
/// terminating NUL included) into `(name, value)`. The search for `=`
/// deliberately skips index 0: Windows' own hidden `=C:=C:\path` per-drive
/// current-directory entries carry a leading `=` that's part of the name,
/// not a separator — the real, and only, separator is the *next* `=` after
/// that.
fn parse_environment_entry(
    units: &[u16],
) -> Option<(alloc::string::String, alloc::string::String)> {
    let equals_index = 1 + units.get(1..)?.iter().position(|&u| u == u16::from(b'='))?;
    Some((
        alloc::string::String::from_utf16_lossy(&units[..equals_index]),
        alloc::string::String::from_utf16_lossy(&units[equals_index + 1..]),
    ))
}

/// Read one live environment variable — `GetEnvironmentVariableW`, the
/// single-variable counterpart to [`environment_snapshot`]'s full-block
/// read. `Ok(None)` means the variable isn't set (`GetLastError() ==
/// ERROR_ENVIRONMENT_VARIABLE_NOT_FOUND`), matching this crate's existing
/// `search_path`-style "not found is not an error" convention rather than
/// folding it into the `Err` case.
pub fn get_env_var(name: &str) -> Result<Option<alloc::string::String>, Win32Error> {
    let wide_name = to_wide(name);
    let mut buf: Vec<u16> = alloc::vec![0u16; 256];
    // At most two attempts: an initial try, then one retry sized exactly to
    // whatever `GetEnvironmentVariableW` reports as actually required —
    // matching `path::current_dir`'s own growing-buffer pattern.
    for _ in 0..2 {
        // SAFETY: `wide_name` is a valid, NUL-terminated UTF-16 string;
        // `buf` is a valid, `buf.len()`-element writable buffer.
        let needed = unsafe {
            GetEnvironmentVariableW(wide_name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32)
        };
        if needed == 0 {
            let err = Win32Error::last();
            return match err {
                Win32Error::ERROR_ENVIRONMENT_VARIABLE_NOT_FOUND => Ok(None),
                // Documented quirk: a variable that's set but empty also
                // reports a 0 return, distinguished from "not found" only by
                // `GetLastError` reporting success rather than actually
                // failing.
                Win32Error::SUCCESS => Ok(Some(alloc::string::String::new())),
                err => Err(err),
            };
        }
        if (needed as usize) > buf.len() {
            buf.resize(needed as usize, 0);
            continue;
        }
        return Ok(Some(alloc::string::String::from_utf16_lossy(
            &buf[..needed as usize],
        )));
    }
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// Write, or with `value: None`, delete one live environment variable —
/// `SetEnvironmentVariableW`. Only affects the calling process's own
/// environment block (and anything it spawns afterward without an explicit
/// override); it has no effect on already-running processes or the
/// environment a `spawn_suspended` child sees if that call's own
/// `environment` argument overrides the block entirely.
pub fn set_env_var(name: &str, value: Option<&str>) -> Result<(), Win32Error> {
    let wide_name = to_wide(name);
    let wide_value = value.map(to_wide);
    let value_ptr = wide_value
        .as_ref()
        .map_or(core::ptr::null(), |v| v.as_ptr());
    // SAFETY: `wide_name` is a valid, NUL-terminated UTF-16 string;
    // `value_ptr` is either NULL (documented as "delete the variable") or a
    // valid, NUL-terminated UTF-16 string from `wide_value`, kept alive for
    // the duration of this call.
    let ok = unsafe { SetEnvironmentVariableW(wide_name.as_ptr(), value_ptr) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// One entry from [`list_processes`]'s system-wide snapshot — a `ps`-row's
/// worth of information about a single process. `exe_file` is the
/// executable's bare filename (e.g. `"cmd.exe"`), not a full path —
/// `PROCESSENTRY32W` itself doesn't carry one; resolving a pid to its full
/// path is a separate, unrelated primitive this crate doesn't currently
/// expose.
#[derive(Debug, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub thread_count: u32,
    pub exe_file: alloc::string::String,
}

/// List every process currently running on the system — a `ps`-equivalent,
/// via `CreateToolhelp32Snapshot`/`Process32FirstW`/`Process32NextW`. The
/// snapshot is a point-in-time copy: a process that exits mid-enumeration
/// still appears (it existed when the snapshot was taken), and one started
/// afterward doesn't.
pub fn list_processes() -> Result<Vec<ProcessEntry>, Win32Error> {
    // SAFETY: `TH32CS_SNAPPROCESS` is a documented-valid flag on its own;
    // `process_id = 0` is a plain value (ignored for a process-only
    // snapshot, not a pointer).
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return Err(Win32Error::last());
    }

    let mut entry = ProcessEntry32W {
        dw_size: core::mem::size_of::<ProcessEntry32W>() as u32,
        ..ProcessEntry32W::default()
    };
    let mut entries = Vec::new();
    // SAFETY: `snapshot` is freshly created and valid; `entry` is a valid,
    // correctly-sized (`dw_size` set, as both calls require) out-pointer.
    let mut found = unsafe { Process32FirstW(snapshot, &mut entry) };
    while found != 0 {
        entries.push(ProcessEntry {
            pid: entry.th32_process_id,
            parent_pid: entry.th32_parent_process_id,
            thread_count: entry.cnt_threads,
            exe_file: decode_exe_file(&entry.sz_exe_file),
        });
        // SAFETY: same as above — `snapshot`/`entry` are still the same
        // valid handle/out-pointer.
        found = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    // `Process32NextW`'s own documented end-of-enumeration signal is a
    // `FALSE` return with `GetLastError() == ERROR_NO_MORE_FILES` — not a
    // real failure, just "nothing left to report." Anything else reaching
    // here (including a `Process32FirstW` that failed on its very first
    // call, for a real reason) is a genuine error.
    let last_error = Win32Error::last();
    // SAFETY: `snapshot` is a valid, currently-open handle, closed exactly
    // once and not used again after this.
    let _ = unsafe { crate::handle::close(snapshot) };
    if last_error != Win32Error::ERROR_NO_MORE_FILES {
        return Err(last_error);
    }
    Ok(entries)
}

/// Decode a `PROCESSENTRY32W::szExeFile`-shaped fixed buffer up to its first
/// NUL (or the whole buffer, if unterminated — not expected in practice,
/// but not assumed away either).
fn decode_exe_file(units: &[u16; 260]) -> alloc::string::String {
    let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    alloc::string::String::from_utf16_lossy(&units[..len])
}

/// List the thread ids belonging to process `pid` — `CreateToolhelp32Snapshot`
/// (`TH32CS_SNAPTHREAD`)/`Thread32First`/`Thread32Next`, filtered by
/// `th32OwnerProcessID` after the fact since the snapshot itself always
/// covers every thread on the system, not just one process's. The missing
/// "pause the whole process" primitive Windows has no direct equivalent
/// for — the closest is suspending every one of a process's threads
/// individually via [`open_thread`]/[`suspend_thread`], the `SIGSTOP`
/// analog a future `bg`/`fg`/Ctrl-Z-style feature would need (currently
/// out of `rush`'s scope, per its own `docs/WINDOWS_JOB_CONTROL.md`).
pub fn list_threads(pid: u32) -> Result<Vec<u32>, Win32Error> {
    // SAFETY: `TH32CS_SNAPTHREAD` is a documented-valid flag on its own;
    // `process_id = 0` is required for a thread snapshot (the process-id
    // filter parameter is documented as ignored for this snapshot type).
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot.is_null() || snapshot as isize == -1 {
        return Err(Win32Error::last());
    }

    let mut entry = ThreadEntry32 {
        dw_size: core::mem::size_of::<ThreadEntry32>() as u32,
        ..ThreadEntry32::default()
    };
    let mut thread_ids = Vec::new();
    // SAFETY: `snapshot` is freshly created and valid; `entry` is a valid,
    // correctly-sized (`dw_size` set, as both calls require) out-pointer.
    let mut found = unsafe { Thread32First(snapshot, &mut entry) };
    while found != 0 {
        if entry.th32_owner_process_id == pid {
            thread_ids.push(entry.th32_thread_id);
        }
        // SAFETY: same as above — `snapshot`/`entry` are still the same
        // valid handle/out-pointer.
        found = unsafe { Thread32Next(snapshot, &mut entry) };
    }
    // Same "FALSE + ERROR_NO_MORE_FILES means ordinary end-of-enumeration"
    // convention `list_processes` relies on for `Process32NextW`.
    let last_error = Win32Error::last();
    // SAFETY: `snapshot` is a valid, currently-open handle, closed exactly
    // once and not used again after this.
    let _ = unsafe { crate::handle::close(snapshot) };
    if last_error != Win32Error::ERROR_NO_MORE_FILES {
        return Err(last_error);
    }
    Ok(thread_ids)
}

/// Open a handle to the thread named by `thread_id` — `OpenThread`, the
/// thread-level counterpart to [`open_by_pid`]. Needed to turn one of
/// [`list_threads`]'s thread ids into a handle [`suspend_thread`]/[`resume`]
/// can act on.
pub fn open_thread(thread_id: u32, desired_access: u32) -> Result<RawHandle, Win32Error> {
    // SAFETY: `desired_access` is a plain access-rights bitmask, not a
    // pointer; `inherit_handle = FALSE` (0) is a documented valid input;
    // `thread_id` is caller-supplied and `OpenThread` itself reports an
    // unknown or inaccessible one as an ordinary `NULL`/`GetLastError`
    // failure, not undefined behavior.
    let handle = unsafe { OpenThread(desired_access, 0, thread_id) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Suspend `thread` — `SuspendThread`, the Windows analog of `SIGSTOP` at
/// the individual-thread level (Windows has no process-wide stop
/// primitive; pausing every thread [`list_threads`] reports for a process
/// is the closest equivalent). Pair with [`resume`] to continue it again —
/// the `SIGCONT` half already wrapped for `spawn_suspended`'s own use.
/// Returns the thread's previous suspend count.
///
/// # Safety
///
/// `thread` must be a currently-open, valid thread handle with the
/// [`THREAD_SUSPEND_RESUME`] access right.
pub unsafe fn suspend_thread(thread: RawHandle) -> Result<u32, Win32Error> {
    // SAFETY: `thread` is caller-supplied per this function's own safety
    // contract; `SuspendThread` has no further precondition.
    let previous_suspend_count = unsafe { SuspendThread(thread) };
    if previous_suspend_count == u32::MAX {
        Err(Win32Error::last())
    } else {
        Ok(previous_suspend_count)
    }
}

/// Resume a thread suspended by [`spawn_suspended`] (or any other
/// `CREATE_SUSPENDED`-started thread this crate hands back a handle to).
///
/// # Safety
///
/// `thread` must be a currently-open, valid thread handle.
pub unsafe fn resume(thread: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `thread` is caller-supplied per this function's own safety
    // contract; `ResumeThread` has no further precondition.
    let previous_suspend_count = unsafe { ResumeThread(thread) };
    if previous_suspend_count == u32::MAX {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Block on `process` for up to `timeout_ms` (`None` = wait forever, the
/// Windows analog of a blocking Unix `waitpid`). Returns `Some(exit_code)`
/// if the process exited within the timeout, `None` on timeout — call with
/// `Some(0)` for a non-blocking poll (the Windows analog of
/// `waitpid(WNOHANG)`).
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn wait(process: RawHandle, timeout_ms: Option<u32>) -> Result<Option<u32>, Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract.
    let result = unsafe { WaitForSingleObject(process, timeout_ms.unwrap_or(INFINITE)) };
    match result {
        WAIT_OBJECT_0 => {
            let mut code: u32 = 0;
            // SAFETY: `process` is the same valid handle just waited on;
            // `code` is a valid out-pointer.
            let ok = unsafe { GetExitCodeProcess(process, &mut code) };
            if ok == 0 {
                Err(Win32Error::last())
            } else {
                Ok(Some(code))
            }
        }
        WAIT_TIMEOUT => Ok(None),
        _ => Err(Win32Error::last()),
    }
}

/// Block on whichever of `processes` exits *first*, for up to `timeout_ms`
/// (`None` = wait forever) — the Windows analog of a blocking Unix
/// `waitpid(-1, ...)` restricted to a known set of pids, and the multi-handle
/// counterpart to [`wait`]. Returns `Some((index, exit_code))` naming the
/// first-signaled handle's position in `processes` and its exit code, or
/// `None` on timeout; call with `Some(0)` for a non-blocking poll across the
/// whole set at once, rather than looping [`wait`] over each handle in turn.
///
/// `processes` must be non-empty and no longer than
/// [`MAXIMUM_WAIT_OBJECTS`] — `WaitForMultipleObjects`'s own documented
/// limit on how many handles a single call accepts; passing more (or zero)
/// reports [`Win32Error::ERROR_INVALID_PARAMETER`], the same failure the raw
/// call itself would report, not a distinct error this crate invents.
///
/// If more than one handle is already signaled at the moment of the call,
/// which index comes back is `WaitForMultipleObjects`'s own documented
/// choice (the lowest signaled index), not something this wrapper adds
/// logic for.
///
/// # Safety
///
/// Every handle in `processes` must be currently-open and valid.
pub unsafe fn wait_any(
    processes: &[RawHandle],
    timeout_ms: Option<u32>,
) -> Result<Option<(usize, u32)>, Win32Error> {
    // SAFETY: `processes` is a caller-supplied slice of valid handles per
    // this function's own safety contract; `processes.as_ptr()`/`.len()`
    // describe that same slice, a valid input `WaitForMultipleObjects`
    // documents (including reporting `ERROR_INVALID_PARAMETER` itself for
    // an empty or oversized one, rather than this wrapper pre-checking).
    let result = unsafe {
        WaitForMultipleObjects(
            processes.len() as u32,
            processes.as_ptr(),
            0,
            timeout_ms.unwrap_or(INFINITE),
        )
    };
    const WAIT_FAILED: u32 = 0xFFFF_FFFF;
    match result {
        WAIT_TIMEOUT => Ok(None),
        WAIT_FAILED => Err(Win32Error::last()),
        index if (index as usize) < processes.len() => {
            let process = processes[index as usize];
            let mut code: u32 = 0;
            // SAFETY: `process` is the same valid handle just signaled;
            // `code` is a valid out-pointer.
            let ok = unsafe { GetExitCodeProcess(process, &mut code) };
            if ok == 0 {
                Err(Win32Error::last())
            } else {
                Ok(Some((index as usize, code)))
            }
        }
        // A signaled-index return outside `0..processes.len()` only
        // happens for the abandoned-mutex range (`WAIT_ABANDONED_0..`),
        // which can't occur for process handles — process objects are
        // never abandoned the way a mutex is. Reported as the raw code
        // rather than silently treated as a timeout or a panic.
        other => Err(Win32Error::from_raw(other)),
    }
}

/// The calling process's own pid — the Windows analog of Unix `getpid`.
pub fn current_pid() -> u32 {
    // SAFETY: `GetCurrentProcessId` takes no arguments and has no
    // preconditions.
    unsafe { GetCurrentProcessId() }
}

/// Open a handle to the process named by `pid` — `OpenProcess`, needed for
/// `kill <pid>` on a pid a caller only knows numerically (e.g. read back
/// from `jobs`/`$!`), not one of this crate's own [`SpawnedProcess`] handles
/// from [`spawn_suspended`]. `desired_access` should be the narrowest set of
/// rights the caller actually needs — [`PROCESS_TERMINATE`] alone for a
/// handle that will only ever be passed to [`terminate`], or
/// `PROCESS_TERMINATE | SYNCHRONIZE` for one that will also be passed to
/// [`wait`]/[`wait_any`].
pub fn open_by_pid(pid: u32, desired_access: u32) -> Result<RawHandle, Win32Error> {
    // SAFETY: `desired_access` is a plain access-rights bitmask, not a
    // pointer; `inherit_handle = FALSE` (0) is a documented valid input;
    // `pid` is caller-supplied and `OpenProcess` itself reports an unknown
    // or inaccessible one as an ordinary `NULL`/`GetLastError` failure, not
    // undefined behavior.
    let handle = unsafe { OpenProcess(desired_access, 0, pid) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// The numeric pid a process handle refers to — `GetProcessId`, the reverse
/// of [`open_by_pid`]'s pid-to-`HANDLE` mapping. Needed anywhere a caller
/// holds a `HANDLE` (e.g. `spawn_suspended`'s own `process` handle) and
/// needs to report/print its numeric pid without having cached it
/// separately.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle with the
/// `PROCESS_QUERY_LIMITED_INFORMATION` access right (or better).
pub unsafe fn process_id_of(process: RawHandle) -> Result<u32, Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `GetProcessId` reports a failing handle as an ordinary
    // `0`/`GetLastError` failure, not undefined behavior.
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 {
        Err(Win32Error::last())
    } else {
        Ok(pid)
    }
}

/// Terminate `process` with `exit_code` — `TerminateProcess`, the
/// single-process counterpart to [`crate::job::terminate`] (which kills
/// every process in a job at once). Needed for `kill <pid>` against a
/// process this crate didn't itself spawn into a job — one opened via
/// [`open_by_pid`] instead.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle with the
/// `PROCESS_TERMINATE` access right.
pub unsafe fn terminate(process: RawHandle, exit_code: u32) -> Result<(), Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `exit_code` is a plain value, not a pointer.
    let ok = unsafe { TerminateProcess(process, exit_code) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// CPU-time accounting for `process` — `GetProcessTimes`. `process` needs
/// only [`PROCESS_QUERY_LIMITED_INFORMATION`] (the narrowest right this
/// call requires), not [`PROCESS_TERMINATE`]/[`SYNCHRONIZE`] — a handle
/// obtained purely to report timing doesn't need rights that let it affect
/// the process at all.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn times(process: RawHandle) -> Result<ProcessTimes, Win32Error> {
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; all four out-pointers are valid, correctly-sized locals.
    let ok = unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(ProcessTimes {
        creation: filetime_to_timespec(creation),
        exit: filetime_to_timespec(exit),
        kernel_time: filetime_to_duration(kernel),
        user_time: filetime_to_duration(user),
    })
}

/// The full executable path for `process` — `QueryFullProcessImageNameW`,
/// completing [`list_processes`]'s `ProcessEntry::exe_file` (issue #21,
/// `PROCESSENTRY32W.szExeFile`), which is only ever a bare filename, not a
/// full path. Like [`times`], `process` needs only
/// [`PROCESS_QUERY_LIMITED_INFORMATION`].
///
/// Unlike this crate's other growing-buffer calls, `QueryFullProcessImageNameW`
/// doesn't report the size actually required on a "buffer too small"
/// failure (`lpdwSize` is documented as unchanged in that case) — so this
/// doubles the buffer and retries rather than growing to an exact reported
/// size, up to a generous cap matching Windows' own long-path maximum.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn image_path(process: RawHandle) -> Result<alloc::string::String, Win32Error> {
    const MAX_ATTEMPTS: u32 = 8;
    let mut buf: Vec<u16> = alloc::vec![0u16; 260];
    for _ in 0..MAX_ATTEMPTS {
        let mut size = buf.len() as u32;
        // SAFETY: `process` is caller-supplied per this function's own
        // safety contract; `buf` is a valid, `buf.len()`-element writable
        // buffer; `size` is a valid in/out pointer set to that same length;
        // `flags = 0` requests the Win32 path format (not the NT native
        // form), a documented valid input.
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) };
        if ok != 0 {
            // On success, `size` is updated to the actual length written,
            // excluding the terminating NUL.
            return Ok(alloc::string::String::from_utf16_lossy(
                &buf[..size as usize],
            ));
        }
        let err = Win32Error::last();
        if err != Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
        buf.resize(buf.len() * 2, 0);
    }
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// Block the calling thread for `milliseconds` — `Sleep`, the direct
/// primitive behind a `sleep`/`usleep` builtin. No `Result`: `Sleep` has no
/// documented failure mode to report, matching this crate's already-
/// established "never fails" pattern for e.g. `GetDriveTypeW`.
pub fn sleep_ms(milliseconds: u32) {
    // SAFETY: `Sleep` has no precondition beyond a plain millisecond count.
    unsafe { Sleep(milliseconds) }
}

/// Alertable-sleep variant of [`sleep_ms`] — `SleepEx`. With `alertable:
/// true`, an APC queued to this thread wakes the sleep early, reported as
/// `WAIT_IO_COMPLETION` (`192`) rather than `0` (the full duration
/// elapsed); with `alertable: false` this behaves identically to
/// `sleep_ms`. No `Result`: like `Sleep`, `SleepEx` has no documented
/// failure mode to report. No current `rush` feature uses APCs, so
/// `alertable: true` has no realistic use yet — filed for Win32 parity.
pub fn sleep_ms_ex(milliseconds: u32, alertable: bool) -> u32 {
    // SAFETY: `SleepEx` has no precondition beyond a plain millisecond
    // count and a plain boolean flag.
    unsafe { SleepEx(milliseconds, i32::from(alertable)) }
}

/// The number of logical processors visible to the calling process —
/// `GetSystemInfo`'s `dwNumberOfProcessors`, the primitive behind an
/// `nproc`-equivalent builtin. No `Result`: `GetSystemInfo` has no
/// documented failure mode, matching this crate's already-established
/// "never fails" pattern (e.g. `GetDriveTypeW`).
pub fn logical_processor_count() -> u32 {
    let mut info = SystemInfo::default();
    // SAFETY: `info` is a valid, correctly-sized out-pointer; `GetSystemInfo`
    // has no other precondition.
    unsafe { GetSystemInfo(&mut info) };
    info.number_of_processors
}

/// Milliseconds elapsed since the system started — `GetTickCount64`, a
/// coarser, simpler monotonic counter alongside
/// [`crate::time::now_monotonic`]'s `QueryPerformanceCounter`-backed high
/// resolution one; some callers may still prefer this for its trivial
/// units. No `Result`: `GetTickCount64` has no documented failure mode,
/// matching this crate's already-established "never fails" pattern (e.g.
/// `GetDriveTypeW`). No current `rush` feature asks for this; filed for
/// Win32 parity.
pub fn tick_count() -> u64 {
    // SAFETY: `GetTickCount64` takes no arguments and has no precondition.
    unsafe { GetTickCount64() }
}

/// `SYSTEM_LOGICAL_PROCESSOR_INFORMATION`'s `Relationship` field — what
/// kind of CPU topology relationship a given
/// [`LogicalProcessorInformation`] entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorRelationship {
    /// The entry describes a single physical processor core (and its set
    /// of logical processors, e.g. via hyperthreading).
    ProcessorCore,
    /// The entry describes a NUMA node.
    NumaNode,
    /// The entry describes a cache (L1/L2/L3) shared by the processors in
    /// its mask.
    Cache,
    /// The entry describes a physical processor package (a socket).
    ProcessorPackage,
    /// A relationship kind this crate doesn't otherwise name, carrying the
    /// raw `Relationship` value Windows reported.
    Unknown(u32),
}

impl ProcessorRelationship {
    fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::ProcessorCore,
            1 => Self::NumaNode,
            2 => Self::Cache,
            3 => Self::ProcessorPackage,
            other => Self::Unknown(other),
        }
    }
}

/// One [`logical_processor_information`] entry — a set of logical
/// processors (`processor_mask`, the same per-bit-a-CPU shape as
/// `SystemInfo`'s `active_processor_mask`) sharing a given topology
/// relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalProcessorInformation {
    /// Which logical processors (bit N = CPU N) this entry covers.
    pub processor_mask: usize,
    /// What kind of relationship this entry describes.
    pub relationship: ProcessorRelationship,
}

/// Detailed CPU topology (cores/logical processors/NUMA nodes/cache
/// layout) — `GetLogicalProcessorInformation`, going beyond
/// [`logical_processor_count`]'s single number. Cache- and NUMA-specific
/// fields (cache size/line size/associativity, NUMA node number) aren't
/// exposed — only `processor_mask`/`relationship`, meaningful across
/// every relationship kind; a future consumer needing those can extend
/// this. No current `rush` feature asks for this; filed for Win32 parity.
pub fn logical_processor_information() -> Result<Vec<LogicalProcessorInformation>, Win32Error> {
    let mut return_length: u32 = 0;
    // SAFETY: a NULL buffer with `return_length = 0` is the documented way
    // to query the required buffer size; the real answer comes back via
    // `return_length` on the expected `ERROR_INSUFFICIENT_BUFFER` failure.
    let ok = unsafe { GetLogicalProcessorInformation(core::ptr::null_mut(), &mut return_length) };
    if ok != 0 {
        // A real machine always has at least one entry; a `0`-size
        // success is surprising but not a failure this wrapper invents —
        // report it as an empty result rather than erroring.
        return Ok(Vec::new());
    }
    let err = Win32Error::last();
    if err != Win32Error::ERROR_INSUFFICIENT_BUFFER {
        return Err(err);
    }

    let count =
        return_length as usize / core::mem::size_of::<SystemLogicalProcessorInformationRaw>();
    let mut buf = alloc::vec![SystemLogicalProcessorInformationRaw::default(); count];
    let mut actual_length = return_length;
    // SAFETY: `buf` is a valid, correctly-sized (per the size this same
    // call just reported) writable buffer of
    // `SystemLogicalProcessorInformationRaw`; `actual_length` is a valid
    // in/out pointer set to that buffer's byte length.
    let ok = unsafe { GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut actual_length) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(buf
        .iter()
        .map(|raw| LogicalProcessorInformation {
            processor_mask: raw.processor_mask,
            relationship: ProcessorRelationship::from_raw(raw.relationship),
        })
        .collect())
}

/// `PEXCEPTION_POINTERS` — the argument passed to a
/// [`VectoredExceptionHandler`]/[`TopLevelExceptionFilter`], carrying raw
/// pointers to the exception record and CPU register context. Neither
/// pointee is decoded by this crate (`EXCEPTION_RECORD` has a
/// variable-length trailing argument array; `CONTEXT` is a large,
/// architecture-specific register dump) — a handler that needs them reads
/// through the raw pointers itself.
#[repr(C)]
pub struct ExceptionPointers {
    pub exception_record: *mut core::ffi::c_void,
    pub context_record: *mut core::ffi::c_void,
}

/// A vectored exception handler — see [`add_vectored_exception_handler`].
pub type VectoredExceptionHandler =
    extern "system" fn(exception_info: *mut ExceptionPointers) -> i32;

/// A process's top-level (unhandled-exception) filter — see
/// [`set_unhandled_exception_filter`].
pub type TopLevelExceptionFilter =
    extern "system" fn(exception_info: *mut ExceptionPointers) -> i32;

/// A [`VectoredExceptionHandler`]/[`TopLevelExceptionFilter`] return value:
/// resume execution at the point the exception occurred — only valid if
/// the handler actually fixed the condition that faulted.
pub const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
/// Pass the exception to the next handler in the chain (or, for a
/// top-level filter, to Windows' own default handling).
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
/// (Top-level filter only.) Let Windows execute its default
/// unhandled-exception handling — typically terminating the process,
/// after this filter itself has already run (e.g. to log the crash).
pub const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

/// Register `handler` on the process-wide vectored-exception-handler
/// chain — `AddVectoredExceptionHandler`, the closest Windows analog to
/// installing a Unix `SIGSEGV`/`SIGABRT` handler. Runs for *every*
/// exception in the process, before structured exception handling
/// (`__try`/`__except`, which this crate has no Rust equivalent of) gets a
/// chance. `first` requests this handler run before any already-installed
/// ones (`true`) or after (`false`). Returns an opaque handle for
/// [`remove_vectored_exception_handler`] — not the same as `handler`
/// itself, since Windows doesn't guarantee pointer-identity removal here
/// the way `SetConsoleCtrlHandler` does. No current `rush` feature asks
/// for this; filed for Win32 parity.
pub fn add_vectored_exception_handler(
    first: bool,
    handler: VectoredExceptionHandler,
) -> Result<*mut core::ffi::c_void, Win32Error> {
    // SAFETY: `handler` is a `'static` function pointer of the exact
    // signature `AddVectoredExceptionHandler` requires; `first` is a plain
    // flag, not a pointer.
    let handle = unsafe { AddVectoredExceptionHandler(u32::from(first), handler) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Unregister a handler previously added by
/// [`add_vectored_exception_handler`], by the handle it returned —
/// `RemoveVectoredExceptionHandler`.
///
/// # Safety
///
/// `handle` must be a value [`add_vectored_exception_handler`] returned,
/// not yet removed.
pub unsafe fn remove_vectored_exception_handler(
    handle: *mut core::ffi::c_void,
) -> Result<(), Win32Error> {
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract, expected to be a value this module's own
    // `add_vectored_exception_handler` returned.
    let ok = unsafe { RemoveVectoredExceptionHandler(handle) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Install `filter` as the process's top-level (unhandled-exception)
/// filter — `SetUnhandledExceptionFilter`, the last resort before
/// Windows' own default crash handling runs. Pass `None` to restore the
/// default handling. Returns whatever filter was previously installed
/// (`None` if none was). No current `rush` feature asks for this; filed
/// for Win32 parity.
pub fn set_unhandled_exception_filter(
    filter: Option<TopLevelExceptionFilter>,
) -> Option<TopLevelExceptionFilter> {
    // SAFETY: `filter` (if any) is a `'static` function pointer of the
    // exact signature `SetUnhandledExceptionFilter` requires.
    unsafe { SetUnhandledExceptionFilter(filter) }
}

/// This machine's NetBIOS computer name — `GetComputerNameW`, the primitive
/// behind `$HOSTNAME`, a shell prompt, or a `hostname` builtin.
pub fn computer_name() -> Result<alloc::string::String, Win32Error> {
    let mut buf: Vec<u16> = alloc::vec![0u16; 256];
    // At most two attempts: an initial try, then one retry sized exactly to
    // whatever `GetComputerNameW` reports as actually required —
    // `ERROR_BUFFER_OVERFLOW`'s documented failure mode updates `size`
    // in-place to the exact required length (including the terminating
    // NUL), unlike this crate's other growing-buffer calls, which only
    // ever report a lower bound.
    for _ in 0..2 {
        let mut size = buf.len() as u32;
        // SAFETY: `buf` is a valid, `buf.len()`-element writable buffer;
        // `size` is a valid in/out pointer set to that same length.
        let ok = unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut size) };
        if ok != 0 {
            return Ok(alloc::string::String::from_utf16_lossy(
                &buf[..size as usize],
            ));
        }
        let err = Win32Error::last();
        if err != Win32Error::ERROR_BUFFER_OVERFLOW {
            return Err(err);
        }
        buf.resize(size as usize, 0);
    }
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// System-wide memory totals and load — `GlobalMemoryStatusEx`, the
/// primitive behind a `free`-style builtin or general resource reporting.
/// Omits the raw struct's `ullAvailExtendedVirtual` field: it's only
/// meaningful for Address Windowing Extensions memory, out of this crate's
/// current scope, and documented as always `0` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryStatus {
    /// Approximate percentage of physical memory currently in use (`0..=100`).
    pub memory_load: u32,
    /// Total physical memory, in bytes.
    pub total_phys: u64,
    /// Available physical memory, in bytes.
    pub avail_phys: u64,
    /// Total size of the page (swap) file(s), in bytes.
    pub total_page_file: u64,
    /// Available size of the page (swap) file(s), in bytes.
    pub avail_page_file: u64,
    /// Total size of the calling process's user-mode virtual address space,
    /// in bytes.
    pub total_virtual: u64,
    /// Available (unreserved, uncommitted) size of the calling process's
    /// user-mode virtual address space, in bytes.
    pub avail_virtual: u64,
}

/// System-wide memory totals and load — `GlobalMemoryStatusEx`.
pub fn memory_status() -> Result<MemoryStatus, Win32Error> {
    let mut info = MemoryStatusEx {
        length: core::mem::size_of::<MemoryStatusEx>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid, correctly-sized out-pointer with `length`
    // set to `sizeof(MEMORYSTATUSEX)` per this call's own documented
    // requirement.
    let ok = unsafe { GlobalMemoryStatusEx(&mut info) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(MemoryStatus {
        memory_load: info.memory_load,
        total_phys: info.total_phys,
        avail_phys: info.avail_phys,
        total_page_file: info.total_page_file,
        avail_page_file: info.avail_page_file,
        total_virtual: info.total_virtual,
        avail_virtual: info.avail_virtual,
    })
}

/// `SetErrorMode`'s bit for suppressing the blocking GUI dialog a hardware
/// or media error (e.g. an empty removable drive, a network path that's
/// gone away) would otherwise pop up — the single most important bit for a
/// non-interactive script run, since without it such an error freezes the
/// whole process waiting for a click that will never come.
pub const SEM_FAILCRITICALERRORS: u32 = 0x0001;
/// `SetErrorMode`'s bit for suppressing the "file not found"-style dialog
/// `OpenFile` would otherwise show.
pub const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;

/// Set the calling process's error mode (the `SEM_*` bits above), returning
/// the previous mode — `SetErrorMode`. No `Result`: `SetErrorMode` has no
/// documented failure mode, matching this crate's already-established
/// "never fails" pattern (e.g. `GetDriveTypeW`, `sleep_ms`). Applies
/// process-wide and is inherited by child processes, so calling this once
/// early in a shell's own startup (before spawning anything) is enough to
/// cover the whole process tree.
pub fn set_error_mode(mode: u32) -> u32 {
    // SAFETY: `SetErrorMode` has no precondition beyond a plain bitmask.
    unsafe { SetErrorMode(mode) }
}

/// `GetPriorityClass`/`SetPriorityClass`'s scheduling priority class:
/// below the system default — the Windows analog of a positive `nice`
/// value.
pub const IDLE_PRIORITY_CLASS: u32 = 0x0000_0040;
/// Slightly below the system default.
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
/// The system default scheduling priority — the Windows analog of `nice`'s
/// default (`0`).
pub const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
/// Slightly above the system default.
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
/// Above the system default — the Windows analog of a negative `nice`
/// value.
pub const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
/// The highest possible priority; can starve system threads and
/// destabilize the machine if misused, the same caution Unix's own
/// highest-priority `nice` values carry.
pub const REALTIME_PRIORITY_CLASS: u32 = 0x0000_0100;

/// `process`'s scheduling priority class (one of the `*_PRIORITY_CLASS`
/// constants above) — `GetPriorityClass`, the Windows analog of `nice`'s
/// read side.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn priority_class(process: RawHandle) -> Result<u32, Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `GetPriorityClass` reports a failing handle as an ordinary
    // `0`/`GetLastError` failure, not undefined behavior.
    let class = unsafe { GetPriorityClass(process) };
    if class == 0 {
        Err(Win32Error::last())
    } else {
        Ok(class)
    }
}

/// Set `process`'s scheduling priority class — `SetPriorityClass`, the
/// Windows analog of `renice`. `priority_class` is the raw
/// `*_PRIORITY_CLASS` bitmask above — this function is a thin, policy-free
/// wrapper, the same as this crate's other raw bitmask parameters. No
/// current `rush` feature asks for this, but it's the natural primitive if
/// a `nice`/`renice`-style feature is ever added.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn set_priority_class(
    process: RawHandle,
    priority_class: u32,
) -> Result<(), Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `priority_class` is a plain value, not a pointer.
    let ok = unsafe { SetPriorityClass(process, priority_class) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// `process`'s CPU affinity mask (which CPUs it may run on) plus the
/// system's own mask of CPUs available at all — `GetProcessAffinityMask`,
/// the Windows analog of `sched_getaffinity`/`taskset`'s read side. No
/// current `rush` feature asks for this; filed for Win32 parity.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn affinity(process: RawHandle) -> Result<(usize, usize), Win32Error> {
    let mut process_mask: usize = 0;
    let mut system_mask: usize = 0;
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; the two out-pointers are valid, distinct local variables.
    let ok = unsafe { GetProcessAffinityMask(process, &mut process_mask, &mut system_mask) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok((process_mask, system_mask))
    }
}

/// Set `process`'s CPU affinity mask — `SetProcessAffinityMask`, the
/// Windows analog of `taskset`/`sched_setaffinity`. `mask` must be a subset
/// of the system affinity mask `affinity` reports, or this fails with
/// `ERROR_INVALID_PARAMETER`.
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle.
pub unsafe fn set_affinity(process: RawHandle, mask: usize) -> Result<(), Win32Error> {
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `mask` is a plain value, not a pointer.
    let ok = unsafe { SetProcessAffinityMask(process, mask) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// `thread`'s exit code — `GetExitCodeThread`, the thread-level counterpart
/// to [`wait`]'s process exit code. Unlike `wait`, this does not block: if
/// `thread` hasn't exited yet, the returned code is `259`
/// (`STILL_ACTIVE`), Windows' own documented sentinel for "not done" rather
/// than a distinct error this wrapper invents — callers that need to block
/// until the thread exits should wait on the handle first (e.g.
/// `handle::wait_single`-style), then call this. No current `rush` feature
/// asks for this; filed for Win32 parity.
///
/// # Safety
///
/// `thread` must be a currently-open, valid thread handle.
pub unsafe fn thread_exit_code(thread: RawHandle) -> Result<u32, Win32Error> {
    let mut code: u32 = 0;
    // SAFETY: `thread` is caller-supplied per this function's own safety
    // contract; `code` is a valid, distinct local out-pointer.
    let ok = unsafe { GetExitCodeThread(thread, &mut code) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(code)
    }
}

/// CPU-time accounting for `thread` — `GetThreadTimes`, the thread-level
/// counterpart to [`times`]. `thread` needs
/// [`THREAD_QUERY_INFORMATION`] (the narrowest right this call requires).
/// No current `rush` feature asks for this; filed for Win32 parity.
///
/// # Safety
///
/// `thread` must be a currently-open, valid thread handle.
pub unsafe fn thread_times(thread: RawHandle) -> Result<ThreadTimes, Win32Error> {
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: `thread` is caller-supplied per this function's own safety
    // contract; all four out-pointers are valid, correctly-sized locals.
    let ok = unsafe { GetThreadTimes(thread, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(ThreadTimes {
        creation: filetime_to_timespec(creation),
        exit: filetime_to_timespec(exit),
        kernel_time: filetime_to_duration(kernel),
        user_time: filetime_to_duration(user),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_suspended_resume_wait_round_trip() {
        // The handoff doc's own suggested first proof: spawn a real
        // command, confirm the exit code round-trips.
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 7", false, false, None) }
            .expect("CreateProcessW should succeed");
        assert_ne!(spawned.process_id, 0);
        assert_ne!(spawned.thread_id, 0);

        // Still suspended: a zero-timeout wait must time out, not report
        // an exit — proves CREATE_SUSPENDED actually held the thread.
        // SAFETY: `spawned.process` is a freshly created, valid handle.
        let still_suspended = unsafe { wait(spawned.process, Some(0)) }.unwrap();
        assert_eq!(still_suspended, None);

        // SAFETY: `spawned.thread` is a freshly created, valid,
        // not-yet-resumed thread handle.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let exit_code = unsafe { wait(spawned.process, None) }.unwrap();
        assert_eq!(exit_code, Some(7));

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn spawn_suspended_with_pseudoconsole_resume_wait_round_trip() {
        let (input_read, input_write) = crate::handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's input pipe");
        let (output_read, output_write) = crate::handle::create_pipe()
            .expect("create_pipe should succeed for the pseudoconsole's output pipe");
        let size = crate::console::Coord { x: 80, y: 25 };
        // SAFETY: `input_read`/`output_write` are freshly created, open
        // pipe handles from the calls above.
        let hpc = unsafe { crate::conpty::create(input_read, output_write, size) }
            .expect("CreatePseudoConsole should succeed with a fresh pipe pair");
        // SAFETY: per `conpty::create`'s documented pattern, close this
        // test's own copies now that ConPTY has duplicated them
        // internally.
        unsafe { crate::handle::close(input_read) }
            .expect("closing this test's own copy of the input pipe's read end should succeed");
        unsafe { crate::handle::close(output_write) }
            .expect("closing this test's own copy of the output pipe's write end should succeed");

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary; `hpc` is a live, valid pseudoconsole
        // handle from `conpty::create` above.
        let spawned = unsafe { spawn_suspended_with_pseudoconsole("cmd.exe /c exit 7", hpc) }
            .expect("CreateProcessW should succeed with EXTENDED_STARTUPINFO_PRESENT");
        assert_ne!(spawned.process_id, 0);
        assert_ne!(spawned.thread_id, 0);

        // Still suspended: a zero-timeout wait must time out, not report
        // an exit -- proves CREATE_SUSPENDED actually held the thread.
        // SAFETY: `spawned.process` is a freshly created, valid handle.
        let still_suspended = unsafe { wait(spawned.process, Some(0)) }.unwrap();
        assert_eq!(still_suspended, None);

        // SAFETY: `spawned.thread` is a freshly created, valid,
        // not-yet-resumed thread handle.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let exit_code = unsafe { wait(spawned.process, None) }.unwrap();
        assert_eq!(exit_code, Some(7));

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }

        // SAFETY: `hpc` is still open from the calls above; the child
        // has already exited, matched by the `wait` call above.
        unsafe { crate::conpty::close(hpc) };
        unsafe { crate::handle::close(input_write) }
            .expect("closing the input pipe's write end should succeed");
        unsafe { crate::handle::close(output_read) }
            .expect("closing the output pipe's read end should succeed");
    }

    #[test]
    fn wait_any_reports_the_first_process_to_exit() {
        // Two processes, both started suspended; only one is ever resumed,
        // so which one `wait_any` reports is deterministic rather than a
        // race between two real running processes.
        // SAFETY: hand-built, correctly quoted command lines for a
        // well-known system binary.
        let a = unsafe { spawn_suspended("cmd.exe /c exit 3", false, false, None) }
            .expect("CreateProcessW should succeed");
        let b = unsafe { spawn_suspended("cmd.exe /c exit 9", false, false, None) }
            .expect("CreateProcessW should succeed");

        // SAFETY: `b.thread` is freshly created, valid, not yet resumed.
        unsafe { resume(b.thread) }.expect("ResumeThread should succeed");

        // SAFETY: both process handles are valid, currently-open, and
        // distinct.
        let (index, code) = unsafe { wait_any(&[a.process, b.process], None) }
            .unwrap()
            .expect("one process should have exited");
        assert_eq!(index, 1, "expected b (index 1), the only resumed process");
        assert_eq!(code, 9);

        // `a` was never resumed — resume it now so its own exit code (and
        // the still-suspended thread) don't leak into the test process
        // list, then clean up every handle.
        // SAFETY: `a.thread` is freshly created, valid, not yet resumed.
        unsafe { resume(a.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `a.process` is a valid, currently-open handle.
        unsafe { wait(a.process, None) }.unwrap();
        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(a.process).unwrap();
            crate::handle::close(a.thread).unwrap();
            crate::handle::close(b.process).unwrap();
            crate::handle::close(b.thread).unwrap();
        }
    }

    #[test]
    fn wait_any_times_out_when_nothing_has_exited() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // Still suspended, so a zero-timeout wait_any must time out.
        // SAFETY: `spawned.process` is a freshly created, valid handle.
        let timed_out = unsafe { wait_any(&[spawned.process], Some(0)) }.unwrap();
        assert_eq!(timed_out, None);

        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { wait(spawned.process, None) }.unwrap();
        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn wait_any_rejects_an_empty_slice() {
        // Matches `WaitForMultipleObjects`'s own documented behavior for
        // `nCount == 0` — this wrapper doesn't pre-validate and invent a
        // distinct error for the same condition.
        let err = unsafe { wait_any(&[], None) }.unwrap_err();
        assert_eq!(err, Win32Error::ERROR_INVALID_PARAMETER);
    }

    #[test]
    fn current_pid_is_nonzero() {
        assert_ne!(current_pid(), 0);
    }

    #[test]
    fn process_id_of_spawned_process_matches_its_own_reported_pid() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // SAFETY: `spawned.process` is a freshly created, valid handle.
        let pid = unsafe { process_id_of(spawned.process) }.expect("GetProcessId should succeed");
        assert_eq!(pid, spawned.process_id);

        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { wait(spawned.process, None) }.unwrap();

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn image_path_of_a_spawned_process_ends_with_its_own_exe_name() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // SAFETY: `spawned.process` is a freshly created, valid handle with
        // full access rights (including PROCESS_QUERY_LIMITED_INFORMATION),
        // since `CreateProcessW` itself opened it.
        let path = unsafe { image_path(spawned.process) }
            .expect("QueryFullProcessImageNameW should succeed");
        assert!(
            path.to_ascii_lowercase().ends_with("cmd.exe"),
            "got: {path}"
        );

        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { wait(spawned.process, None) }.unwrap();

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn open_by_pid_then_terminate_kills_a_process_known_only_by_pid() {
        // The scenario this pair exists for: a pid read back numerically
        // (e.g. from `jobs`/`$!`), with no `SpawnedProcess` handle in hand —
        // open a fresh handle from just the pid, terminate through *that*
        // handle, and confirm the original process handle still reports the
        // resulting exit code.
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known long-running system command.
        let spawned =
            unsafe { spawn_suspended("cmd.exe /c ping -n 30 127.0.0.1 >nul", false, false, None) }
                .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        let opened = open_by_pid(spawned.process_id, PROCESS_TERMINATE | SYNCHRONIZE)
            .expect("OpenProcess should succeed for a live pid this test itself just started");
        assert_ne!(
            opened, spawned.process,
            "OpenProcess should hand back an independent handle value, not the original one"
        );

        // SAFETY: `opened` is a freshly opened, valid handle with
        // PROCESS_TERMINATE.
        unsafe { terminate(opened, 42) }.expect("TerminateProcess should succeed");

        // SAFETY: `spawned.process` is still a valid handle — terminating
        // via a *different* handle to the same process doesn't invalidate
        // it, only the process itself.
        let exit = unsafe { wait(spawned.process, Some(5_000)) }.unwrap();
        assert_eq!(exit, Some(42));

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(opened).unwrap();
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn open_by_pid_fails_for_pid_zero() {
        // Pid 0 (the System Idle Process) is documented to never be
        // openable via `OpenProcess` — a stable, deterministic "this pid
        // does not resolve to an openable process" case to test against,
        // unlike an arbitrary made-up pid that could coincidentally collide
        // with something real on a given machine.
        assert!(open_by_pid(0, PROCESS_TERMINATE).is_err());
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AllocConsole() -> i32;
    }

    #[test]
    fn new_process_group_receives_a_targeted_ctrl_break() {
        // The scenario `new_process_group` exists for: start a child in its
        // own console process group, then interrupt *just* that group via
        // `console::generate_ctrl_event(CTRL_BREAK_EVENT, group_id)` —
        // Windows defines a new process group's id as its creating
        // process's own pid, so `spawned.process_id` doubles as the group
        // id here. `CTRL_C_EVENT` can't be scoped this way at all (Windows
        // only ever broadcasts it console-wide), which is exactly why this
        // primitive sends `CTRL_BREAK_EVENT` instead.
        //
        // SAFETY: `AllocConsole` has no precondition; a console already
        // being attached (the ordinary case for a `cargo test` process) is
        // itself a documented, harmless failure mode here — either way this
        // process ends up with a real console attached, which is all
        // `GenerateConsoleCtrlEvent` requires.
        unsafe { AllocConsole() };

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known long-running system command.
        let spawned =
            unsafe { spawn_suspended("cmd.exe /c ping -n 30 127.0.0.1 >nul", false, true, None) }
                .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        crate::console::generate_ctrl_event(crate::console::CTRL_BREAK_EVENT, spawned.process_id)
            .expect(
                "GenerateConsoleCtrlEvent should succeed for a process group this process just created",
            );

        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let exit_code = unsafe { wait(spawned.process, Some(10_000)) }.unwrap();
        assert!(
            exit_code.is_some(),
            "the child should have exited in response to CTRL_BREAK_EVENT within the timeout"
        );

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn spawn_suspended_with_environment_overrides_the_childs_view() {
        // A minimal environment block containing exactly one variable: if
        // it reaches the child, `if defined` sees it and the process exits
        // 42; if `environment` were silently ignored (e.g. the flag/pointer
        // wiring regressed), the child would see none of our variables and
        // exit 1 instead — the round trip a plain "does it compile" check
        // wouldn't catch.
        let block = environment_block(core::iter::once(("RUSTY_WIN32_TEST_VAR", "1")));
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary; `block` was built by
        // `environment_block`, which always double-NUL-terminates.
        let spawned = unsafe {
            spawn_suspended(
                "cmd.exe /c if defined RUSTY_WIN32_TEST_VAR (exit 42) else (exit 1)",
                false,
                false,
                Some(&block),
            )
        }
        .expect("CreateProcessW should succeed");

        // SAFETY: `spawned.thread` is freshly created, not yet resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let exit_code = unsafe { wait(spawned.process, None) }.unwrap();
        assert_eq!(exit_code, Some(42));

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn environment_block_is_double_nul_terminated() {
        let block = environment_block(core::iter::empty());
        assert_eq!(block, alloc::vec![0u16, 0u16]);

        let block = environment_block([("A", "1"), ("B", "2")].into_iter());
        let text: alloc::string::String =
            char::decode_utf16(block[..block.len() - 1].iter().copied())
                .map(|r| r.unwrap())
                .collect();
        assert_eq!(text, "A=1\0B=2\0");
        assert_eq!(block.last(), Some(&0u16));
    }

    #[test]
    fn environment_snapshot_includes_a_well_known_system_variable() {
        let pairs = environment_snapshot().expect("GetEnvironmentStringsW should succeed");
        assert!(
            pairs
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("SystemRoot")),
            "every real Windows process should have SystemRoot in its environment"
        );
    }

    #[test]
    fn environment_snapshot_includes_a_variable_this_process_just_set() {
        // SAFETY: this crate's CI runs its test suite with
        // RUST_TEST_THREADS=1 (see .github/workflows/ci.yml), so no other
        // test can concurrently read/write the real environment while this
        // one does.
        unsafe { std::env::set_var("RUSTY_WIN32_ENV_SNAPSHOT_TEST", "hello") };

        let pairs = environment_snapshot().expect("GetEnvironmentStringsW should succeed");
        let found = pairs
            .iter()
            .find(|(name, _)| name == "RUSTY_WIN32_ENV_SNAPSHOT_TEST");
        assert_eq!(
            found.map(|(_, value)| value.as_str()),
            Some("hello"),
            "the snapshot should include a variable this process just set"
        );

        // SAFETY: see above.
        unsafe { std::env::remove_var("RUSTY_WIN32_ENV_SNAPSHOT_TEST") };
    }

    #[test]
    fn get_env_var_reports_none_for_an_unset_variable() {
        let found = get_env_var("RUSTY_WIN32_THIS_VAR_SHOULD_NOT_EXIST")
            .expect("GetEnvironmentVariableW should succeed even when the variable is unset");
        assert_eq!(found, None);
    }

    #[test]
    fn set_env_var_then_get_env_var_round_trips() {
        set_env_var("RUSTY_WIN32_ENV_VAR_TEST", Some("hello"))
            .expect("SetEnvironmentVariableW should succeed");
        let found = get_env_var("RUSTY_WIN32_ENV_VAR_TEST")
            .expect("GetEnvironmentVariableW should succeed");
        assert_eq!(found.as_deref(), Some("hello"));

        set_env_var("RUSTY_WIN32_ENV_VAR_TEST", None)
            .expect("SetEnvironmentVariableW should succeed deleting a variable");
        let found_after_delete = get_env_var("RUSTY_WIN32_ENV_VAR_TEST")
            .expect("GetEnvironmentVariableW should succeed");
        assert_eq!(found_after_delete, None);
    }

    #[test]
    fn list_processes_includes_the_calling_process_itself() {
        let entries = list_processes()
            .expect("CreateToolhelp32Snapshot/Process32FirstW/Process32NextW should succeed");
        let this_pid = current_pid();
        let found = entries
            .iter()
            .find(|entry| entry.pid == this_pid)
            .unwrap_or_else(|| panic!("the calling process's own pid {this_pid} should appear in a system-wide snapshot"));
        assert!(
            found.thread_count >= 1,
            "a running process should report at least one thread"
        );
        assert!(
            !found.exe_file.is_empty(),
            "the calling process's own entry should report a non-empty exe_file"
        );
    }

    #[test]
    fn times_reports_plausible_creation_and_exit_timestamps() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        // Queried before waiting for the exit — `cmd.exe /c exit 0` may
        // already be done by now, so only `creation`/`kernel_time`/
        // `user_time` are asserted on here, not `exit`.
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        let before = unsafe { times(spawned.process) }.expect("GetProcessTimes should succeed");
        assert!(
            before.creation.secs > 1_700_000_000,
            "creation should be a plausible wall-clock timestamp (after ~2023)"
        );
        assert!(before.kernel_time.nanos < 1_000_000_000);
        assert!(before.user_time.nanos < 1_000_000_000);

        // SAFETY: `spawned.process` is still the same valid handle.
        unsafe { wait(spawned.process, None) }.unwrap();

        // SAFETY: same valid handle, now signaled/exited.
        let after = unsafe { times(spawned.process) }.expect("GetProcessTimes should succeed");
        assert!(
            after.exit.secs > 1_700_000_000,
            "exit should be a plausible wall-clock timestamp once the process has exited"
        );
        assert!(
            after.exit.secs >= after.creation.secs,
            "exit must not precede creation"
        );

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn sleep_ms_blocks_for_at_least_the_requested_duration() {
        let start = std::time::Instant::now();
        sleep_ms(50);
        assert!(
            start.elapsed().as_millis() >= 50,
            "Sleep should block for at least the requested duration"
        );
    }

    #[test]
    fn sleep_ms_ex_blocks_and_reports_zero_when_not_woken_by_an_apc() {
        const WAIT_IO_COMPLETION: u32 = 192;
        let start = std::time::Instant::now();
        let result = sleep_ms_ex(50, false);
        assert!(
            start.elapsed().as_millis() >= 50,
            "SleepEx should block for at least the requested duration"
        );
        assert_ne!(
            result, WAIT_IO_COMPLETION,
            "no APC was queued, so this should report 0, not WAIT_IO_COMPLETION"
        );
    }

    #[test]
    fn logical_processor_count_is_nonzero() {
        assert!(
            logical_processor_count() > 0,
            "every real machine has at least one logical processor"
        );
    }

    #[test]
    fn tick_count_is_nondecreasing_and_advances() {
        let before = tick_count();
        sleep_ms(20);
        let after = tick_count();
        assert!(after >= before, "GetTickCount64 should never go backwards");
        assert!(
            after > before,
            "GetTickCount64 should have advanced after a real sleep"
        );
    }

    #[test]
    fn logical_processor_information_reports_at_least_one_processor_core_entry() {
        let entries =
            logical_processor_information().expect("GetLogicalProcessorInformation should succeed");
        assert!(
            !entries.is_empty(),
            "a real machine should report at least one topology entry"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.relationship == ProcessorRelationship::ProcessorCore),
            "a real machine should report at least one ProcessorCore entry, got: {entries:?}"
        );
        for entry in &entries {
            assert_ne!(
                entry.processor_mask, 0,
                "every entry's processor mask should cover at least one CPU"
            );
        }
    }

    #[test]
    fn add_vectored_exception_handler_intercepts_a_raised_exception() {
        // A custom, application-defined exception code (the documented
        // `0xE0000000`-`0xEFFFFFFF` range) that this test raises itself
        // via `RaiseException` — a safe, deterministic way to exercise a
        // handler without triggering a real CPU fault. Declared locally
        // (not a public wrapper) since raising exceptions isn't part of
        // this issue's scope, only handling them.
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn RaiseException(
                exception_code: u32,
                exception_flags: u32,
                number_of_arguments: u32,
                arguments: *const usize,
            );
        }
        const TEST_EXCEPTION_CODE: u32 = 0xE0AA_0001;
        static CAUGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

        extern "system" fn handler(exception_info: *mut ExceptionPointers) -> i32 {
            // SAFETY: Windows always passes a valid, non-null pointer to
            // a registered vectored exception handler; `ExceptionRecord`
            // is the first field of the real `EXCEPTION_RECORD` struct
            // (`ExceptionCode`, a `DWORD`), always readable at offset 0
            // regardless of the rest of that struct's (unmodeled) layout.
            let code = unsafe { *(*exception_info).exception_record.cast::<u32>() };
            if code == TEST_EXCEPTION_CODE {
                CAUGHT.store(true, std::sync::atomic::Ordering::SeqCst);
                EXCEPTION_CONTINUE_EXECUTION
            } else {
                EXCEPTION_CONTINUE_SEARCH
            }
        }

        let vectored_handle = add_vectored_exception_handler(true, handler)
            .expect("AddVectoredExceptionHandler should succeed");

        // SAFETY: a made-up, non-fatal application exception code with no
        // arguments; `handler` above returns `EXCEPTION_CONTINUE_EXECUTION`
        // for it, so `RaiseException` returns normally instead of crashing
        // this test process.
        unsafe { RaiseException(TEST_EXCEPTION_CODE, 0, 0, core::ptr::null()) };

        assert!(
            CAUGHT.load(std::sync::atomic::Ordering::SeqCst),
            "the installed vectored handler should have intercepted the raised exception"
        );

        // SAFETY: `vectored_handle` is the value `add_vectored_exception_handler`
        // returned above, not yet removed.
        unsafe { remove_vectored_exception_handler(vectored_handle) }
            .expect("RemoveVectoredExceptionHandler should succeed");
    }

    #[test]
    fn computer_name_matches_the_real_environment_computername() {
        let name = computer_name().expect("GetComputerNameW should succeed");
        let env_name = std::env::var("COMPUTERNAME")
            .expect("COMPUTERNAME should be set in any real Windows process's environment");
        assert!(
            name.eq_ignore_ascii_case(&env_name),
            "expected {env_name:?}, got {name:?}"
        );
    }

    #[test]
    fn memory_status_reports_plausible_values() {
        let status = memory_status().expect("GlobalMemoryStatusEx should succeed");
        assert!(
            status.memory_load <= 100,
            "memory load should be a percentage, got {}",
            status.memory_load
        );
        assert!(
            status.total_phys > 0,
            "a real machine should report nonzero total physical memory"
        );
        assert!(
            status.avail_phys <= status.total_phys,
            "available physical memory can't exceed the total"
        );
        assert!(
            status.avail_page_file <= status.total_page_file,
            "available page-file space can't exceed the total"
        );
    }

    #[test]
    fn set_error_mode_returns_the_previous_mode() {
        let original = set_error_mode(SEM_FAILCRITICALERRORS);
        let previous = set_error_mode(original);
        assert_eq!(
            previous, SEM_FAILCRITICALERRORS,
            "SetErrorMode should return the mode just set by the prior call"
        );
    }

    #[test]
    fn set_priority_class_then_priority_class_round_trips() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // SAFETY: `spawned.process` is a freshly created, valid handle.
        unsafe { set_priority_class(spawned.process, IDLE_PRIORITY_CLASS) }
            .expect("SetPriorityClass should succeed");
        // SAFETY: same handle.
        let class =
            unsafe { priority_class(spawned.process) }.expect("GetPriorityClass should succeed");
        assert_eq!(class, IDLE_PRIORITY_CLASS);

        // SAFETY: same handle.
        unsafe { set_priority_class(spawned.process, NORMAL_PRIORITY_CLASS) }
            .expect("SetPriorityClass should succeed");
        // SAFETY: same handle.
        let class =
            unsafe { priority_class(spawned.process) }.expect("GetPriorityClass should succeed");
        assert_eq!(class, NORMAL_PRIORITY_CLASS);

        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { wait(spawned.process, None) }.unwrap();

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn set_affinity_then_affinity_round_trips() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // SAFETY: `spawned.process` is a freshly created, valid handle.
        let (_, system_mask) =
            unsafe { affinity(spawned.process) }.expect("GetProcessAffinityMask should succeed");
        // Restrict to just the lowest bit the system mask actually has set,
        // so this passes on a single-CPU CI runner too.
        let one_cpu = 1usize << system_mask.trailing_zeros();

        // SAFETY: same handle; `one_cpu` is a subset of `system_mask`.
        unsafe { set_affinity(spawned.process, one_cpu) }
            .expect("SetProcessAffinityMask should succeed");
        // SAFETY: same handle.
        let (process_mask, _) =
            unsafe { affinity(spawned.process) }.expect("GetProcessAffinityMask should succeed");
        assert_eq!(process_mask, one_cpu);

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn list_threads_open_thread_suspend_and_resume_round_trip() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known long-running system command.
        let spawned =
            unsafe { spawn_suspended("cmd.exe /c ping -n 30 127.0.0.1 >nul", false, false, None) }
                .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        let thread_ids = list_threads(spawned.process_id)
            .expect("CreateToolhelp32Snapshot/Thread32First/Thread32Next should succeed");
        assert!(
            thread_ids.contains(&spawned.thread_id),
            "the process's own main thread id should appear in its thread list"
        );

        let thread_handle = open_thread(spawned.thread_id, THREAD_SUSPEND_RESUME)
            .expect("OpenThread should succeed for a live thread id this test itself just started");

        // SAFETY: `thread_handle` is a freshly opened, valid handle with
        // THREAD_SUSPEND_RESUME; this is the operation under test.
        unsafe { suspend_thread(thread_handle) }.expect("SuspendThread should succeed");
        // SAFETY: same handle.
        unsafe { resume(thread_handle) }.expect("ResumeThread should succeed");

        // SAFETY: `spawned.process` is a valid, currently-open handle with
        // full access rights (CreateProcessW itself opened it).
        unsafe { terminate(spawned.process, 0) }.expect("TerminateProcess should succeed");
        // SAFETY: still the same valid handle.
        unsafe { wait(spawned.process, Some(5_000)) }.unwrap();

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(thread_handle).unwrap();
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn thread_exit_code_reports_still_active_then_the_real_code() {
        const STILL_ACTIVE: u32 = 259;

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known long-running system command.
        let spawned =
            unsafe { spawn_suspended("cmd.exe /c ping -n 30 127.0.0.1 >nul", false, false, None) }
                .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        let thread_handle = open_thread(spawned.thread_id, THREAD_QUERY_INFORMATION | SYNCHRONIZE)
            .expect("OpenThread should succeed for a live thread id this test itself just started");

        // SAFETY: `thread_handle` is a freshly opened, valid handle with
        // THREAD_QUERY_INFORMATION; this is the operation under test.
        let code = unsafe { thread_exit_code(thread_handle) }
            .expect("GetExitCodeThread should succeed for a still-running thread");
        assert_eq!(code, STILL_ACTIVE);

        // SAFETY: `spawned.process` is a valid, currently-open handle with
        // full access rights (CreateProcessW itself opened it).
        unsafe { terminate(spawned.process, 7) }.expect("TerminateProcess should succeed");
        // SAFETY: still the same valid handle.
        unsafe { wait(spawned.process, Some(5_000)) }.unwrap();
        // The process object becoming signaled doesn't guarantee
        // `GetExitCodeThread` already reflects the thread's final exit
        // code — Windows doesn't document those two transitions as
        // atomic with each other, and this raced under CI. Explicitly
        // wait on `thread_handle` itself (needs `SYNCHRONIZE`, added
        // above) to close that window before reading the exit code.
        // SAFETY: `thread_handle` is a valid, currently-open handle with
        // SYNCHRONIZE.
        unsafe { crate::handle::wait_single_ex(thread_handle, Some(5_000), false) }
            .expect("WaitForSingleObjectEx should succeed waiting on the thread handle");

        // SAFETY: same handle, now that the thread itself has been waited
        // on and confirmed signaled.
        let code = unsafe { thread_exit_code(thread_handle) }
            .expect("GetExitCodeThread should succeed after the thread has exited");
        assert_eq!(code, 7);

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(thread_handle).unwrap();
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn thread_times_reports_plausible_creation_and_exit_timestamps() {
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { resume(spawned.thread) }.expect("ResumeThread should succeed");

        let thread_handle = open_thread(spawned.thread_id, THREAD_QUERY_INFORMATION | SYNCHRONIZE)
            .expect("OpenThread should succeed for a live thread id this test itself just started");

        // Queried before waiting for the exit — `cmd.exe /c exit 0` may
        // already be done by now, so only `creation`/`kernel_time`/
        // `user_time` are asserted on here, not `exit`.
        // SAFETY: `thread_handle` is a valid, currently-open handle.
        let before = unsafe { thread_times(thread_handle) }.expect("GetThreadTimes should succeed");
        assert!(
            before.creation.secs > 1_700_000_000,
            "creation should be a plausible wall-clock timestamp (after ~2023)"
        );
        assert!(before.kernel_time.nanos < 1_000_000_000);
        assert!(before.user_time.nanos < 1_000_000_000);

        // SAFETY: `spawned.process` is still the same valid handle.
        unsafe { wait(spawned.process, None) }.unwrap();
        // The process handle becoming signaled doesn't guarantee
        // GetThreadTimes' `exit` field is already populated for the
        // thread itself — the same race already fixed for
        // `thread_exit_code_reports_still_active_then_the_real_code`.
        // Explicitly wait on the thread handle (needs `SYNCHRONIZE`,
        // added above) to close that window.
        // SAFETY: `thread_handle` is a valid, currently-open handle with
        // SYNCHRONIZE.
        unsafe { crate::handle::wait_single_ex(thread_handle, Some(5_000), false) }
            .expect("WaitForSingleObjectEx should succeed waiting on the thread handle");

        // SAFETY: same valid handle, now that the thread itself has been
        // waited on and confirmed signaled.
        let after = unsafe { thread_times(thread_handle) }.expect("GetThreadTimes should succeed");
        assert!(
            after.exit.secs > 1_700_000_000,
            "exit should be a plausible wall-clock timestamp once the thread has exited"
        );

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(thread_handle).unwrap();
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }
}
