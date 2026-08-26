//! `ReadDirectoryChangesW` — filesystem change notification, the Windows
//! analog of Linux's `inotify` (shaped very differently: one call watches
//! one directory handle, not a single fd multiplexing many watched paths).
//!
//! No current `rush` feature (no file-watch builtin) asks for this — added
//! per the round-2 capability assessment as a standard building block a
//! maturing interactive shell eventually wants (auto-reload on external
//! file changes, a `watch`-style builtin), not because of an existing gap.
//!
//! Unlike every other primitive in this crate, this one genuinely needs
//! `OVERLAPPED` I/O: `ReadDirectoryChangesW` has no way to bound how long it
//! blocks other than overlapped completion (there is no `dwNotifyFilter`
//! "timeout" parameter, and closing the directory handle from another
//! thread mid-call is not a safe cancellation technique). [`read_changes`]
//! wraps the overlapped path behind the same `Option<u32>` timeout
//! convention [`crate::process::wait`] already uses — `None` blocks
//! forever, `Some(ms)` bounds the wait and cancels the pending read via
//! `CancelIoEx` on timeout — so a caller never risks an unbounded hang the
//! way a naive synchronous wrapper would.

use crate::error::Win32Error;
use crate::handle::RawHandle;

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// `ReadDirectoryChangesW`'s `dwNotifyFilter` bits — which kind of change
/// to report. Exposed raw, the same policy-free convention this crate uses
/// for its other bitmask fields (`fs::FILE_ATTRIBUTE_*`,
/// `console::ENABLE_*`).
pub const FILE_NOTIFY_CHANGE_FILE_NAME: u32 = 0x0000_0001;
pub const FILE_NOTIFY_CHANGE_DIR_NAME: u32 = 0x0000_0002;
pub const FILE_NOTIFY_CHANGE_ATTRIBUTES: u32 = 0x0000_0004;
pub const FILE_NOTIFY_CHANGE_SIZE: u32 = 0x0000_0008;
pub const FILE_NOTIFY_CHANGE_LAST_WRITE: u32 = 0x0000_0010;
pub const FILE_NOTIFY_CHANGE_LAST_ACCESS: u32 = 0x0000_0020;
pub const FILE_NOTIFY_CHANGE_CREATION: u32 = 0x0000_0040;
pub const FILE_NOTIFY_CHANGE_SECURITY: u32 = 0x0000_0100;

/// `FILE_NOTIFY_INFORMATION::Action` — what happened to
/// [`FileNotification::file_name`].
pub const FILE_ACTION_ADDED: u32 = 1;
pub const FILE_ACTION_REMOVED: u32 = 2;
pub const FILE_ACTION_MODIFIED: u32 = 3;
/// The old name half of a rename — always immediately followed by a
/// [`FILE_ACTION_RENAMED_NEW_NAME`] entry for the same rename.
pub const FILE_ACTION_RENAMED_OLD_NAME: u32 = 4;
/// The new name half of a rename.
pub const FILE_ACTION_RENAMED_NEW_NAME: u32 = 5;

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const FILE_SHARE_DELETE: u32 = 0x0000_0004;
const OPEN_EXISTING: u32 = 3;
/// `CreateFileW`'s flag required to open a directory (a plain `CreateFileW`
/// on a directory path otherwise fails) — same constant [`crate::fs`]
/// duplicates locally for `readlink`'s own directory-symlink case, per
/// this crate's per-module-locality convention for tiny FFI-related
/// constants.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// `CreateFileW`'s flag requesting overlapped (asynchronous-capable) I/O on
/// the returned handle — required for [`read_changes`]'s bounded-wait
/// design; without it, `ReadDirectoryChangesW` only offers the unbounded
/// synchronous mode this module deliberately avoids.
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;

const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;

// OVERLAPPED: `size_of` 32, `align_of` 8 on x86_64. Verified against
// mingw-w64's `minwinbase.h` the same way as every other struct in this
// crate (a `_Static_assert` probe compiled with `x86_64-w64-mingw32-gcc`
// against the real header). The `Offset`/`OffsetHigh`/`Pointer` union is
// unused by `ReadDirectoryChangesW` (meaningful only for file-position-based
// I/O) and represented here only by its widest (pointer-sized) variant,
// the same convention `console::InputRecordKeyEvent` uses for a union it
// only ever populates one way.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    pointer: *mut core::ffi::c_void,
    h_event: RawHandle,
}
const _: () = assert!(core::mem::size_of::<Overlapped>() == 32);
const _: () = assert!(core::mem::align_of::<Overlapped>() == 8);
const _: () = assert!(core::mem::offset_of!(Overlapped, internal_high) == 8);
const _: () = assert!(core::mem::offset_of!(Overlapped, h_event) == 24);

/// Buffer size for one [`read_changes`] call. `ReadDirectoryChangesW` has
/// no "how much do you actually need" retry protocol the way
/// `SearchPathW`/`GetShortPathNameW` etc. do — if more change records
/// arrive in one OS-buffered burst than fit, the whole batch is discarded
/// and the call fails with `ERROR_NOTIFY_ENUM_DIR` (Windows' own signal
/// that changes were missed), not silently truncated. 64 KiB is the same
/// generous default Microsoft's own sample code uses.
const BUFFER_LEN: usize = 64 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: RawHandle,
    ) -> RawHandle;
    fn ReadDirectoryChangesW(
        directory: RawHandle,
        buffer: *mut u8,
        buffer_length: u32,
        watch_subtree: i32,
        notify_filter: u32,
        bytes_returned: *mut u32,
        overlapped: *mut Overlapped,
        completion_routine: *mut core::ffi::c_void,
    ) -> i32;
    fn CreateEventW(
        event_attributes: *const core::ffi::c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> RawHandle;
    fn WaitForSingleObject(handle: RawHandle, milliseconds: u32) -> u32;
    fn GetOverlappedResult(
        file: RawHandle,
        overlapped: *mut Overlapped,
        bytes_transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn CancelIoEx(file: RawHandle, overlapped: *mut Overlapped) -> i32;
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Open `path` (a directory) for watching via [`read_changes`] —
/// `CreateFileW` with `FILE_FLAG_BACKUP_SEMANTICS` (required to open any
/// directory at all) and `FILE_FLAG_OVERLAPPED` (required for
/// `read_changes`'s bounded-wait design). Shares read/write/delete with
/// every other handle, so watching a directory doesn't block ordinary
/// access to it.
pub fn open_directory(path: &str) -> Result<RawHandle, Win32Error> {
    let wide = to_wide(path);
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
    // `security_attributes = NULL` (default, non-inheritable) and
    // `template_file = NULL` (ignored by `OPEN_EXISTING`) are
    // documented-valid inputs.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            core::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle as isize == -1 {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// One reported change — `action` is one of the `FILE_ACTION_*` constants
/// above; `file_name` is relative to the watched directory (never a full
/// path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNotification {
    pub action: u32,
    pub file_name: String,
}

/// Block on `directory` (from [`open_directory`]) for up to `timeout_ms`
/// (`None` = wait forever), returning every change reported in one
/// `ReadDirectoryChangesW` completion — `Some(notifications)` if the wait
/// completed within the timeout (an empty `Vec` is possible, though rare:
/// see the buffer-overflow caveat below), `None` on timeout, matching
/// [`crate::process::wait`]'s own `Option<u32>` timeout convention. Call
/// repeatedly (typically from a dedicated loop/thread) to keep watching
/// after each completion — this reports one batch, not a continuous stream.
///
/// If more change records arrive in one OS-buffered burst than the
/// internal 64 KiB buffer holds, the whole batch is discarded and this
/// reports [`Win32Error::ERROR_NOTIFY_ENUM_DIR`] — Windows' own signal
/// that changes were missed, not a `Vec` this wrapper could still salvage
/// a partial result from.
///
/// # Safety
///
/// `directory` must be a currently-open, valid handle from
/// [`open_directory`] (specifically: opened with `FILE_FLAG_OVERLAPPED`,
/// which `open_directory` always includes).
pub unsafe fn read_changes(
    directory: RawHandle,
    watch_subtree: bool,
    notify_filter: u32,
    timeout_ms: Option<u32>,
) -> Result<Option<Vec<FileNotification>>, Win32Error> {
    // SAFETY: a manual-reset, initially-unsignaled event with no name and
    // default security attributes — all documented-valid inputs.
    let event = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    if event.is_null() {
        return Err(Win32Error::last());
    }

    let mut overlapped = Overlapped {
        h_event: event,
        ..Default::default()
    };
    let mut buf = alloc::vec![0u8; BUFFER_LEN];

    // SAFETY: `directory` is caller-supplied per this function's own
    // safety contract (opened overlapped, per that contract); `buf` is a
    // valid, `buf.len()`-byte writable buffer; `bytes_returned = NULL` is
    // required (not merely permitted) when `lpOverlapped` is non-NULL;
    // `overlapped` is a valid, freshly initialized struct whose `hEvent`
    // was just created above.
    let ok = unsafe {
        ReadDirectoryChangesW(
            directory,
            buf.as_mut_ptr(),
            buf.len() as u32,
            i32::from(watch_subtree),
            notify_filter,
            core::ptr::null_mut(),
            &mut overlapped,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = Win32Error::last();
        if err != Win32Error::ERROR_IO_PENDING {
            // SAFETY: `event` is a freshly created, valid handle, not used
            // again after this.
            let _ = unsafe { crate::handle::close(event) };
            return Err(err);
        }
    }

    // SAFETY: `event` is the same valid handle just created/started
    // above.
    let wait_result = unsafe { WaitForSingleObject(event, timeout_ms.unwrap_or(INFINITE)) };
    if wait_result == WAIT_TIMEOUT {
        // Cancel the still-pending read before returning: `overlapped`
        // and `buf` are both about to be dropped, and the OS must not go
        // on writing into either after that happens. `CancelIoEx` only
        // requests cancellation — `GetOverlappedResult` with `wait = TRUE`
        // below blocks until the cancellation has actually completed,
        // which is what actually makes dropping `overlapped`/`buf` safe.
        // SAFETY: `directory`/`overlapped` are the same valid handle/struct
        // from the call above.
        unsafe { CancelIoEx(directory, &mut overlapped) };
        let mut discarded: u32 = 0;
        // SAFETY: same `directory`/`overlapped`; `discarded` is a valid
        // out-pointer; `wait = TRUE` (1) blocks for the cancellation's
        // own completion, not a real timeout risk (cancellation always
        // completes, unlike the original I/O this function gave up
        // waiting on).
        unsafe { GetOverlappedResult(directory, &mut overlapped, &mut discarded, 1) };
        // SAFETY: `event` is still valid, not used again after this.
        let _ = unsafe { crate::handle::close(event) };
        return Ok(None);
    }
    if wait_result != WAIT_OBJECT_0 {
        // SAFETY: `event` is still valid, not used again after this.
        let _ = unsafe { crate::handle::close(event) };
        return Err(Win32Error::last());
    }

    let mut bytes_transferred: u32 = 0;
    // SAFETY: `directory`/`overlapped` are the same valid handle/struct;
    // the event just reported signaled, so the I/O has genuinely
    // completed — `wait = FALSE` (0) only fetches the already-available
    // result, it doesn't block.
    let ok = unsafe { GetOverlappedResult(directory, &mut overlapped, &mut bytes_transferred, 0) };
    // SAFETY: `event` is still valid, not used again after this.
    let _ = unsafe { crate::handle::close(event) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    if bytes_transferred == 0 {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(parse_notifications(
        &buf[..bytes_transferred as usize],
    )))
}

/// Walk one `ReadDirectoryChangesW` result buffer, decoding each
/// `FILE_NOTIFY_INFORMATION` record in turn via its own `NextEntryOffset`
/// chain (the last record's is `0`).
fn parse_notifications(buf: &[u8]) -> Vec<FileNotification> {
    const HEADER_LEN: usize = 12; // NextEntryOffset + Action + FileNameLength, each a u32
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        // A malformed/short trailing record (shouldn't happen — the OS
        // guarantees well-formed records up through `bytes_transferred` —
        // but not assumed away, the same defensive stance this crate
        // takes elsewhere) just ends parsing rather than panicking or
        // reading out of bounds.
        if offset + HEADER_LEN > buf.len() {
            break;
        }
        let next_entry_offset =
            u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        let action = u32::from_ne_bytes(buf[offset + 4..offset + 8].try_into().unwrap());
        let file_name_length =
            u32::from_ne_bytes(buf[offset + 8..offset + 12].try_into().unwrap()) as usize;

        let name_start = offset + HEADER_LEN;
        let name_end = name_start + file_name_length;
        if name_end > buf.len() {
            break;
        }
        // `FileNameLength` is a byte count, and `FileName` is UTF-16 with
        // no terminating NUL of its own (its length is exactly
        // `FileNameLength` bytes) — decode by 2-byte units rather than
        // scanning for a terminator.
        let units: Vec<u16> = buf[name_start..name_end]
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        out.push(FileNotification {
            action,
            file_name: String::from_utf16_lossy(&units),
        });

        if next_entry_offset == 0 {
            break;
        }
        offset += next_entry_offset;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_changes_reports_a_created_file() {
        let dir = std::env::temp_dir().join(alloc::format!(
            "rusty_win32_watch_test_{}",
            crate::process::current_pid()
        ));
        std::fs::create_dir_all(&dir).expect("create_dir_all should succeed");
        let dir_str = dir.to_str().unwrap().to_string();

        let directory =
            open_directory(&dir_str).expect("CreateFileW should succeed for an existing directory");

        // A raw `HANDLE` isn't `Send` — cross the thread boundary as a
        // plain `usize`, the same technique `pipe.rs`'s own threaded test
        // uses, and cast back inside the thread.
        let directory_val = directory as usize;
        let watch_thread = std::thread::spawn(move || {
            // SAFETY: `directory_val` names the handle opened above,
            // opened via `open_directory` (so `FILE_FLAG_OVERLAPPED` is
            // set) and kept alive until this thread is joined below,
            // before the handle is closed.
            unsafe {
                read_changes(
                    directory_val as RawHandle,
                    false,
                    FILE_NOTIFY_CHANGE_FILE_NAME,
                    Some(10_000),
                )
            }
        });

        // Give the watch thread a moment to actually reach
        // `ReadDirectoryChangesW` before triggering the change below —
        // otherwise the write could (rarely) happen first and there'd be
        // nothing pending for the read to observe. A real regression
        // (the read never completing) still fails deterministically via
        // the 10-second timeout above rather than hanging the test suite.
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::fs::write(dir.join("new_file.txt"), b"hello").expect("fs::write should succeed");

        let notifications = watch_thread
            .join()
            .expect("watch thread should not panic")
            .expect("ReadDirectoryChangesW should succeed")
            .expect("the change should be observed well within the 10s timeout");
        assert!(
            notifications
                .iter()
                .any(|n| n.file_name == "new_file.txt" && n.action == FILE_ACTION_ADDED),
            "expected a FILE_ACTION_ADDED notification for new_file.txt, got: {notifications:?}"
        );

        // SAFETY: `directory` is a valid, currently-open handle, closed
        // exactly once and not used again after this.
        unsafe { crate::handle::close(directory).unwrap() };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_changes_times_out_when_nothing_changes() {
        let dir = std::env::temp_dir().join(alloc::format!(
            "rusty_win32_watch_timeout_test_{}",
            crate::process::current_pid()
        ));
        std::fs::create_dir_all(&dir).expect("create_dir_all should succeed");
        let dir_str = dir.to_str().unwrap().to_string();

        let directory =
            open_directory(&dir_str).expect("CreateFileW should succeed for an existing directory");
        // SAFETY: `directory` is freshly opened via `open_directory`
        // (overlapped) and valid; nothing touches this directory for the
        // duration of the short timeout below.
        let result =
            unsafe { read_changes(directory, false, FILE_NOTIFY_CHANGE_FILE_NAME, Some(200)) }
                .expect("ReadDirectoryChangesW should succeed even on timeout");
        assert_eq!(result, None);

        // SAFETY: `directory` is a valid, currently-open handle, closed
        // exactly once.
        unsafe { crate::handle::close(directory).unwrap() };
        std::fs::remove_dir_all(&dir).ok();
    }
}
