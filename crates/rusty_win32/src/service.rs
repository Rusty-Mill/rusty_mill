//! Windows service control — `winsvc.h`, a new module added in round 2,
//! previously excluded by `ARCHITECTURE.md`'s non-goals (see
//! `docs/archive/gap-analysis.md`'s, now closed out and archived, "Round
//! 2: previously out-of-scope subsystems"
//! sweep), now in scope per explicit round-2 direction.
//!
//! Scope: a `systemctl`-equivalent — list/query/start/stop a named
//! service. Installing/removing services (`CreateServiceW`/
//! `DeleteService`), driver services, and service-controller-side
//! (writing a service's own `main`) support are all explicitly out of
//! scope.
//!
//! This first piece is the SCM/service handle lifecycle. An `SC_HANDLE`
//! is reused as this crate's existing [`crate::handle::RawHandle`]
//! (`*mut c_void`) rather than a distinct type — it's ABI-compatible
//! (`DECLARE_HANDLE`-based, exactly like `HANDLE` itself), and every
//! function in this module accepts/returns it, so a separate wrapper
//! type would add nothing; its destructor is `CloseServiceHandle`,
//! never `CloseHandle`, so it must never be passed to
//! [`crate::handle`]'s own close functions.

extern crate alloc;
use crate::error::Win32Error;
use crate::handle::RawHandle;
use alloc::vec::Vec;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenSCManagerW(
        machine_name: *const u16,
        database_name: *const u16,
        desired_access: u32,
    ) -> RawHandle;
    fn OpenServiceW(scm: RawHandle, service_name: *const u16, desired_access: u32) -> RawHandle;
    fn CloseServiceHandle(handle: RawHandle) -> i32;
    fn StartServiceW(
        handle: RawHandle,
        num_service_args: u32,
        service_arg_vectors: *const *const u16,
    ) -> i32;
    fn ControlService(
        handle: RawHandle,
        control: u32,
        service_status: *mut ServiceStatusRaw,
    ) -> i32;
    fn QueryServiceStatusEx(
        handle: RawHandle,
        info_level: u32,
        buffer: *mut u8,
        buf_size: u32,
        bytes_needed: *mut u32,
    ) -> i32;
    fn EnumServicesStatusExW(
        scm: RawHandle,
        info_level: u32,
        service_type: u32,
        service_state: u32,
        services: *mut u8,
        buf_size: u32,
        bytes_needed: *mut u32,
        services_returned: *mut u32,
        resume_handle: *mut u32,
        group_name: *const u16,
    ) -> i32;
    fn QueryServiceConfigW(
        handle: RawHandle,
        service_config: *mut u8,
        buf_size: u32,
        bytes_needed: *mut u32,
    ) -> i32;
    fn GetServiceDisplayNameW(
        scm: RawHandle,
        service_name: *const u16,
        display_name: *mut u16,
        buf_len: *mut u32,
    ) -> i32;
    fn GetServiceKeyNameW(
        scm: RawHandle,
        display_name: *const u16,
        service_name: *mut u16,
        buf_len: *mut u32,
    ) -> i32;
    fn EnumDependentServicesW(
        handle: RawHandle,
        service_state: u32,
        services: *mut u8,
        buf_size: u32,
        bytes_needed: *mut u32,
        services_returned: *mut u32,
    ) -> i32;
}

/// `SC_MANAGER_CONNECT` — the minimal access right needed to open a
/// connection to the SCM at all. Verified against mingw-w64's own
/// `winsvc.h` with a compiled `_Static_assert` probe.
pub const SC_MANAGER_CONNECT: u32 = 0x0001;

/// `SC_MANAGER_ENUMERATE_SERVICE` — the access right [`enum_services`]
/// needs in addition to [`SC_MANAGER_CONNECT`]. Verified against
/// mingw-w64's own `winsvc.h` with a compiled `_Static_assert` probe.
pub const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x0004;

/// `SERVICE_QUERY_CONFIG`. Verified against mingw-w64's own `winsvc.h`
/// with a compiled `_Static_assert` probe.
pub const SERVICE_QUERY_CONFIG: u32 = 0x0001;

/// `SERVICE_QUERY_STATUS`. Verified against mingw-w64's own `winsvc.h`
/// with a compiled `_Static_assert` probe.
pub const SERVICE_QUERY_STATUS: u32 = 0x0004;

/// `SERVICE_START` — the access right [`start`] needs. Verified against
/// mingw-w64's own `winsvc.h` with a compiled `_Static_assert` probe.
pub const SERVICE_START: u32 = 0x0010;

/// `SERVICE_INTERROGATE` — the access right [`control`] needs to issue
/// [`ServiceControl::Interrogate`] (a different, narrower right than
/// stop/pause/continue would need, since interrogating doesn't change
/// the service's state). Verified against mingw-w64's own `winsvc.h`
/// with a compiled `_Static_assert` probe.
pub const SERVICE_INTERROGATE: u32 = 0x0080;

/// `SERVICE_ENUMERATE_DEPENDENTS` — the access right [`dependent_services`]
/// needs. Verified against mingw-w64's own `winsvc.h` with a compiled
/// `_Static_assert` probe.
pub const SERVICE_ENUMERATE_DEPENDENTS: u32 = 0x0008;

/// Connect to the local Service Control Manager — `OpenSCManagerW`, the
/// entry point every other function in this module needs (directly, or
/// via a handle [`open_service`] opened from one). Always connects to
/// the local machine's active services database — this module has no
/// remote-machine or alternate-database (`SERVICES_FAILED_DATABASE`)
/// support.
pub fn open_manager(access: u32) -> Result<RawHandle, Win32Error> {
    // SAFETY: a null machine/database name is documented to mean "the
    // local machine's active services database"; `access` is a plain
    // access-rights bitmask, not a pointer.
    let handle = unsafe { OpenSCManagerW(core::ptr::null(), core::ptr::null(), access) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Open a handle to the service named `name` — `OpenServiceW`, the
/// `service` module's equivalent of [`crate::process::open_by_pid`]:
/// turning a name a caller only knows as a string into a handle real
/// operations (status/start/stop) can be performed on.
///
/// # Safety
///
/// `scm` must be a currently-open, valid SCM handle from [`open_manager`]
/// with at least [`SC_MANAGER_CONNECT`].
pub unsafe fn open_service(
    scm: RawHandle,
    name: &str,
    access: u32,
) -> Result<RawHandle, Win32Error> {
    let wide: Vec<u16> = crate::wide::to_wide(name);
    // SAFETY: `scm` is caller-supplied per this function's own safety
    // contract; `wide` is a valid, NUL-terminated UTF-16 string live for
    // the whole call.
    let handle = unsafe { OpenServiceW(scm, wide.as_ptr(), access) };
    if handle.is_null() {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Close a handle opened by [`open_manager`] or [`open_service`] —
/// `CloseServiceHandle`. Never [`crate::handle`]'s own close function:
/// an `SC_HANDLE`'s destructor is always this one, never `CloseHandle`.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle from [`open_manager`]
/// or [`open_service`], not already closed.
pub unsafe fn close(handle: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract.
    let ok = unsafe { CloseServiceHandle(handle) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Start an already-installed service — `StartServiceW`, the
/// zero-argument case only (`lpServiceArgVectors` only matters for
/// driver-style services, out of this module's scope).
///
/// # Safety
///
/// `handle` must be a currently-open, valid service handle from
/// [`open_service`] with at least [`SERVICE_START`].
pub unsafe fn start(handle: RawHandle) -> Result<(), Win32Error> {
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `0`/null is documented as "no service-specific
    // arguments," the only case this module supports.
    let ok = unsafe { StartServiceW(handle, 0, core::ptr::null()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

// SERVICE_STATUS: `size_of` 28 — seven plain `DWORD`s, no padding, no
// trailing process id (the older, pid-less struct `QueryServiceStatusEx`
// supersedes — see `status`'s own doc comment). Verified against
// mingw-w64's own `winsvc.h` with a compiled `_Static_assert` probe. Not
// exposed publicly: `control`'s own doc comment explains why the
// immediate status this struct carries is discarded rather than
// returned.
#[repr(C)]
#[derive(Clone, Copy)]
struct ServiceStatusRaw {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    check_point: u32,
    wait_hint: u32,
}
const _: () = assert!(core::mem::size_of::<ServiceStatusRaw>() == 28);

/// `SERVICE_CONTROL_*` — the four ordinary controls [`control`] can send
/// (out of the many `ControlService` itself supports: `SHUTDOWN`/
/// `PARAMCHANGE`/`NETBINDADD`/… are service-controller-internal signals,
/// out of this module's scope). Verified against mingw-w64's own
/// `winsvc.h` with a compiled `_Static_assert` probe.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceControl {
    Stop = 1,
    Pause = 2,
    Continue = 3,
    Interrogate = 4,
}

/// Send a stop/pause/continue/interrogate control to a running service —
/// `ControlService`. Doesn't itself return the service's resulting
/// status: `ControlService`'s own `lpServiceStatus` out-parameter only
/// reports the state at the instant of the call (often still
/// `SERVICE_STOP_PENDING`/`SERVICE_PAUSE_PENDING`/etc., not yet
/// settled), so this crate discards it and expects a caller to poll
/// [`status`] afterward to see the state actually settle — the same
/// poll-don't-block shape [`crate::job::process_ids`] already uses,
/// rather than this crate inventing its own wait-until-settled loop with
/// its own timeout/backoff policy.
///
/// # Safety
///
/// `handle` must be a currently-open, valid service handle from
/// [`open_service`] with the access right [`control`] needs for
/// `control` (e.g. [`SERVICE_INTERROGATE`] for
/// [`ServiceControl::Interrogate`]).
pub unsafe fn control(handle: RawHandle, control: ServiceControl) -> Result<(), Win32Error> {
    let mut service_status = core::mem::MaybeUninit::<ServiceStatusRaw>::uninit();
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `service_status` is a valid, correctly-sized out-pointer
    // `ControlService` requires even though this function discards its
    // contents.
    let ok = unsafe { ControlService(handle, control as u32, service_status.as_mut_ptr()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// `SC_ENUM_PROCESS_INFO` — `EnumServicesStatusExW`'s only defined
/// `SC_ENUM_TYPE` value (the `InfoLevel` parameter always takes this;
/// Windows has never defined another). Verified against mingw-w64's own
/// `winsvc.h` with a compiled `_Static_assert` probe.
const SC_ENUM_PROCESS_INFO: u32 = 0;

/// `SERVICE_WIN32` — ordinary user-mode services (`SERVICE_WIN32_OWN_PROCESS`
/// `|` `SERVICE_WIN32_SHARE_PROCESS`), this module's only supported
/// [`enum_services`] `dwServiceType` value; driver services
/// (`SERVICE_DRIVER`) are out of this module's scope. Verified against
/// mingw-w64's own `winnt.h` with a compiled `_Static_assert` probe.
pub const SERVICE_WIN32: u32 = 0x30;

/// `SERVICE_ACTIVE`. Verified against mingw-w64's own `winsvc.h` with a
/// compiled `_Static_assert` probe.
pub const SERVICE_ACTIVE: u32 = 0x0000_0001;

/// `SERVICE_INACTIVE`. Verified against mingw-w64's own `winsvc.h` with a
/// compiled `_Static_assert` probe.
pub const SERVICE_INACTIVE: u32 = 0x0000_0002;

/// `SERVICE_STATE_ALL` (`SERVICE_ACTIVE` `|` `SERVICE_INACTIVE`) —
/// [`enum_services`]'s default `dwServiceState` value. Verified against
/// mingw-w64's own `winsvc.h` with a compiled `_Static_assert` probe.
pub const SERVICE_STATE_ALL: u32 = SERVICE_ACTIVE | SERVICE_INACTIVE;

/// `SERVICE_STATUS_PROCESS.dwCurrentState`'s seven possible values,
/// exposed raw and policy-free (matching this crate's existing
/// convention for other bitmask/status fields) rather than as an enum —
/// [`ServiceStatus::current_state`]/[`ServiceStatusEntry::current_state`]
/// carry one of these. Verified against mingw-w64's own `winsvc.h` with a
/// compiled `_Static_assert` probe.
pub const SERVICE_STOPPED: u32 = 0x0000_0001;
pub const SERVICE_START_PENDING: u32 = 0x0000_0002;
pub const SERVICE_STOP_PENDING: u32 = 0x0000_0003;
pub const SERVICE_RUNNING: u32 = 0x0000_0004;
pub const SERVICE_CONTINUE_PENDING: u32 = 0x0000_0005;
pub const SERVICE_PAUSE_PENDING: u32 = 0x0000_0006;
pub const SERVICE_PAUSED: u32 = 0x0000_0007;

// SERVICE_STATUS_PROCESS: `size_of` 36 — nine plain `DWORD`s, no padding.
// Verified against mingw-w64's own `winsvc.h` with a compiled
// `_Static_assert` probe.
#[repr(C)]
#[derive(Clone, Copy)]
struct ServiceStatusProcess {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    check_point: u32,
    wait_hint: u32,
    process_id: u32,
    service_flags: u32,
}
const _: () = assert!(core::mem::size_of::<ServiceStatusProcess>() == 36);

// ENUM_SERVICE_STATUS_PROCESSW: `size_of` 56, `ServiceStatusProcess` at
// offset 16 — two pointers plus the fixed status block above. Verified
// against mingw-w64's own `winsvc.h` with a compiled `_Static_assert`
// probe. `lpServiceName`/`lpDisplayName` point into the same buffer
// `EnumServicesStatusExW` filled, not separately owned/freed — this
// crate copies them out into owned `String`s before the buffer is
// dropped.
#[repr(C)]
#[derive(Clone, Copy)]
struct EnumServiceStatusProcessW {
    service_name: *mut u16,
    display_name: *mut u16,
    service_status_process: ServiceStatusProcess,
}
const _: () = assert!(core::mem::size_of::<EnumServiceStatusProcessW>() == 56);
const _: () =
    assert!(core::mem::offset_of!(EnumServiceStatusProcessW, service_status_process) == 16);

/// One service's name, display name, and current status, as returned by
/// [`enum_services`] — the core of a `systemctl list-units`-equivalent.
/// Every field beyond the two names is copied straight out of
/// `SERVICE_STATUS_PROCESS`, exposed raw and policy-free, matching this
/// crate's existing convention for other bitmask/status fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatusEntry {
    pub service_name: alloc::string::String,
    pub display_name: alloc::string::String,
    pub service_type: u32,
    pub current_state: u32,
    pub controls_accepted: u32,
    pub win32_exit_code: u32,
    pub service_specific_exit_code: u32,
    pub check_point: u32,
    pub wait_hint: u32,
    pub process_id: u32,
    pub service_flags: u32,
}

/// `SC_STATUS_PROCESS_INFO` — `QueryServiceStatusEx`'s only defined
/// `SC_STATUS_TYPE` value (the `InfoLevel` parameter always takes this;
/// Windows has never defined another). Verified against mingw-w64's own
/// `winsvc.h` with a compiled `_Static_assert` probe.
const SC_STATUS_PROCESS_INFO: u32 = 0;

/// One service's live status, as returned by [`status`] — the same
/// fields [`ServiceStatusEntry`] carries, minus the two names (a caller
/// querying a single service by handle already knows which one it is).
/// Includes the backing process id ([`ServiceStatus::process_id`]),
/// superseding the older, pid-less `QueryServiceStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceStatus {
    pub service_type: u32,
    pub current_state: u32,
    pub controls_accepted: u32,
    pub win32_exit_code: u32,
    pub service_specific_exit_code: u32,
    pub check_point: u32,
    pub wait_hint: u32,
    pub process_id: u32,
    pub service_flags: u32,
}

/// Query `handle`'s live status — `QueryServiceStatusEx`
/// (`SC_STATUS_PROCESS_INFO`), including the backing process id,
/// superseding the older, pid-less `QueryServiceStatus`.
///
/// # Safety
///
/// `handle` must be a currently-open, valid service handle from
/// [`open_service`] with at least [`SERVICE_QUERY_STATUS`].
pub unsafe fn status(handle: RawHandle) -> Result<ServiceStatus, Win32Error> {
    let mut raw = core::mem::MaybeUninit::<ServiceStatusProcess>::uninit();
    let mut bytes_needed: u32 = 0;
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `raw` is a valid, exactly `SERVICE_STATUS_PROCESS`-sized
    // out-buffer, matched by the `size_of` passed as `cbBufSize`;
    // `bytes_needed` is a valid out-pointer.
    let ok = unsafe {
        QueryServiceStatusEx(
            handle,
            SC_STATUS_PROCESS_INFO,
            raw.as_mut_ptr().cast(),
            core::mem::size_of::<ServiceStatusProcess>() as u32,
            &mut bytes_needed,
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    // SAFETY: a successful call with a buffer already sized to exactly
    // `size_of::<ServiceStatusProcess>()` guarantees the whole struct was
    // written.
    let raw = unsafe { raw.assume_init() };
    Ok(ServiceStatus {
        service_type: raw.service_type,
        current_state: raw.current_state,
        controls_accepted: raw.controls_accepted,
        win32_exit_code: raw.win32_exit_code,
        service_specific_exit_code: raw.service_specific_exit_code,
        check_point: raw.check_point,
        wait_hint: raw.wait_hint,
        process_id: raw.process_id,
        service_flags: raw.service_flags,
    })
}

/// Decode a NUL-terminated wide string pointed to by `ptr` — used for
/// [`enum_services`]'s name/display-name fields, which point into a
/// buffer this crate itself allocated and doesn't guarantee any
/// particular alignment for beyond `u8`.
///
/// # Safety
///
/// `ptr`, if non-null, must point to a NUL-terminated, readable sequence
/// of `u16`s.
unsafe fn decode_wide_cstr(ptr: *const u16) -> alloc::string::String {
    if ptr.is_null() {
        return alloc::string::String::new();
    }
    let mut len = 0usize;
    // SAFETY: `ptr` is caller-supplied per this function's own safety
    // contract; `read_unaligned` doesn't require `ptr` (or `ptr.add(len)`)
    // to be 2-byte aligned.
    while unsafe { core::ptr::read_unaligned(ptr.add(len)) } != 0 {
        len += 1;
    }
    let mut units = Vec::with_capacity(len);
    for i in 0..len {
        // SAFETY: same contract as above; `i` is within `0..len`, the
        // range just walked to find the terminating NUL.
        units.push(unsafe { core::ptr::read_unaligned(ptr.add(i)) });
    }
    alloc::string::String::from_utf16_lossy(&units)
}

/// List every service known to the SCM with its current status —
/// `EnumServicesStatusExW`, the core of a `systemctl list-units`-
/// equivalent. Pages internally via the resume-handle protocol
/// `EnumServicesStatusExW` itself documents (growing the buffer and
/// retrying the same page on `ERROR_INSUFFICIENT_BUFFER`/
/// `ERROR_MORE_DATA` with zero entries returned; otherwise processing
/// whatever entries came back and continuing until the call finally
/// succeeds) until the whole database has been walked.
///
/// # Safety
///
/// `scm` must be a currently-open, valid SCM handle from
/// [`open_manager`] with at least [`SC_MANAGER_CONNECT`] `|`
/// [`SC_MANAGER_ENUMERATE_SERVICE`].
pub unsafe fn enum_services(scm: RawHandle) -> Result<Vec<ServiceStatusEntry>, Win32Error> {
    let mut entries = Vec::new();
    let mut resume_handle: u32 = 0;
    let mut buf_len: u32 = 16 * 1024;
    loop {
        let mut buf: Vec<u8> = alloc::vec![0u8; buf_len as usize];
        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        // SAFETY: `scm` is caller-supplied per this function's own safety
        // contract; `buf` is a valid buffer matched by `buf_len` naming
        // its exact size; `bytes_needed`/`services_returned`/
        // `resume_handle` are valid in/out-pointers; a null `group_name`
        // means "don't filter by group".
        let ok = unsafe {
            EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                buf.as_mut_ptr(),
                buf_len,
                &mut bytes_needed,
                &mut services_returned,
                &mut resume_handle,
                core::ptr::null(),
            )
        };
        if ok == 0 {
            let err = Win32Error::last();
            if err != Win32Error::ERROR_MORE_DATA {
                return Err(err);
            }
            if services_returned == 0 {
                // The buffer couldn't fit even one entry this page --
                // grow to the reported requirement and retry the exact
                // same page (the resume handle is untouched when nothing
                // was consumed).
                buf_len = bytes_needed.max(buf_len * 2);
                continue;
            }
        }

        // SAFETY: `EnumServicesStatusExW` guarantees `services_returned`
        // fixed-size `EnumServiceStatusProcessW` records packed
        // contiguously starting at `buf`'s own start, whether this call
        // returned success or a partial-page `ERROR_MORE_DATA`.
        let mut ptr = buf.as_ptr();
        for _ in 0..services_returned {
            let record: EnumServiceStatusProcessW =
                unsafe { core::ptr::read_unaligned(ptr.cast()) };
            entries.push(ServiceStatusEntry {
                // SAFETY: `record.service_name`/`record.display_name`
                // point into `buf`, which is still alive for this whole
                // loop body.
                service_name: unsafe { decode_wide_cstr(record.service_name) },
                display_name: unsafe { decode_wide_cstr(record.display_name) },
                service_type: record.service_status_process.service_type,
                current_state: record.service_status_process.current_state,
                controls_accepted: record.service_status_process.controls_accepted,
                win32_exit_code: record.service_status_process.win32_exit_code,
                service_specific_exit_code: record
                    .service_status_process
                    .service_specific_exit_code,
                check_point: record.service_status_process.check_point,
                wait_hint: record.service_status_process.wait_hint,
                process_id: record.service_status_process.process_id,
                service_flags: record.service_status_process.service_flags,
            });
            ptr = unsafe { ptr.add(core::mem::size_of::<EnumServiceStatusProcessW>()) };
        }

        if ok != 0 {
            // A successful return means the whole database has now been
            // walked -- no more pages remain.
            break;
        }
    }
    Ok(entries)
}

// QUERY_SERVICE_CONFIGW: `size_of` 64 — three `DWORD`s, two `LPWSTR`s,
// one `DWORD`, three more `LPWSTR`s, in that exact order (verified
// field-by-field against mingw-w64's own `winsvc.h` with a compiled
// `_Static_assert` probe). `lpBinaryPathName`/`lpLoadOrderGroup`/
// `lpDependencies`/`lpServiceStartName`/`lpDisplayName` all point into
// the same buffer `QueryServiceConfigW` filled, not separately
// owned/freed -- this crate copies them out into owned `String`s/
// `Vec<String>` before the buffer is dropped, the same treatment
// `enum_services`'s `EnumServiceStatusProcessW` gets.
#[repr(C)]
#[derive(Clone, Copy)]
struct QueryServiceConfigRaw {
    service_type: u32,
    start_type: u32,
    error_control: u32,
    binary_path_name: *mut u16,
    load_order_group: *mut u16,
    tag_id: u32,
    dependencies: *mut u16,
    service_start_name: *mut u16,
    display_name: *mut u16,
}
const _: () = assert!(core::mem::size_of::<QueryServiceConfigRaw>() == 64);

/// One service's static configuration, as returned by [`config`] — start
/// type, binary path, display name, and the rest, for a `systemctl
/// show`-style detail view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    pub service_type: u32,
    pub start_type: u32,
    pub error_control: u32,
    pub binary_path_name: alloc::string::String,
    pub load_order_group: alloc::string::String,
    pub tag_id: u32,
    /// The names of every service this one depends on — decoded from
    /// `lpDependencies`'s `REG_MULTI_SZ`-shaped double-NUL-terminated
    /// list (an empty inner string ends the list), the same encoding
    /// [`crate::registry`]'s own `RegistryValue::MultiSz` decodes.
    /// Empty if the service has no dependencies.
    pub dependencies: Vec<alloc::string::String>,
    pub service_start_name: alloc::string::String,
    pub display_name: alloc::string::String,
}

/// Decode a `REG_MULTI_SZ`-shaped double-NUL-terminated list of wide
/// strings pointed to by `ptr` — used for [`config`]'s `dependencies`
/// field, which (like [`decode_wide_cstr`]'s targets) points into a
/// buffer this crate itself allocated and doesn't guarantee any
/// particular alignment for beyond `u8`.
///
/// # Safety
///
/// `ptr`, if non-null, must point to a valid `REG_MULTI_SZ`-shaped
/// sequence: zero or more NUL-terminated strings, the whole sequence
/// itself ended by an extra (empty-string) NUL.
unsafe fn decode_wide_multi_sz(mut ptr: *const u16) -> Vec<alloc::string::String> {
    let mut result = Vec::new();
    if ptr.is_null() {
        return result;
    }
    loop {
        let mut len = 0usize;
        // SAFETY: `ptr` is caller-supplied per this function's own safety
        // contract; `read_unaligned` doesn't require 2-byte alignment.
        while unsafe { core::ptr::read_unaligned(ptr.add(len)) } != 0 {
            len += 1;
        }
        if len == 0 {
            break;
        }
        let mut units = Vec::with_capacity(len);
        for i in 0..len {
            // SAFETY: same contract as above; `i` is within `0..len`,
            // the range just walked to find this entry's terminating
            // NUL.
            units.push(unsafe { core::ptr::read_unaligned(ptr.add(i)) });
        }
        result.push(alloc::string::String::from_utf16_lossy(&units));
        // SAFETY: `ptr.add(len)` is the NUL just walked to; advancing one
        // more past it moves to the next entry (or the list's own
        // terminating empty string).
        ptr = unsafe { ptr.add(len + 1) };
    }
    result
}

/// Query `handle`'s static configuration — `QueryServiceConfigW`,
/// growing the buffer on `ERROR_INSUFFICIENT_BUFFER` (the query-size-
/// then-allocate idiom this crate already uses for
/// [`crate::security::lookup_account_sid`]/[`crate::registry::query_value`],
/// here needing only one retry since `QueryServiceConfigW` reports the
/// exact size needed up front rather than paging like
/// [`enum_services`]).
///
/// # Safety
///
/// `handle` must be a currently-open, valid service handle from
/// [`open_service`] with at least [`SERVICE_QUERY_CONFIG`].
pub unsafe fn config(handle: RawHandle) -> Result<ServiceConfig, Win32Error> {
    let mut buf_len: u32 = 8 * 1024;
    loop {
        let mut buf: Vec<u8> = alloc::vec![0u8; buf_len as usize];
        let mut bytes_needed: u32 = 0;
        // SAFETY: `handle` is caller-supplied per this function's own
        // safety contract; `buf` is a valid buffer matched by `buf_len`
        // naming its exact size; `bytes_needed` is a valid out-pointer.
        let ok =
            unsafe { QueryServiceConfigW(handle, buf.as_mut_ptr(), buf_len, &mut bytes_needed) };
        if ok == 0 {
            let err = Win32Error::last();
            if err == Win32Error::ERROR_INSUFFICIENT_BUFFER {
                buf_len = bytes_needed;
                continue;
            }
            return Err(err);
        }

        // SAFETY: a successful call guarantees `buf` starts with a fully
        // populated `QUERY_SERVICE_CONFIGW` record.
        let record: QueryServiceConfigRaw =
            unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };
        return Ok(ServiceConfig {
            service_type: record.service_type,
            start_type: record.start_type,
            error_control: record.error_control,
            // SAFETY: every pointer field below points into `buf`, which
            // is still alive for this whole match arm.
            binary_path_name: unsafe { decode_wide_cstr(record.binary_path_name) },
            load_order_group: unsafe { decode_wide_cstr(record.load_order_group) },
            tag_id: record.tag_id,
            dependencies: unsafe { decode_wide_multi_sz(record.dependencies) },
            service_start_name: unsafe { decode_wide_cstr(record.service_start_name) },
            display_name: unsafe { decode_wide_cstr(record.display_name) },
        });
    }
}

/// The number of times [`display_name`]/[`key_name`] retry after
/// `ERROR_INSUFFICIENT_BUFFER` with a larger buffer than the size
/// `GetServiceDisplayNameW`/`GetServiceKeyNameW` themselves reported —
/// both functions have a real, documented-elsewhere Windows quirk where
/// the reported required size is sometimes still one character short of
/// what the very next call actually needs, so this crate grows past the
/// reported size rather than trusting it exactly. Bounded rather than an
/// unconditional loop, in case a future Windows version's behavior
/// changes and this stops converging.
const NAME_QUERY_MAX_RETRIES: u32 = 4;

/// Translate a service's short key name (`"eventlog"`) to its
/// human-readable display name (`"Windows Event Log"`) —
/// `GetServiceDisplayNameW`. The reverse direction of [`key_name`].
///
/// Grows the buffer past whatever size `GetServiceDisplayNameW` itself
/// reports on `ERROR_INSUFFICIENT_BUFFER` — see
/// `NAME_QUERY_MAX_RETRIES`'s own doc comment for why one extra
/// character of headroom isn't always enough in practice.
///
/// # Safety
///
/// `scm` must be a currently-open, valid SCM handle from
/// [`open_manager`] with at least [`SC_MANAGER_CONNECT`].
pub unsafe fn display_name(
    scm: RawHandle,
    key_name: &str,
) -> Result<alloc::string::String, Win32Error> {
    let wide_key: Vec<u16> = crate::wide::to_wide(key_name);
    let mut buf_len: u32 = 0;
    for _ in 0..=NAME_QUERY_MAX_RETRIES {
        let mut buf: Vec<u16> = alloc::vec![0u16; buf_len as usize];
        let mut actual_len = buf_len;
        // SAFETY: `scm` is caller-supplied per this function's own safety
        // contract; `wide_key` is a valid, NUL-terminated UTF-16 string
        // live for the whole call; `buf` is a valid buffer (possibly
        // zero-length, in which case a null/zeroed `lpDisplayName` with
        // `buf_len` `0` is documented to report the required size
        // without needing a real buffer yet) matched by `actual_len`
        // naming its exact length.
        let ok = unsafe {
            GetServiceDisplayNameW(scm, wide_key.as_ptr(), buf.as_mut_ptr(), &mut actual_len)
        };
        if ok != 0 {
            return Ok(alloc::string::String::from_utf16_lossy(
                &buf[..actual_len as usize],
            ));
        }
        let err = Win32Error::last();
        if err != Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
        buf_len = actual_len.max(buf_len + 1) + 1;
    }
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// Translate a service's human-readable display name
/// (`"Windows Event Log"`) to its short key name (`"eventlog"`) —
/// `GetServiceKeyNameW`. The reverse direction of [`display_name`].
///
/// Grows the buffer past whatever size `GetServiceKeyNameW` itself
/// reports on `ERROR_INSUFFICIENT_BUFFER` — see
/// `NAME_QUERY_MAX_RETRIES`'s own doc comment for why one extra
/// character of headroom isn't always enough in practice.
///
/// # Safety
///
/// `scm` must be a currently-open, valid SCM handle from
/// [`open_manager`] with at least [`SC_MANAGER_CONNECT`].
pub unsafe fn key_name(
    scm: RawHandle,
    display_name: &str,
) -> Result<alloc::string::String, Win32Error> {
    let wide_display: Vec<u16> = crate::wide::to_wide(display_name);
    let mut buf_len: u32 = 0;
    for _ in 0..=NAME_QUERY_MAX_RETRIES {
        let mut buf: Vec<u16> = alloc::vec![0u16; buf_len as usize];
        let mut actual_len = buf_len;
        // SAFETY: `scm` is caller-supplied per this function's own safety
        // contract; `wide_display` is a valid, NUL-terminated UTF-16
        // string live for the whole call; `buf` is a valid buffer
        // (possibly zero-length, in which case a null/zeroed
        // `lpServiceName` with `buf_len` `0` is documented to report the
        // required size without needing a real buffer yet) matched by
        // `actual_len` naming its exact length.
        let ok = unsafe {
            GetServiceKeyNameW(
                scm,
                wide_display.as_ptr(),
                buf.as_mut_ptr(),
                &mut actual_len,
            )
        };
        if ok != 0 {
            return Ok(alloc::string::String::from_utf16_lossy(
                &buf[..actual_len as usize],
            ));
        }
        let err = Win32Error::last();
        if err != Win32Error::ERROR_INSUFFICIENT_BUFFER {
            return Err(err);
        }
        buf_len = actual_len.max(buf_len + 1) + 1;
    }
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// `EnumDependentServicesW`'s `dwServiceState` selector — the same three
/// values [`enum_services`] itself always passes as
/// [`SERVICE_STATE_ALL`] internally, exposed here as a real parameter
/// since [`dependent_services`]'s own issue explicitly calls for one.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Active = SERVICE_ACTIVE,
    Inactive = SERVICE_INACTIVE,
    All = SERVICE_STATE_ALL,
}

// ENUM_SERVICE_STATUSW: `size_of` 48 (44 bytes of real fields, padded to
// an 8-byte boundary by the two leading pointers) -- verified against
// mingw-w64's own `winsvc.h` with a compiled `_Static_assert` probe. The
// older, pid-less `SERVICE_STATUS` (this crate's existing
// `ServiceStatusRaw`, from `control`), not `SERVICE_STATUS_PROCESS`.
// `lpServiceName`/`lpDisplayName` point into the same buffer
// `EnumDependentServicesW` filled, the same treatment `enum_services`'s
// `EnumServiceStatusProcessW` gets.
#[repr(C)]
#[derive(Clone, Copy)]
struct EnumServiceStatusRaw {
    service_name: *mut u16,
    display_name: *mut u16,
    service_status: ServiceStatusRaw,
}
const _: () = assert!(core::mem::size_of::<EnumServiceStatusRaw>() == 48);

/// One dependent service's name, display name, and status, as returned
/// by [`dependent_services`] — the "will stopping this break something
/// else" check before calling [`control`] with
/// [`ServiceControl::Stop`]. Lacks a process id/service-flags field
/// (unlike [`ServiceStatusEntry`]): `EnumDependentServicesW` reports the
/// older, pid-less `SERVICE_STATUS`, not `SERVICE_STATUS_PROCESS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependentService {
    pub service_name: alloc::string::String,
    pub display_name: alloc::string::String,
    pub service_type: u32,
    pub current_state: u32,
    pub controls_accepted: u32,
    pub win32_exit_code: u32,
    pub service_specific_exit_code: u32,
    pub check_point: u32,
    pub wait_hint: u32,
}

/// List every service depending on `handle`'s own service, in the given
/// `state` — `EnumDependentServicesW`, the "will stopping this break
/// something else" check before calling [`control`] with
/// [`ServiceControl::Stop`]. Grows the buffer and retries on either
/// `ERROR_MORE_DATA` (this function's own documented failure code for
/// an undersized buffer) or `ERROR_INSUFFICIENT_BUFFER` (treated the
/// same, out of caution after [`display_name`]/[`key_name`]'s own
/// buffer-sizing surprise — see `NAME_QUERY_MAX_RETRIES`'s doc
/// comment).
///
/// # Safety
///
/// `handle` must be a currently-open, valid service handle from
/// [`open_service`] with at least [`SERVICE_ENUMERATE_DEPENDENTS`].
pub unsafe fn dependent_services(
    handle: RawHandle,
    state: ServiceState,
) -> Result<Vec<DependentService>, Win32Error> {
    let mut buf_len: u32 = 8 * 1024;
    loop {
        let mut buf: Vec<u8> = alloc::vec![0u8; buf_len as usize];
        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        // SAFETY: `handle` is caller-supplied per this function's own
        // safety contract; `buf` is a valid buffer matched by `buf_len`
        // naming its exact size; `bytes_needed`/`services_returned` are
        // valid out-pointers.
        let ok = unsafe {
            EnumDependentServicesW(
                handle,
                state as u32,
                buf.as_mut_ptr(),
                buf_len,
                &mut bytes_needed,
                &mut services_returned,
            )
        };
        if ok == 0 {
            let err = Win32Error::last();
            if err != Win32Error::ERROR_MORE_DATA && err != Win32Error::ERROR_INSUFFICIENT_BUFFER {
                return Err(err);
            }
            buf_len = bytes_needed.max(buf_len + 1).max(buf_len * 2);
            continue;
        }

        let mut ptr = buf.as_ptr();
        let mut result = Vec::with_capacity(services_returned as usize);
        for _ in 0..services_returned {
            // SAFETY: `EnumDependentServicesW` guarantees
            // `services_returned` fixed-size `EnumServiceStatusRaw`
            // records packed contiguously starting at `buf`'s own start.
            let record: EnumServiceStatusRaw = unsafe { core::ptr::read_unaligned(ptr.cast()) };
            result.push(DependentService {
                // SAFETY: `record.service_name`/`record.display_name`
                // point into `buf`, which is still alive for this whole
                // loop body.
                service_name: unsafe { decode_wide_cstr(record.service_name) },
                display_name: unsafe { decode_wide_cstr(record.display_name) },
                service_type: record.service_status.service_type,
                current_state: record.service_status.current_state,
                controls_accepted: record.service_status.controls_accepted,
                win32_exit_code: record.service_status.win32_exit_code,
                service_specific_exit_code: record.service_status.service_specific_exit_code,
                check_point: record.service_status.check_point,
                wait_hint: record.service_status.wait_hint,
            });
            ptr = unsafe { ptr.add(core::mem::size_of::<EnumServiceStatusRaw>()) };
        }
        return Ok(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_manager_then_close_round_trips() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` was just opened above and hasn't been closed yet.
        unsafe { close(scm) }
            .expect("CloseServiceHandle should succeed on a freshly opened SCM handle");
    }

    #[test]
    fn open_service_then_close_round_trips_on_the_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");

        // "EventLog" (Windows Event Log) is a core OS service present
        // and installed on every Windows edition, including Server
        // Core -- a safe choice for a CI test that shouldn't depend on
        // optional roles/features being present.
        // SAFETY: `scm` is valid and open from the call just above.
        let service = unsafe { open_service(scm, "EventLog", SERVICE_QUERY_STATUS) }
            .expect("OpenServiceW should succeed for the well-known EventLog service");
        // SAFETY: `service` was just opened above and hasn't been closed
        // yet.
        unsafe { close(service) }
            .expect("CloseServiceHandle should succeed on a freshly opened service handle");

        // SAFETY: `scm` is still open (not yet closed).
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn open_service_fails_for_a_nonexistent_service_name() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");

        // SAFETY: `scm` is valid and open from the call just above.
        let err = unsafe {
            open_service(
                scm,
                "rusty_win32_definitely_not_a_real_service",
                SERVICE_QUERY_STATUS,
            )
        }
        .expect_err("OpenServiceW should fail for a nonexistent service name");
        assert_eq!(err, Win32Error::ERROR_SERVICE_DOES_NOT_EXIST);

        // SAFETY: `scm` is still open (not yet closed).
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn enum_services_includes_the_well_known_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE).expect(
            "OpenSCManagerW should succeed with SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE",
        );

        // SAFETY: `scm` is valid and open from the call just above.
        let services = unsafe { enum_services(scm) }
            .expect("EnumServicesStatusExW should succeed enumerating the local SCM database");
        assert!(
            !services.is_empty(),
            "a real Windows machine should have at least one service"
        );
        assert!(
            services
                .iter()
                .any(|s| s.service_name.eq_ignore_ascii_case("EventLog")),
            "the well-known EventLog service should appear in the enumeration, got: {:?}",
            services.iter().map(|s| &s.service_name).collect::<Vec<_>>()
        );
        // Every returned entry should have a real, nonempty display name
        // -- confirming the pointer-into-buffer decoding actually worked,
        // not just that the fixed-size fields happened to come through.
        assert!(
            services.iter().all(|s| !s.display_name.is_empty()),
            "every enumerated service should have a nonempty display name"
        );

        // SAFETY: `scm` is still open (not yet closed).
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn status_reports_a_plausible_state_for_the_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` is valid and open from the call just above.
        let service = unsafe { open_service(scm, "EventLog", SERVICE_QUERY_STATUS) }
            .expect("OpenServiceW should succeed for the well-known EventLog service");

        // SAFETY: `service` is valid and open from the call just above.
        let status = unsafe { status(service) }
            .expect("QueryServiceStatusEx should succeed for a valid service handle");
        assert!(
            matches!(
                status.current_state,
                SERVICE_STOPPED
                    | SERVICE_START_PENDING
                    | SERVICE_STOP_PENDING
                    | SERVICE_RUNNING
                    | SERVICE_CONTINUE_PENDING
                    | SERVICE_PAUSE_PENDING
                    | SERVICE_PAUSED
            ),
            "current_state should be one of the seven documented SERVICE_* states, got: {}",
            status.current_state
        );
        if status.current_state == SERVICE_RUNNING {
            assert!(
                status.process_id != 0,
                "a running service should report a nonzero backing process id"
            );
        }

        // SAFETY: `service`/`scm` are still open (not yet closed).
        unsafe { close(service) }.expect("CloseServiceHandle should succeed on the service handle");
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn start_fails_with_already_running_for_the_event_log_service() {
        // EventLog is a core OS service that's always already running by
        // the time this test executes -- calling `start` on it exercises
        // the real `StartServiceW` error path without this test actually
        // starting or stopping anything on the CI machine (a
        // non-destructive test, matching this crate's existing
        // discipline of avoiding side effects on shared system state).
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` is valid and open from the call just above.
        let service = match unsafe {
            open_service(scm, "EventLog", SERVICE_START | SERVICE_QUERY_STATUS)
        } {
            Ok(s) => s,
            Err(Win32Error::ERROR_ACCESS_DENIED) => {
                unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
                return;
            }
            Err(e) => panic!("OpenServiceW failed for EventLog service: {e:?}"),
        };

        // SAFETY: `service` is valid and open from the call just above.
        let status = unsafe { status(service) }
            .expect("QueryServiceStatusEx should succeed for a valid service handle");
        assert_eq!(
            status.current_state, SERVICE_RUNNING,
            "EventLog should already be running by the time this test runs"
        );

        // SAFETY: `service` is valid and open from the call just above.
        let err = unsafe { start(service) }
            .expect_err("StartServiceW should fail for an already-running service");
        assert_eq!(err, Win32Error::ERROR_SERVICE_ALREADY_RUNNING);

        // SAFETY: `service`/`scm` are still open (not yet closed).
        unsafe { close(service) }.expect("CloseServiceHandle should succeed on the service handle");
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn control_interrogate_succeeds_for_the_event_log_service() {
        // `SERVICE_CONTROL_INTERROGATE` never changes a service's state
        // (it just asks the service to re-report its current status) --
        // a non-destructive real exercise of `ControlService`, matching
        // this crate's existing discipline of avoiding side effects on
        // shared system state.
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` is valid and open from the call just above.
        let service = unsafe { open_service(scm, "EventLog", SERVICE_INTERROGATE) }
            .expect("OpenServiceW should succeed for the well-known EventLog service");

        // SAFETY: `service` is valid and open from the call just above.
        unsafe { control(service, ServiceControl::Interrogate) }
            .expect("ControlService should succeed interrogating a real, running service");

        // SAFETY: `service`/`scm` are still open (not yet closed).
        unsafe { close(service) }.expect("CloseServiceHandle should succeed on the service handle");
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn config_reports_a_plausible_configuration_for_the_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` is valid and open from the call just above.
        let service = unsafe { open_service(scm, "EventLog", SERVICE_QUERY_CONFIG) }
            .expect("OpenServiceW should succeed for the well-known EventLog service");

        // SAFETY: `service` is valid and open from the call just above.
        let config = unsafe { config(service) }
            .expect("QueryServiceConfigW should succeed for a valid service handle");
        assert!(
            !config.binary_path_name.is_empty(),
            "a real service should have a nonempty binary path"
        );
        assert!(
            !config.display_name.is_empty(),
            "a real service should have a nonempty display name"
        );
        // SERVICE_BOOT_START(0)..SERVICE_DISABLED(4) -- the five
        // documented `dwStartType` values.
        assert!(
            config.start_type <= 4,
            "start_type should be one of the five documented SERVICE_*_START/SERVICE_DISABLED values, got: {}",
            config.start_type
        );

        // SAFETY: `service`/`scm` are still open (not yet closed).
        unsafe { close(service) }.expect("CloseServiceHandle should succeed on the service handle");
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn display_name_then_key_name_round_trips_for_the_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");

        // SAFETY: `scm` is valid and open from the call just above.
        let display = unsafe { display_name(scm, "EventLog") }
            .expect("GetServiceDisplayNameW should succeed for the well-known EventLog service");
        assert!(
            !display.is_empty(),
            "EventLog's display name should be nonempty"
        );

        // SAFETY: `scm` is still valid and open.
        let key = unsafe { key_name(scm, &display) }.expect(
            "GetServiceKeyNameW should succeed round-tripping the display name just fetched",
        );
        assert!(
            key.eq_ignore_ascii_case("EventLog"),
            "round-tripping EventLog's display name back to a key name should recover \"eventlog\" (case-insensitive), got: {key}"
        );

        // SAFETY: `scm` is still open (not yet closed).
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn display_name_fails_for_a_nonexistent_service_name() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");

        // SAFETY: `scm` is valid and open from the call just above.
        let err = unsafe { display_name(scm, "rusty_win32_definitely_not_a_real_service") }
            .expect_err("GetServiceDisplayNameW should fail for a nonexistent service name");
        assert_eq!(err, Win32Error::ERROR_SERVICE_DOES_NOT_EXIST);

        // SAFETY: `scm` is still open (not yet closed).
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }

    #[test]
    fn dependent_services_succeeds_for_the_event_log_service() {
        let scm = open_manager(SC_MANAGER_CONNECT)
            .expect("OpenSCManagerW should succeed with SC_MANAGER_CONNECT");
        // SAFETY: `scm` is valid and open from the call just above.
        let service = unsafe { open_service(scm, "EventLog", SERVICE_ENUMERATE_DEPENDENTS) }
            .expect("OpenServiceW should succeed for the well-known EventLog service");

        // EventLog may or may not have any real dependents on a given
        // machine -- this only asserts the call itself succeeds and
        // decodes cleanly, not any particular count.
        // SAFETY: `service` is valid and open from the call just above.
        let dependents = unsafe { dependent_services(service, ServiceState::All) }
            .expect("EnumDependentServicesW should succeed for a valid service handle");
        assert!(
            dependents
                .iter()
                .all(|d| !d.service_name.is_empty() && !d.display_name.is_empty()),
            "every reported dependent should have nonempty names, got: {dependents:?}"
        );

        // SAFETY: `service`/`scm` are still open (not yet closed).
        unsafe { close(service) }.expect("CloseServiceHandle should succeed on the service handle");
        unsafe { close(scm) }.expect("CloseServiceHandle should succeed on the SCM handle");
    }
}
