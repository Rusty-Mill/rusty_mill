//! Windows Job Objects — the primitive rush's own
//! `docs/WINDOWS_JOB_CONTROL.md` designs background-job tracking against
//! (`&`, `jobs`, `wait`, `kill`, `$!`). A Job Object groups processes for
//! lifetime management, the closest Windows analog of a POSIX process group
//! for this purpose (not a perfect match — see that design doc's
//! "Terminology" note). Assigning a suspended process (see
//! [`crate::process::spawn_suspended`]) to a job *before* resuming its main
//! thread, with [`set_kill_on_close`] applied, means closing the job handle
//! (e.g. on shell exit) kills every process in it — the Windows analog of
//! the process-group-wide cleanup rush's Unix `job.rs` gets from signals.
//! Any child the job's own member processes spawn inherits job membership
//! automatically (a job property, not something a child opts into), which
//! is what makes this correct for a whole subtree, not just one process.
//!
//! [`create_completion_port`]/[`associate_completion_port`]/
//! [`wait_for_message`] close a real gap in the above: [`process_ids`] is
//! only ever a poll, never a push — there is no Unix-`SIGCHLD` equivalent
//! for "a job member just exited" without them. Associating a job with an
//! I/O completion port (a mechanism otherwise entirely about file I/O, not
//! process lifecycle — Windows repurposes it here rather than defining a
//! job-specific notification primitive) makes the OS itself post a message
//! every time a member process is created, exits, or the job empties out,
//! which [`wait_for_message`] blocks for. As with every other primitive in
//! this crate, deciding what a caller does with a given [`JOB_OBJECT_MSG_EXIT_PROCESS`]
//! (etc.) is the caller's policy — this module only delivers the raw message.
//!
//! [`set_resource_limits`]/[`limits`]/[`accounting`] round out the round-2
//! capability assessment's Job-Object-related item: `rush`'s `ulimit` is a
//! flat "not supported" on Windows today, and Job-Object memory/CPU-time/
//! active-process limits are that doc's own answer for the only realistic
//! partial fix — the struct fields these use
//! (`JobObjectExtendedLimitInformation`'s memory/CPU-time/process-count
//! fields) were already modeled bit-for-bit for [`set_kill_on_close`], just
//! never set beyond its one `LimitFlags` bit until now. [`accounting`]
//! (`JobObjectBasicAndIoAccountingInformation`) is Windows' real analog of
//! POSIX `cutime`/`cstime`: CPU time aggregated across every process a job
//! has *ever* contained, including ones already exited — unlike
//! [`crate::process::times`], which only ever reports one still-open
//! process handle's own times.

use crate::error::Win32Error;
use crate::handle::RawHandle;
use crate::time::Timespec;

extern crate alloc;
use alloc::vec::Vec;

/// `SetInformationJobObject`'s `LimitFlags` bit: close the job handle (or
/// have every handle to it close, e.g. on process exit) and every process
/// still assigned to it terminates.
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
/// `LimitFlags` bit: cap a single process's user-mode CPU time
/// (`per_process_user_time_limit`).
const JOB_OBJECT_LIMIT_PROCESS_TIME: u32 = 0x0000_0002;
/// `LimitFlags` bit: cap the job's total user-mode CPU time across every
/// member process (`per_job_user_time_limit`).
const JOB_OBJECT_LIMIT_JOB_TIME: u32 = 0x0000_0004;
/// `LimitFlags` bit: cap the number of processes simultaneously active in
/// the job (`active_process_limit`).
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
/// `LimitFlags` bit: cap one process's committed memory, in bytes
/// (`process_memory_limit`).
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
/// `LimitFlags` bit: cap the job's total committed memory, in bytes
/// (`job_memory_limit`).
const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x0000_0200;

/// `QueryInformationJobObject`/`SetInformationJobObject`'s
/// `JobObjectExtendedLimitInformation` class.
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: u32 = 9;
/// `QueryInformationJobObject`'s `JobObjectBasicProcessIdList` class.
const JOB_OBJECT_BASIC_PROCESS_ID_LIST_CLASS: u32 = 3;
/// `SetInformationJobObject`'s `JobObjectAssociateCompletionPortInformation`
/// class.
const JOB_OBJECT_ASSOCIATE_COMPLETION_PORT_CLASS: u32 = 7;
/// `QueryInformationJobObject`'s `JobObjectBasicAndIoAccountingInformation`
/// class.
const JOB_OBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION_CLASS: u32 = 8;

/// 100ns ticks per second — `LARGE_INTEGER` CPU-time fields in this module
/// (`JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`'s `TotalUserTime`/
/// `TotalKernelTime`, `JOBOBJECT_BASIC_LIMIT_INFORMATION`'s
/// `*_user_time_limit`) use the same tick unit `FILETIME` does, without a
/// `FILETIME`'s two-`u32` layout — these are already a plain `i64`. The same
/// standard conversion constant `process.rs`/`fs.rs`/`time.rs` use for
/// `FILETIME`, duplicated locally per this crate's existing convention.
const HUNDRED_NS_PER_SEC: i64 = 10_000_000;
const NANOS_PER_HUNDRED_NS: i64 = 100;

/// An elapsed-duration 100ns tick count (no FILETIME epoch to subtract,
/// unlike a wall-clock timestamp) to [`Timespec`] — the same reuse
/// `process::times`'s `kernel_time`/`user_time` and `time::now_monotonic`'s
/// result already rely on for a non-wall-clock value.
fn ticks_to_duration(ticks_100ns: i64) -> Timespec {
    let secs = ticks_100ns / HUNDRED_NS_PER_SEC;
    let remainder_100ns = ticks_100ns % HUNDRED_NS_PER_SEC;
    Timespec {
        secs,
        nanos: (remainder_100ns * NANOS_PER_HUNDRED_NS) as u32,
    }
}

/// The reverse of [`ticks_to_duration`], for building a limit's raw
/// `LARGE_INTEGER` tick count from a caller-supplied [`Timespec`] duration.
fn duration_to_ticks(t: Timespec) -> i64 {
    t.secs * HUNDRED_NS_PER_SEC + i64::from(t.nanos) / NANOS_PER_HUNDRED_NS
}

/// `GetQueuedCompletionStatus`'s own documented "no packet queued within the
/// timeout" code, surfaced via `GetLastError()` on a `FALSE` return — the
/// same numeric value `process::wait`/`console::wait_readable` already
/// compare against locally (not one of `error.rs`'s named `ERROR_*`
/// constants, since it's a wait-outcome code reused as an error rather than
/// an ordinary `GetLastError` failure).
const WAIT_TIMEOUT: u32 = 258;

/// A member process was created — includes the job's own initial process.
pub const JOB_OBJECT_MSG_NEW_PROCESS: u32 = 6;
/// A member process exited normally. `pid` names which one.
pub const JOB_OBJECT_MSG_EXIT_PROCESS: u32 = 7;
/// A member process exited abnormally (e.g. crashed, or was terminated).
pub const JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS: u32 = 8;
/// The job has no member processes left — the clearest "this job is done"
/// signal, the nearest analog to a Unix process group with no members left.
pub const JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO: u32 = 4;
/// The job as a whole exceeded its total user-mode CPU time limit (only
/// meaningful if such a limit was set, which this crate doesn't currently
/// expose a way to set).
pub const JOB_OBJECT_MSG_END_OF_JOB_TIME: u32 = 1;
/// A single process exceeded its per-process user-mode CPU time limit (same
/// caveat as [`JOB_OBJECT_MSG_END_OF_JOB_TIME`]).
pub const JOB_OBJECT_MSG_END_OF_PROCESS_TIME: u32 = 2;
/// The job's active-process-count limit was exceeded (same caveat).
pub const JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT: u32 = 3;
/// A process tried to exceed its memory limit (same caveat).
pub const JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT: u32 = 9;
/// The job as a whole tried to exceed its memory limit (same caveat).
pub const JOB_OBJECT_MSG_JOB_MEMORY_LIMIT: u32 = 10;

// Layouts below verified against mingw-w64's `winnt.h` (`_IO_COUNTERS`,
// `_JOBOBJECT_BASIC_LIMIT_INFORMATION`, `_JOBOBJECT_EXTENDED_LIMIT_INFORMATION`,
// `_JOBOBJECT_BASIC_PROCESS_ID_LIST`) the same way as `process.rs`'s
// structs: a `_Static_assert`-based probe compiled against the real header,
// not hand-computed padding.

/// `IO_COUNTERS`: `size_of` 48, `align_of` 8 on x86_64.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}
const _: () = assert!(core::mem::size_of::<IoCounters>() == 48);
const _: () = assert!(core::mem::align_of::<IoCounters>() == 8);

/// `JOBOBJECT_BASIC_LIMIT_INFORMATION`: `size_of` 64, `align_of` 8.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}
const _: () = assert!(core::mem::size_of::<JobObjectBasicLimitInformation>() == 64);
const _: () = assert!(core::mem::align_of::<JobObjectBasicLimitInformation>() == 8);
const _: () = assert!(core::mem::offset_of!(JobObjectBasicLimitInformation, limit_flags) == 16);

/// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`: `size_of` 144, `align_of` 8.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}
const _: () = assert!(core::mem::size_of::<JobObjectExtendedLimitInformation>() == 144);
const _: () = assert!(core::mem::align_of::<JobObjectExtendedLimitInformation>() == 8);
const _: () = assert!(core::mem::offset_of!(JobObjectExtendedLimitInformation, io_info) == 64);
const _: () =
    assert!(core::mem::offset_of!(JobObjectExtendedLimitInformation, process_memory_limit) == 112);

/// `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION`: `size_of` 48, `align_of` 8.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JobObjectBasicAccountingInformation {
    total_user_time: i64,
    total_kernel_time: i64,
    this_period_total_user_time: i64,
    this_period_total_kernel_time: i64,
    total_page_fault_count: u32,
    total_processes: u32,
    active_processes: u32,
    total_terminated_processes: u32,
}
const _: () = assert!(core::mem::size_of::<JobObjectBasicAccountingInformation>() == 48);
const _: () = assert!(core::mem::align_of::<JobObjectBasicAccountingInformation>() == 8);
const _: () =
    assert!(core::mem::offset_of!(JobObjectBasicAccountingInformation, total_kernel_time) == 8);
const _: () = assert!(
    core::mem::offset_of!(JobObjectBasicAccountingInformation, total_page_fault_count) == 32
);
const _: () =
    assert!(core::mem::offset_of!(JobObjectBasicAccountingInformation, total_processes) == 36);
const _: () =
    assert!(core::mem::offset_of!(JobObjectBasicAccountingInformation, active_processes) == 40);
const _: () = assert!(
    core::mem::offset_of!(
        JobObjectBasicAccountingInformation,
        total_terminated_processes
    ) == 44
);

/// `JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION`: `size_of` 96,
/// `align_of` 8.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct JobObjectBasicAndIoAccountingInformation {
    basic_info: JobObjectBasicAccountingInformation,
    io_info: IoCounters,
}
const _: () = assert!(core::mem::size_of::<JobObjectBasicAndIoAccountingInformation>() == 96);
const _: () = assert!(core::mem::align_of::<JobObjectBasicAndIoAccountingInformation>() == 8);
const _: () =
    assert!(core::mem::offset_of!(JobObjectBasicAndIoAccountingInformation, io_info) == 48);

/// `JOBOBJECT_ASSOCIATE_COMPLETION_PORT`: `size_of` 16, `align_of` 8 on
/// x86_64 — two pointer-sized fields, no padding.
#[repr(C)]
#[derive(Clone, Copy)]
struct JobObjectAssociateCompletionPort {
    completion_key: *mut core::ffi::c_void,
    completion_port: RawHandle,
}
const _: () = assert!(core::mem::size_of::<JobObjectAssociateCompletionPort>() == 16);
const _: () = assert!(core::mem::align_of::<JobObjectAssociateCompletionPort>() == 8);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(job_attributes: *const core::ffi::c_void, name: *const u16) -> RawHandle;
    fn OpenJobObjectW(desired_access: u32, inherit_handle: i32, name: *const u16) -> RawHandle;
    fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
    fn IsProcessInJob(process: RawHandle, job: RawHandle, result: *mut i32) -> i32;
    fn TerminateJobObject(job: RawHandle, exit_code: u32) -> i32;
    fn SetInformationJobObject(
        job: RawHandle,
        job_object_information_class: u32,
        job_object_information: *const core::ffi::c_void,
        job_object_information_length: u32,
    ) -> i32;
    fn QueryInformationJobObject(
        job: RawHandle,
        job_object_information_class: u32,
        job_object_information: *mut core::ffi::c_void,
        job_object_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
    fn CreateIoCompletionPort(
        file_handle: RawHandle,
        existing_completion_port: RawHandle,
        completion_key: usize,
        number_of_concurrent_threads: u32,
    ) -> RawHandle;
    fn GetQueuedCompletionStatus(
        completion_port: RawHandle,
        bytes_transferred: *mut u32,
        completion_key: *mut usize,
        overlapped: *mut *mut core::ffi::c_void,
        milliseconds: u32,
    ) -> i32;
}

/// Create a new, anonymous Job Object — one shell background job's worth of
/// process-group-equivalent tracking. No process is assigned yet; pair with
/// [`assign`] (before resuming a suspended process — see
/// [`crate::process::spawn_suspended`]) and, almost always,
/// [`set_kill_on_close`].
pub fn create() -> Result<RawHandle, Win32Error> {
    // SAFETY: both arguments are documented-valid NULLs (default security
    // attributes, anonymous/unnamed job).
    let job = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
    if job.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(job)
    }
}

/// `OpenJobObjectW`'s/every other Job Object call's full-access-rights
/// bitmask (`STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F`) — matching
/// [`create`]'s own implicit, always-full-access grant, so a caller that
/// doesn't need to think about individual `JOB_OBJECT_*` bits can just use
/// this.
pub const JOB_OBJECT_ALL_ACCESS: u32 = 0x001F_003F;

/// Open a named Job Object by name — `OpenJobObjectW`, the reverse
/// direction of [`create`], which only ever makes anonymous jobs. No
/// current `rush` feature asks for this; filed for Win32 parity.
pub fn open_by_name(name: &str, desired_access: u32) -> Result<RawHandle, Win32Error> {
    let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
    // `inherit_handle = 0` (not inheritable) is a documented valid input.
    let job = unsafe { OpenJobObjectW(desired_access, 0, wide.as_ptr()) };
    if job.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(job)
    }
}

/// Set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on `job`: every process still
/// assigned to it terminates when its last handle closes, e.g. via
/// [`crate::handle::close`] on shell exit — the Windows analog of the
/// process-group-wide cleanup rush's Unix `job.rs` gets from signals.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn set_kill_on_close(job: RawHandle) -> Result<(), Win32Error> {
    let info = JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation {
            limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized, initialized struct of
    // exactly the type `JobObjectExtendedLimitInformation` names.
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            (&info as *const JobObjectExtendedLimitInformation).cast(),
            core::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Clear `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on `job` (the reverse of
/// [`set_kill_on_close`]): its member processes survive every handle to
/// `job` closing, including implicitly at this process's own exit —
/// backs `disown`. Without this, there's no way to let a tracked job
/// outlive the shell: a job handle is closed exactly like any other
/// handle when its owning process terminates, so kill-on-close would
/// still fire even if a caller simply stopped tracking the job and closed
/// its own handle to it, or just exited without closing anything at all.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn clear_kill_on_close(job: RawHandle) -> Result<(), Win32Error> {
    let info = JobObjectExtendedLimitInformation::default();
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized, initialized struct of
    // exactly the type `JobObjectExtendedLimitInformation` names.
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            (&info as *const JobObjectExtendedLimitInformation).cast(),
            core::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Assign `process` to `job`. Any child `process` itself later spawns
/// inherits job membership automatically. Must happen before a
/// `CREATE_SUSPENDED`-started `process` is resumed (see
/// [`crate::process::spawn_suspended`]/[`crate::process::resume`]) for job
/// membership to be guaranteed before anything in the process tree runs.
///
/// # Safety
///
/// `job` and `process` must both be currently-open, valid handles.
pub unsafe fn assign(job: RawHandle, process: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: both handles are caller-supplied per this function's own
    // safety contract.
    let ok = unsafe { AssignProcessToJobObject(job, process) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Check whether `process` already belongs to `job` — or, with `job: None`,
/// whether it belongs to *any* job at all (`IsProcessInJob`'s own
/// documented `NULL`-means-any-job convention). Windows automatically nests
/// every child a job member spawns into that same job, and some
/// environments (e.g. GitHub Actions' Windows runners, which wrap each
/// step's whole process tree in an ambient job for orphan cleanup) start a
/// process already job-scoped before this crate ever sees it — checking
/// this first avoids a surprise [`assign`] failure (a process can't be
/// reassigned to a different job unless its current one was created with
/// `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`/`BREAKAWAY_OK`, which this crate
/// doesn't set).
///
/// # Safety
///
/// `process` must be a currently-open, valid process handle; `job`, if
/// given, must be a currently-open, valid Job Object handle.
pub unsafe fn is_in_job(process: RawHandle, job: Option<RawHandle>) -> Result<bool, Win32Error> {
    let mut result: i32 = 0;
    // SAFETY: `process` is caller-supplied per this function's own safety
    // contract; `job` is either NULL (documented as "any job") or a
    // caller-supplied, valid handle per the same contract; `result` is a
    // valid out-pointer.
    let ok = unsafe { IsProcessInJob(process, job.unwrap_or(core::ptr::null_mut()), &mut result) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(result != 0)
    }
}

/// Terminate every process currently assigned to `job` with `exit_code` —
/// backs `kill %n`. Unlike closing the job handle with
/// [`set_kill_on_close`] set, this doesn't require giving up the handle.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn terminate(job: RawHandle, exit_code: u32) -> Result<(), Win32Error> {
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract.
    let ok = unsafe { TerminateJobObject(job, exit_code) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// The pids of every process currently assigned to `job` — one way to poll
/// "is this job still running" (an empty result) for `wait`/`jobs`, in the
/// absence of a blocking wait-for-job-completion primitive as simple as
/// Unix `SIGCHLD` (see rush's `docs/WINDOWS_JOB_CONTROL.md` for the
/// polling-vs-completion-port discussion).
///
/// Returns `Vec<u32>`, matching every other pid-carrying value in this
/// crate's public surface (`ProcessEntry.pid`, `JobMessage.pid`,
/// `SpawnedProcess.process_id`, `open_by_pid`'s parameter) — the raw
/// `JOBOBJECT_BASIC_PROCESS_ID_LIST` wire format Windows reports is
/// pointer-sized (`ULONG_PTR`, for struct alignment, not because a pid is
/// ever wider than 32 bits), narrowed here rather than leaking that
/// internal width into this function's own return type.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn process_ids(job: RawHandle) -> Result<Vec<u32>, Win32Error> {
    const HEADER_LEN: usize = 8; // NumberOfAssignedProcesses + NumberOfProcessIdsInList
    let mut capacity: u32 = 32;
    loop {
        let buf_len = HEADER_LEN + capacity as usize * core::mem::size_of::<usize>();
        let mut buf: Vec<u8> = alloc::vec![0u8; buf_len];
        let mut returned_len: u32 = 0;
        // SAFETY: `job` is caller-supplied per this function's own safety
        // contract; `buf` is a valid, zeroed, `buf_len`-byte buffer;
        // `returned_len` is a valid out-pointer.
        let ok = unsafe {
            QueryInformationJobObject(
                job,
                JOB_OBJECT_BASIC_PROCESS_ID_LIST_CLASS,
                buf.as_mut_ptr().cast(),
                buf_len as u32,
                &mut returned_len,
            )
        };
        if ok == 0 {
            let err = Win32Error::last();
            if err == Win32Error::ERROR_MORE_DATA {
                // NumberOfAssignedProcesses is documented to be filled in
                // correctly even when the list itself didn't fit.
                // SAFETY: `buf` holds at least `HEADER_LEN` initialized
                // bytes even on this "too small" failure path.
                let assigned = unsafe { core::ptr::read_unaligned(buf.as_ptr().cast::<u32>()) };
                capacity = assigned.max(capacity * 2);
                continue;
            }
            return Err(err);
        }
        // SAFETY: a successful call guarantees the header and
        // `number_of_process_ids_in_list` pid-sized entries after it are
        // initialized.
        let in_list =
            unsafe { core::ptr::read_unaligned(buf.as_ptr().add(4).cast::<u32>()) } as usize;
        let pids_ptr = unsafe { buf.as_ptr().add(HEADER_LEN).cast::<usize>() };
        let pids = unsafe { core::slice::from_raw_parts(pids_ptr, in_list) };
        // Narrowing `usize` (the wire format's native pointer width) to
        // `u32` (every other pid in this crate) — safe for any real pid,
        // which Windows itself never assigns above 32 bits.
        return Ok(pids.iter().map(|&pid| pid as u32).collect());
    }
}

/// Resource limits settable via [`set_resource_limits`]/readable via
/// [`limits`] — the narrow subset of `ulimit` classes a Windows Job Object
/// can actually enforce (rush's own `docs/WINDOWS_BACKEND_ANALYSIS.md` §6
/// names this the only legitimate way to ever partially support `ulimit` on
/// Windows, rather than a `getrlimit`/`setrlimit` port). Every field is
/// `Option`: `None` leaves that particular limit unset/unenforced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobLimits {
    /// Maximum number of processes simultaneously active in the job at
    /// once.
    pub active_process_limit: Option<u32>,
    /// Maximum committed memory for a single process in the job, in bytes.
    pub process_memory_limit: Option<usize>,
    /// Maximum committed memory across the whole job, in bytes.
    pub job_memory_limit: Option<usize>,
    /// Maximum user-mode CPU time for a single process in the job.
    pub per_process_user_time_limit: Option<Timespec>,
    /// Maximum total user-mode CPU time across every process the job has
    /// ever contained.
    pub per_job_user_time_limit: Option<Timespec>,
}

/// Set `job`'s resource limits to `limits` —
/// `SetInformationJobObject(JobObjectExtendedLimitInformation)`.
///
/// **This replaces the job's entire limit-information block in one call**,
/// the same as [`set_kill_on_close`]/[`clear_kill_on_close`]: calling this
/// after either of those (or vice versa) clears whatever the other call
/// set, since none of the three queries the job's current limits first
/// before writing a fresh block. A caller wanting kill-on-close *and*
/// resource limits together needs to combine both concerns into one
/// `SetInformationJobObject` call itself — not currently exposed as a
/// single primitive here.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn set_resource_limits(job: RawHandle, limits: JobLimits) -> Result<(), Win32Error> {
    let mut basic = JobObjectBasicLimitInformation::default();
    if let Some(n) = limits.active_process_limit {
        basic.limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        basic.active_process_limit = n;
    }
    if let Some(t) = limits.per_process_user_time_limit {
        basic.limit_flags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
        basic.per_process_user_time_limit = duration_to_ticks(t);
    }
    if let Some(t) = limits.per_job_user_time_limit {
        basic.limit_flags |= JOB_OBJECT_LIMIT_JOB_TIME;
        basic.per_job_user_time_limit = duration_to_ticks(t);
    }
    let mut info = JobObjectExtendedLimitInformation {
        basic_limit_information: basic,
        ..Default::default()
    };
    if let Some(m) = limits.process_memory_limit {
        info.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.process_memory_limit = m;
    }
    if let Some(m) = limits.job_memory_limit {
        info.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        info.job_memory_limit = m;
    }
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized, initialized struct of
    // exactly the type `JobObjectExtendedLimitInformation` names.
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            (&info as *const JobObjectExtendedLimitInformation).cast(),
            core::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Read `job`'s current resource limits back —
/// `QueryInformationJobObject(JobObjectExtendedLimitInformation)`, the query
/// counterpart to [`set_resource_limits`]. A field reports `None` exactly
/// when its corresponding `LimitFlags` bit is clear, matching
/// [`set_resource_limits`]'s own `Option` convention rather than reporting a
/// meaningless raw zero for a limit nothing ever set.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn limits(job: RawHandle) -> Result<JobLimits, Win32Error> {
    let mut info = JobObjectExtendedLimitInformation::default();
    let mut returned_len: u32 = 0;
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized out-pointer;
    // `returned_len` is a valid out-pointer.
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            (&mut info as *mut JobObjectExtendedLimitInformation).cast(),
            core::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            &mut returned_len,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    let flags = info.basic_limit_information.limit_flags;
    Ok(JobLimits {
        active_process_limit: (flags & JOB_OBJECT_LIMIT_ACTIVE_PROCESS != 0)
            .then_some(info.basic_limit_information.active_process_limit),
        process_memory_limit: (flags & JOB_OBJECT_LIMIT_PROCESS_MEMORY != 0)
            .then_some(info.process_memory_limit),
        job_memory_limit: (flags & JOB_OBJECT_LIMIT_JOB_MEMORY != 0)
            .then_some(info.job_memory_limit),
        per_process_user_time_limit: (flags & JOB_OBJECT_LIMIT_PROCESS_TIME != 0).then_some(
            ticks_to_duration(info.basic_limit_information.per_process_user_time_limit),
        ),
        per_job_user_time_limit: (flags & JOB_OBJECT_LIMIT_JOB_TIME != 0).then_some(
            ticks_to_duration(info.basic_limit_information.per_job_user_time_limit),
        ),
    })
}

/// [`accounting`]'s result — `QueryInformationJobObject`'s
/// `JobObjectBasicAndIoAccountingInformation`, Windows' real analog of
/// POSIX `cutime`/`cstime`: CPU time and I/O counts aggregated across
/// *every* process the job has ever contained, including ones that already
/// exited — unlike [`crate::process::times`], which only ever reports one
/// still-open process handle's own times.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobAccounting {
    /// Total user-mode CPU time across every process the job has ever
    /// contained.
    pub total_user_time: Timespec,
    /// Total kernel-mode CPU time across every process the job has ever
    /// contained.
    pub total_kernel_time: Timespec,
    pub total_page_fault_count: u32,
    /// Every process ever assigned to the job, including ones already
    /// terminated.
    pub total_processes: u32,
    /// Processes currently assigned to the job — `0` once the job has
    /// emptied out, the same condition [`process_ids`] reports as an empty
    /// list.
    pub active_processes: u32,
    pub total_terminated_processes: u32,
    pub read_operation_count: u64,
    pub write_operation_count: u64,
    pub other_operation_count: u64,
    pub read_transfer_count: u64,
    pub write_transfer_count: u64,
    pub other_transfer_count: u64,
}

/// CPU-time and I/O accounting for `job`, aggregated across its whole
/// lifetime — `QueryInformationJobObject(JobObjectBasicAndIoAccountingInformation)`.
///
/// # Safety
///
/// `job` must be a currently-open, valid Job Object handle.
pub unsafe fn accounting(job: RawHandle) -> Result<JobAccounting, Win32Error> {
    let mut info = JobObjectBasicAndIoAccountingInformation::default();
    let mut returned_len: u32 = 0;
    // SAFETY: `job` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized out-pointer;
    // `returned_len` is a valid out-pointer.
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JOB_OBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION_CLASS,
            (&mut info as *mut JobObjectBasicAndIoAccountingInformation).cast(),
            core::mem::size_of::<JobObjectBasicAndIoAccountingInformation>() as u32,
            &mut returned_len,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(JobAccounting {
        total_user_time: ticks_to_duration(info.basic_info.total_user_time),
        total_kernel_time: ticks_to_duration(info.basic_info.total_kernel_time),
        total_page_fault_count: info.basic_info.total_page_fault_count,
        total_processes: info.basic_info.total_processes,
        active_processes: info.basic_info.active_processes,
        total_terminated_processes: info.basic_info.total_terminated_processes,
        read_operation_count: info.io_info.read_operation_count,
        write_operation_count: info.io_info.write_operation_count,
        other_operation_count: info.io_info.other_operation_count,
        read_transfer_count: info.io_info.read_transfer_count,
        write_transfer_count: info.io_info.write_transfer_count,
        other_transfer_count: info.io_info.other_transfer_count,
    })
}

/// Create a fresh I/O completion port not yet tied to any file or job —
/// `CreateIoCompletionPort` with no file handle to associate yet
/// (`INVALID_HANDLE_VALUE`) and no existing port to attach to. Pair with
/// [`associate_completion_port`] to start receiving a job's messages on it,
/// and [`wait_for_message`] to block for one.
pub fn create_completion_port() -> Result<RawHandle, Win32Error> {
    // `INVALID_HANDLE_VALUE` — all bits set, the documented sentinel
    // `CreateIoCompletionPort` requires in `FileHandle` when creating a new,
    // standalone port rather than associating an existing file/device
    // handle with one.
    let invalid_handle_value = usize::MAX as RawHandle;
    // SAFETY: `invalid_handle_value` is the documented sentinel above, not a
    // real handle being dereferenced; `existing_completion_port = NULL` and
    // `completion_key = 0` are the documented inputs for "create a new
    // port"; `number_of_concurrent_threads = 1` is a plain value (this
    // crate's callers poll `wait_for_message` from a single thread).
    let port = unsafe { CreateIoCompletionPort(invalid_handle_value, core::ptr::null_mut(), 0, 1) };
    if port.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(port)
    }
}

/// Associate `job` with `completion_port`: from this call on, the OS posts a
/// message to the port every time a member process is created or exits, or
/// the job empties out — the push counterpart to [`process_ids`]'s poll,
/// and this crate's closest analog to Unix `SIGCHLD` for job membership
/// changes. `completion_key` is an arbitrary caller-chosen value echoed back
/// by [`wait_for_message`] unchanged, letting a caller sharing one port
/// across multiple jobs tell which job a given message came from.
///
/// # Safety
///
/// `job` and `completion_port` must both be currently-open, valid handles
/// (`completion_port` from [`create_completion_port`]).
pub unsafe fn associate_completion_port(
    job: RawHandle,
    completion_port: RawHandle,
    completion_key: usize,
) -> Result<(), Win32Error> {
    let info = JobObjectAssociateCompletionPort {
        completion_key: completion_key as *mut core::ffi::c_void,
        completion_port,
    };
    // SAFETY: `job` and `completion_port` are caller-supplied per this
    // function's own safety contract; `info` is a valid, correctly-sized,
    // initialized struct of exactly the type
    // `JobObjectAssociateCompletionPortInformation` names.
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_ASSOCIATE_COMPLETION_PORT_CLASS,
            (&info as *const JobObjectAssociateCompletionPort).cast(),
            core::mem::size_of::<JobObjectAssociateCompletionPort>() as u32,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// One job-lifecycle notification — `message` is one of the
/// `JOB_OBJECT_MSG_*` constants (e.g. [`JOB_OBJECT_MSG_EXIT_PROCESS`]),
/// `pid` is the process it concerns (meaningful for the per-process
/// messages; `0` for job-wide ones like [`JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO`],
/// which carry no specific pid), and `completion_key` echoes back whatever
/// [`associate_completion_port`] was called with for the job this message
/// came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobMessage {
    pub completion_key: usize,
    pub message: u32,
    pub pid: u32,
}

/// Block on `completion_port` for up to `timeout_ms` (`None` = wait
/// forever), returning the next queued job-lifecycle message —
/// `GetQueuedCompletionStatus`. Returns `Some(message)` if one arrived
/// within the timeout, `None` on timeout — call with `Some(0)` for a
/// non-blocking poll, matching [`crate::process::wait`]'s own `Option<u32>`
/// timeout convention.
///
/// For job notifications specifically, Windows repurposes
/// `GetQueuedCompletionStatus`'s `lpOverlapped` out-parameter (ordinarily a
/// real `OVERLAPPED` pointer for file I/O) to instead carry the process ID
/// as a plain integer value — a documented quirk of this API being reused
/// for job notifications rather than its original file-I/O purpose, not
/// something this wrapper invents.
///
/// # Safety
///
/// `completion_port` must be a currently-open, valid I/O completion port
/// handle (from [`create_completion_port`]).
pub unsafe fn wait_for_message(
    completion_port: RawHandle,
    timeout_ms: Option<u32>,
) -> Result<Option<JobMessage>, Win32Error> {
    const INFINITE: u32 = 0xFFFF_FFFF;

    let mut bytes_transferred: u32 = 0;
    let mut completion_key: usize = 0;
    let mut overlapped: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: `completion_port` is caller-supplied per this function's own
    // safety contract; `bytes_transferred`/`completion_key`/`overlapped` are
    // valid out-pointers to correctly-typed locals.
    let ok = unsafe {
        GetQueuedCompletionStatus(
            completion_port,
            &mut bytes_transferred,
            &mut completion_key,
            &mut overlapped,
            timeout_ms.unwrap_or(INFINITE),
        )
    };
    if ok == 0 {
        let err = Win32Error::last();
        return if err.code() == WAIT_TIMEOUT {
            Ok(None)
        } else {
            Err(err)
        };
    }
    Ok(Some(JobMessage {
        completion_key,
        message: bytes_transferred,
        pid: overlapped as usize as u32,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process;

    #[test]
    fn create_terminate_and_kill_on_close_round_trip() {
        let job = create().expect("CreateJobObjectW should succeed");
        // SAFETY: `job` is freshly created and valid.
        unsafe { set_kill_on_close(job).expect("SetInformationJobObject should succeed") };

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known long-running system command — this test terminates it
        // via the job before it would exit on its own.
        let spawned = unsafe {
            process::spawn_suspended("cmd.exe /c ping -n 30 127.0.0.1 >nul", true, false, None)
        }
        .expect("CreateProcessW should succeed");

        // SAFETY: `job`/`spawned.process` are both freshly created, valid
        // handles; assignment happens before `resume`, so job membership is
        // guaranteed before the process runs.
        unsafe { assign(job, spawned.process).expect("AssignProcessToJobObject should succeed") };
        // SAFETY: `spawned.thread` is a freshly created, valid,
        // not-yet-resumed thread handle.
        unsafe { process::resume(spawned.thread).expect("ResumeThread should succeed") };

        // SAFETY: `job` is a valid handle with exactly one assigned process.
        let ids_before =
            unsafe { process_ids(job) }.expect("QueryInformationJobObject should succeed");
        assert_eq!(ids_before, alloc::vec![spawned.process_id]);

        // SAFETY: `job` is a valid handle; this is the operation under
        // test.
        unsafe { terminate(job, 1).expect("TerminateJobObject should succeed") };

        // SAFETY: `spawned.process` is still a valid handle — terminating
        // via the job doesn't invalidate the handle, only the process.
        let exit = unsafe { process::wait(spawned.process, Some(5_000)) }.unwrap();
        assert_eq!(exit, Some(1));

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
            crate::handle::close(job).unwrap();
        }
    }

    #[test]
    fn clear_kill_on_close_lets_the_process_survive_closing_the_job_handle() {
        let job = create().expect("CreateJobObjectW should succeed");
        // SAFETY: `job` is freshly created and valid.
        unsafe { set_kill_on_close(job).expect("SetInformationJobObject should succeed") };

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { process::spawn_suspended("cmd.exe /c exit 3", false, false, None) }
            .expect("CreateProcessW should succeed");
        // SAFETY: `job`/`spawned.process` are both freshly created, valid
        // handles; assignment happens before `resume`.
        unsafe { assign(job, spawned.process).expect("AssignProcessToJobObject should succeed") };
        // SAFETY: `spawned.thread` is a freshly created, valid,
        // not-yet-resumed thread handle.
        unsafe { process::resume(spawned.thread).expect("ResumeThread should succeed") };

        // SAFETY: `job` is a valid handle; this is the operation under
        // test — reversing kill-on-close before the handle closes below.
        unsafe { clear_kill_on_close(job).expect("SetInformationJobObject should succeed") };
        // SAFETY: `job` is a valid, currently-open handle, closed exactly
        // once. Without `clear_kill_on_close` above actually working,
        // this would terminate `spawned.process` immediately instead of
        // letting it run to its own natural exit.
        unsafe { crate::handle::close(job).unwrap() };

        // SAFETY: `spawned.process` is still a valid handle — closing the
        // job handle doesn't invalidate it, only (were kill-on-close still
        // set) the process it names.
        let exit = unsafe { process::wait(spawned.process, Some(5_000)) }.unwrap();
        assert_eq!(exit, Some(3));

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
        }
    }

    #[test]
    fn is_in_job_reports_true_only_after_assign() {
        let job = create().expect("CreateJobObjectW should succeed");
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { process::spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");

        // Checked against this specific job (not `None`/"any job") so this
        // assertion holds even under an ambient job wrapping the whole test
        // process tree (e.g. GitHub Actions' Windows runners — see this
        // function's own doc comment).
        // SAFETY: `spawned.process`/`job` are both freshly created, valid
        // handles.
        let before = unsafe { is_in_job(spawned.process, Some(job)) }
            .expect("IsProcessInJob should succeed");
        assert!(
            !before,
            "a process shouldn't be in a job it hasn't been assigned to yet"
        );

        // SAFETY: `job`/`spawned.process` are both freshly created, valid
        // handles; assignment happens before `resume`.
        unsafe { assign(job, spawned.process).expect("AssignProcessToJobObject should succeed") };

        // SAFETY: same handles, still valid.
        let after = unsafe { is_in_job(spawned.process, Some(job)) }
            .expect("IsProcessInJob should succeed");
        assert!(
            after,
            "the process should report membership right after assign"
        );

        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { process::resume(spawned.thread).expect("ResumeThread should succeed") };
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { process::wait(spawned.process, None) }.unwrap();

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
            crate::handle::close(job).unwrap();
        }
    }

    #[test]
    fn associate_completion_port_reports_process_exit() {
        let job = create().expect("CreateJobObjectW should succeed");
        let port = create_completion_port().expect("CreateIoCompletionPort should succeed");
        // SAFETY: both handles are freshly created and valid.
        unsafe { associate_completion_port(job, port, 42) }
            .expect("SetInformationJobObject should succeed");

        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { process::spawn_suspended("cmd.exe /c exit 5", false, false, None) }
            .expect("CreateProcessW should succeed");
        // SAFETY: `job`/`spawned.process` are both freshly created, valid
        // handles; assignment happens before `resume`.
        unsafe { assign(job, spawned.process) }.expect("AssignProcessToJobObject should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { process::resume(spawned.thread) }.expect("ResumeThread should succeed");

        // The OS posts several messages across this one process's lifecycle
        // (at least NEW_PROCESS and EXIT_PROCESS, usually followed by
        // ACTIVE_PROCESS_ZERO once the job empties) — loop until the
        // specific one this test cares about arrives, bounded so a real
        // regression fails outright instead of hanging.
        let mut saw_exit = false;
        for _ in 0..10 {
            // SAFETY: `port` is a valid, currently-open completion port
            // handle.
            let msg = unsafe { wait_for_message(port, Some(5_000)) }
                .expect("GetQueuedCompletionStatus should succeed")
                .expect("a message should arrive well within the timeout");
            assert_eq!(
                msg.completion_key, 42,
                "completion_key should echo back what associate_completion_port was called with"
            );
            if msg.message == JOB_OBJECT_MSG_EXIT_PROCESS {
                assert_eq!(msg.pid, spawned.process_id);
                saw_exit = true;
                break;
            }
        }
        assert!(saw_exit, "should have observed JOB_OBJECT_MSG_EXIT_PROCESS");

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
            crate::handle::close(job).unwrap();
            crate::handle::close(port).unwrap();
        }
    }

    #[test]
    fn wait_for_message_times_out_with_nothing_queued() {
        let port = create_completion_port().expect("CreateIoCompletionPort should succeed");
        // SAFETY: `port` is freshly created and valid.
        let result = unsafe { wait_for_message(port, Some(0)) }
            .expect("GetQueuedCompletionStatus should succeed");
        assert_eq!(result, None);
        // SAFETY: `port` is a valid, currently-open handle, closed exactly
        // once.
        unsafe { crate::handle::close(port).unwrap() };
    }

    #[test]
    fn set_resource_limits_round_trips_through_a_query() {
        let job = create().expect("CreateJobObjectW should succeed");
        let limits_in = JobLimits {
            active_process_limit: Some(4),
            process_memory_limit: Some(64 * 1024 * 1024),
            job_memory_limit: Some(256 * 1024 * 1024),
            per_process_user_time_limit: Some(Timespec { secs: 30, nanos: 0 }),
            per_job_user_time_limit: Some(Timespec { secs: 60, nanos: 0 }),
        };
        // SAFETY: `job` is freshly created and valid.
        unsafe { set_resource_limits(job, limits_in) }
            .expect("SetInformationJobObject should succeed");

        // SAFETY: `job` is the same valid handle.
        let limits_out = unsafe { limits(job) }.expect("QueryInformationJobObject should succeed");
        assert_eq!(
            limits_out, limits_in,
            "every limit set should read back exactly as set"
        );

        // SAFETY: `job` is a valid, currently-open handle, closed exactly
        // once.
        unsafe { crate::handle::close(job).unwrap() };
    }

    #[test]
    fn limits_reports_none_for_anything_never_set() {
        let job = create().expect("CreateJobObjectW should succeed");
        // SAFETY: `job` is freshly created and valid; nothing has set any
        // limit on it yet.
        let limits_out = unsafe { limits(job) }.expect("QueryInformationJobObject should succeed");
        assert_eq!(
            limits_out,
            JobLimits::default(),
            "a fresh job should report every limit unset"
        );
        // SAFETY: `job` is a valid, currently-open handle, closed exactly
        // once.
        unsafe { crate::handle::close(job).unwrap() };
    }

    #[test]
    fn accounting_reports_process_counts_after_a_process_exits() {
        let job = create().expect("CreateJobObjectW should succeed");
        // SAFETY: a hand-built, correctly quoted command line for a
        // well-known system binary.
        let spawned = unsafe { process::spawn_suspended("cmd.exe /c exit 0", false, false, None) }
            .expect("CreateProcessW should succeed");
        // SAFETY: `job`/`spawned.process` are both freshly created, valid
        // handles; assignment happens before `resume`.
        unsafe { assign(job, spawned.process) }.expect("AssignProcessToJobObject should succeed");
        // SAFETY: `spawned.thread` is freshly created, valid, not yet
        // resumed.
        unsafe { process::resume(spawned.thread) }.expect("ResumeThread should succeed");
        // SAFETY: `spawned.process` is a valid, currently-open handle.
        unsafe { process::wait(spawned.process, None) }.unwrap();

        // The process handle becoming signaled doesn't guarantee the job
        // object's own `active_processes` bookkeeping has already been
        // decremented — the two aren't documented as updated atomically
        // with each other, and this raced under CI. Poll with a bounded
        // retry instead of asserting on the very first read.
        let mut acc = unsafe { accounting(job) }.expect("QueryInformationJobObject should succeed");
        for _ in 0..20 {
            if acc.active_processes == 0 {
                break;
            }
            process::sleep_ms(50);
            // SAFETY: `job` is still a valid handle.
            acc = unsafe { accounting(job) }.expect("QueryInformationJobObject should succeed");
        }
        assert!(
            acc.total_processes >= 1,
            "at least one process was assigned to this job"
        );
        assert_eq!(
            acc.active_processes, 0,
            "the process has already exited, so none should remain active"
        );
        assert!(acc.total_user_time.nanos < 1_000_000_000);
        assert!(acc.total_kernel_time.nanos < 1_000_000_000);

        // SAFETY: every handle here is valid and each closed exactly once.
        unsafe {
            crate::handle::close(spawned.process).unwrap();
            crate::handle::close(spawned.thread).unwrap();
            crate::handle::close(job).unwrap();
        }
    }

    #[test]
    fn open_by_name_opens_the_same_job_a_named_create_made() {
        let name = "rusty_win32_test_job_open_by_name";
        let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
        // `create()` only ever makes anonymous jobs — call the private
        // `CreateJobObjectW` extern directly (this test module can, being
        // inside `job.rs` itself) to get a *named* job to open by name.
        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
        // `job_attributes = NULL` requests default security attributes, a
        // documented valid input.
        let original = unsafe { CreateJobObjectW(core::ptr::null(), wide.as_ptr()) };
        assert!(
            !original.is_null(),
            "CreateJobObjectW should succeed creating a named job"
        );

        let opened = open_by_name(name, JOB_OBJECT_ALL_ACCESS)
            .expect("OpenJobObjectW should succeed for a job this test itself just created");

        // SAFETY: both handles are valid, currently-open handles.
        let same = unsafe { crate::handle::same_object(original, opened) }
            .expect("CompareObjectHandles should succeed");
        assert!(
            same,
            "open_by_name should return a handle to the same job object CreateJobObjectW made"
        );

        // SAFETY: both handles are valid and each closed exactly once.
        unsafe {
            crate::handle::close(original).unwrap();
            crate::handle::close(opened).unwrap();
        }
    }
}
