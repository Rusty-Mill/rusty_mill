//! `GetFileAttributesExW`/`GetFileInformationByHandle` — the Windows
//! counterpart of Unix `stat`/`fstat`. `rusty_libc` can answer "is this a
//! directory/is this executable/how big is this file" with a `stat` mode
//! and size field; this module is the Windows analog, needed for `ls`,
//! globbing, and the `[ -d/-f/... ]` test operators to work there too.
//!
//! Deciding what a given `FILE_ATTRIBUTE_*` bit or field means to a caller
//! (e.g. "is this a directory") is the caller's policy — this module only
//! exposes the raw attributes, the same way [`crate::console`]'s
//! `ENABLE_*` bits are exposed without a raw-mode recipe baked in.
//!
//! [`read_dir`] (`FindFirstFileW`/`FindNextFileW`) rounds this module out
//! with directory listing — from a parity-loop pass against the real
//! Win32 API surface (`docs/archive/gap-analysis.md`, now closed out and
//! archived), the single biggest concrete gap
//! found: this module could stat individual paths but had no way to
//! enumerate a directory's contents at all, which any future `ls`/tab-
//! completion/glob implementation that walks a directory needs.
//!
//! This module's several-item surface (half a dozen functions, several
//! result structs, and the `FILE_ATTRIBUTE_*` constants) is deliberately
//! not re-exported at the crate root, the same reasoning [`crate::job`]
//! documents for its own multi-item surface: reach it via
//! `rusty_win32::fs::*`.

use crate::error::Win32Error;
use crate::handle::RawHandle;
use crate::time::Timespec;

extern crate alloc;
use alloc::vec::Vec;

// FILE_ATTRIBUTE_* bits (GetFileAttributesExW's/GetFileInformationByHandle's
// dwFileAttributes) this crate has an immediate use for. Windows defines
// several more (COMPRESSED, ENCRYPTED, TEMPORARY, ...) — out of scope until
// something actually needs them.
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0002;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0004;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0020;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x0080;
/// Set on symlinks/junctions/mount points — the bit a future symlink-aware
/// `stat`-vs-`lstat` distinction would key on.
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

/// `GetFileAttributesExW`'s `GET_FILEEX_INFO_LEVELS::GetFileExInfoStandard`
/// — the only info level Windows currently defines.
const GET_FILE_EX_INFO_STANDARD: u32 = 0;

/// `CreateSymbolicLinkW`'s `dwFlags`: the target is a directory, not a file
/// — Windows symlinks (unlike Unix ones) need to know this up front rather
/// than discovering it by following the link.
pub const SYMBOLIC_LINK_FLAG_DIRECTORY: u32 = 0x1;
/// Lets `CreateSymbolicLinkW` succeed without elevation, given either
/// Developer Mode enabled or the `SeCreateSymbolicLinkPrivilege` user right
/// — [`create_symlink`] always includes this bit internally, since there's
/// no reason a caller would ever want to *forbid* the unprivileged path
/// when the OS supports it (this crate's Windows 10 1809+ floor postdates
/// its introduction).
const SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE: u32 = 0x2;

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;
/// `CreateFileW`'s `dwCreationDisposition`: always create a new file,
/// truncating one that already exists — [`create_file`]'s counterpart to
/// Unix `O_CREAT | O_TRUNC`.
const CREATE_ALWAYS: u32 = 2;
/// `CreateFileW`'s `dwCreationDisposition`: open the file if it exists,
/// else create it — [`create_file`]'s counterpart to Unix `O_CREAT`
/// without `O_TRUNC`.
const OPEN_ALWAYS: u32 = 4;
/// `CreateFileW`'s `dwDesiredAccess` bit — read access. Public: callers
/// building their own `desired_access` for [`open_file`] need it (mirrors
/// [`crate::pipe::GENERIC_READ`], duplicated here per this crate's
/// per-module FFI-constant convention).
pub const GENERIC_READ: u32 = 0x8000_0000;
/// `CreateFileW`'s `dwDesiredAccess` bit — write access.
pub const GENERIC_WRITE: u32 = 0x4000_0000;
/// `CreateFileW`'s flag to open the reparse point itself (its stored
/// target) rather than following it — the Windows analog of `O_NOFOLLOW`,
/// and the primitive [`readlink`] needs to report a link's target instead
/// of whatever the link resolves to.
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// `CreateFileW`'s flag required to open a directory (a plain `CreateFileW`
/// on a directory path otherwise fails) — needed here since a directory
/// symlink/junction is exactly the reparse-point case [`readlink`] targets.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// `DeviceIoControl`'s IO control code to read a reparse point's raw target
/// data — there is no ordinary Win32 API for this; it's an NT-native
/// `DeviceIoControl` request, the same way Windows repurposes I/O
/// completion ports for job notifications in [`crate::job`] rather than
/// defining a dedicated high-level function.
const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
/// The maximum size `DeviceIoControl(FSCTL_GET_REPARSE_POINT, ...)` ever
/// needs — a documented Windows constant, not one this crate invents.
const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;
/// `REPARSE_DATA_BUFFER::ReparseTag` value for a symlink (as opposed to a
/// junction/mount point or a vendor-specific reparse point) — the only tag
/// [`readlink`] currently understands how to parse.
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;

// FILETIME: `size_of` 8, `align_of` 4 on x86_64 — mirrors `time.rs`'s
// private struct of the same shape; duplicated locally rather than shared,
// matching this crate's existing per-module-locality convention for tiny
// FFI-mirror structs (e.g. `WAIT_TIMEOUT` is redefined in `process.rs`,
// `console.rs`, and `job.rs` rather than centralized).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}
const _: () = assert!(core::mem::size_of::<FileTime>() == 8);
const _: () = assert!(core::mem::align_of::<FileTime>() == 4);

/// 100ns ticks between the FILETIME epoch (1601-01-01) and the Unix epoch
/// (1970-01-01) — the same standard conversion constant `time.rs` uses.
const FILETIME_UNIX_EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const HUNDRED_NS_PER_SEC: i64 = 10_000_000;
const NANOS_PER_HUNDRED_NS: i64 = 100;

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

// WIN32_FILE_ATTRIBUTE_DATA: `size_of` 36, `align_of` 4 on x86_64. Verified
// against mingw-w64's `minwinbase.h` the same way as every other struct in
// this crate (a `_Static_assert` probe compiled with
// `x86_64-w64-mingw32-gcc` against the real header).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Win32FileAttributeData {
    file_attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    file_size_high: u32,
    file_size_low: u32,
}
const _: () = assert!(core::mem::size_of::<Win32FileAttributeData>() == 36);
const _: () = assert!(core::mem::align_of::<Win32FileAttributeData>() == 4);

// BY_HANDLE_FILE_INFORMATION: `size_of` 52, `align_of` 4 on x86_64. Verified
// the same way.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}
const _: () = assert!(core::mem::size_of::<ByHandleFileInformation>() == 52);
const _: () = assert!(core::mem::align_of::<ByHandleFileInformation>() == 4);

// REPARSE_DATA_BUFFER's fixed header, symlink variant only (the
// `SymbolicLinkReparseBuffer` arm of the real struct's union — this crate
// doesn't parse `MountPointReparseBuffer`/`GenericReparseBuffer`, see
// `readlink`'s own doc comment): `size_of`/offset-of-`Flags` verified
// against mingw-w64's `ddk/ntifs.h` the same way as every other struct in
// this crate (a `_Static_assert` probe compiled with
// `x86_64-w64-mingw32-gcc` against the real header — mingw-w64 ships this
// one under its DDK headers, not the ordinary Win32 set, since reparse
// points are an NT-native mechanism with no ordinary Win32 struct for it).
// `PathBuffer` (variable-length UTF-16 data) immediately follows this fixed
// 20-byte header; not represented as a Rust field since its length isn't
// known until `reparse_data_length` is read.
#[repr(C)]
#[derive(Clone, Copy)]
struct ReparseDataBufferSymlinkHeader {
    reparse_tag: u32,
    reparse_data_length: u16,
    reserved: u16,
    substitute_name_offset: u16,
    substitute_name_length: u16,
    print_name_offset: u16,
    print_name_length: u16,
    flags: u32,
}
const _: () = assert!(core::mem::size_of::<ReparseDataBufferSymlinkHeader>() == 20);
const _: () = assert!(core::mem::offset_of!(ReparseDataBufferSymlinkHeader, flags) == 16);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileAttributesExW(
        file_name: *const u16,
        info_level_id: u32,
        file_information: *mut core::ffi::c_void,
    ) -> i32;
    fn GetFileInformationByHandle(
        file: RawHandle,
        file_information: *mut ByHandleFileInformation,
    ) -> i32;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const core::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: RawHandle,
    ) -> RawHandle;
    // `CreateSymbolicLinkW` is documented to return `BOOLEAN` (an 8-bit
    // value), not the ordinary 32-bit `BOOL` every other function in this
    // crate returns — declared as `u8` rather than `i32` so this doesn't
    // read undefined bits above the byte the real ABI actually guarantees.
    fn CreateSymbolicLinkW(
        symlink_file_name: *const u16,
        target_file_name: *const u16,
        flags: u32,
    ) -> u8;
    fn CreateHardLinkW(
        file_name: *const u16,
        existing_file_name: *const u16,
        security_attributes: *const core::ffi::c_void,
    ) -> i32;
    fn GetFinalPathNameByHandleW(
        file: RawHandle,
        file_path: *mut u16,
        file_path_size: u32,
        flags: u32,
    ) -> u32;
    fn DeviceIoControl(
        device: RawHandle,
        io_control_code: u32,
        in_buffer: *const core::ffi::c_void,
        in_buffer_size: u32,
        out_buffer: *mut core::ffi::c_void,
        out_buffer_size: u32,
        bytes_returned: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn FindFirstFileW(file_name: *const u16, find_file_data: *mut FindDataW) -> RawHandle;
    fn FindNextFileW(find_file: RawHandle, find_file_data: *mut FindDataW) -> i32;
    fn FindClose(find_file: RawHandle) -> i32;
    fn GetCompressedFileSizeW(file_name: *const u16, file_size_high: *mut u32) -> u32;
    fn CopyFileW(
        existing_file_name: *const u16,
        new_file_name: *const u16,
        fail_if_exists: i32,
    ) -> i32;
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
    fn DeleteFileW(file_name: *const u16) -> i32;
    fn CreateDirectoryW(
        path_name: *const u16,
        security_attributes: *const core::ffi::c_void,
    ) -> i32;
    fn RemoveDirectoryW(path_name: *const u16) -> i32;
    fn LockFileEx(
        file: RawHandle,
        flags: u32,
        reserved: u32,
        number_of_bytes_to_lock_low: u32,
        number_of_bytes_to_lock_high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: RawHandle,
        reserved: u32,
        number_of_bytes_to_unlock_low: u32,
        number_of_bytes_to_unlock_high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn ReadFile(
        file: RawHandle,
        buffer: *mut u8,
        bytes_to_read: u32,
        bytes_read: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn WriteFile(
        file: RawHandle,
        buffer: *const u8,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
}

// OVERLAPPED: same shape as `watch.rs`'s own private mirror of this struct
// (`size_of` 32, `align_of` 8 on x86_64, already verified there against
// mingw-w64's `minwinbase.h`) — duplicated locally per this crate's
// per-module-locality convention for tiny FFI-mirror structs.
// `LockFileEx`/`UnlockFileEx` require a non-NULL `OVERLAPPED` even for an
// ordinary synchronous file handle, using it only to carry the 64-bit lock
// offset (the `Offset`/`OffsetHigh` union members, here represented by
// their union's widest `Pointer` variant, exactly like `watch.rs`'s own
// `Overlapped`) rather than for actual overlapped I/O — a handle opened
// without `FILE_FLAG_OVERLAPPED` still blocks synchronously until the lock
// is granted, per `LockFileEx`'s own documented behavior.
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

/// `LockFileEx`'s `dwFlags` bit requesting an exclusive (write) lock;
/// without it, the lock is shared (read).
const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;

/// `MoveFileExW`'s `dwFlags` bit: overwrite `to` if it already exists
/// (`MoveFileExW`'s own default refuses to, matching `rename`'s POSIX
/// semantics being the exception, not the rule, on Windows).
pub const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
/// `MoveFileExW`'s `dwFlags` bit: fall back to a copy-then-delete if `from`
/// and `to` are on different volumes — plain `MoveFileW`/a rename-only move
/// fails across volumes; this is the flag that makes `MoveFileExW` succeed
/// there instead, unlike `std::fs::rename` on Windows, which doesn't.
pub const MOVEFILE_COPY_ALLOWED: u32 = 0x2;

// WIN32_FIND_DATAW: `size_of` 592, `align_of` 4 on x86_64. Verified against
// mingw-w64's `minwinbase.h` the same way as every other struct in this
// crate (a `_Static_assert` probe compiled with `x86_64-w64-mingw32-gcc`
// against the real header). The Mac-only trailing fields (`#ifdef _MAC`)
// don't exist on this target and aren't modeled here.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FindDataW {
    file_attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    file_size_high: u32,
    file_size_low: u32,
    reserved0: u32,
    reserved1: u32,
    file_name: [u16; 260],
    alternate_file_name: [u16; 14],
}

// `[u16; 260]`/`[u16; 14]` don't implement `Default` (std only special-cases
// arrays up to length 32) — written by hand, the same reason
// `process::ProcessEntry32W` has a hand-written `Default` impl.
impl Default for FindDataW {
    fn default() -> Self {
        FindDataW {
            file_attributes: 0,
            creation_time: FileTime::default(),
            last_access_time: FileTime::default(),
            last_write_time: FileTime::default(),
            file_size_high: 0,
            file_size_low: 0,
            reserved0: 0,
            reserved1: 0,
            file_name: [0u16; 260],
            alternate_file_name: [0u16; 14],
        }
    }
}

const _: () = assert!(core::mem::size_of::<FindDataW>() == 592);
const _: () = assert!(core::mem::align_of::<FindDataW>() == 4);
const _: () = assert!(core::mem::offset_of!(FindDataW, creation_time) == 4);
const _: () = assert!(core::mem::offset_of!(FindDataW, file_size_high) == 28);
const _: () = assert!(core::mem::offset_of!(FindDataW, reserved0) == 36);
const _: () = assert!(core::mem::offset_of!(FindDataW, file_name) == 44);
const _: () = assert!(core::mem::offset_of!(FindDataW, alternate_file_name) == 44 + 260 * 2);

/// `INVALID_HANDLE_VALUE` — the sentinel `FindFirstFileW` returns instead of
/// `NULL` on failure (unlike most handle-returning calls in this crate).
const INVALID_HANDLE_VALUE: isize = -1;

fn decode_find_file_name(units: &[u16; 260]) -> alloc::string::String {
    let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    alloc::string::String::from_utf16_lossy(&units[..len])
}

/// `stat`-by-path result — attributes, timestamps, and size, via
/// `GetFileAttributesExW`. Does not require opening a handle to the file
/// first (unlike [`stat_by_handle`]), the same tradeoff Unix `stat` makes
/// over `fstat`.
#[derive(Debug, Clone, Copy)]
pub struct FileInfo {
    pub attributes: u32,
    pub creation_time: Timespec,
    pub last_access_time: Timespec,
    pub last_write_time: Timespec,
    pub size: u64,
}

/// `stat`-by-open-handle result — everything [`FileInfo`] has, plus
/// `volume_serial_number`/`file_index` (Windows' closest analog of a Unix
/// `(st_dev, st_ino)` pair — together they identify a file uniquely on its
/// volume, e.g. for detecting hardlinks) and `link_count`, none of which
/// `GetFileAttributesExW` reports.
#[derive(Debug, Clone, Copy)]
pub struct FileInfoByHandle {
    pub attributes: u32,
    pub creation_time: Timespec,
    pub last_access_time: Timespec,
    pub last_write_time: Timespec,
    pub size: u64,
    pub volume_serial_number: u32,
    pub link_count: u32,
    pub file_index: u64,
}

/// `stat` a path — `GetFileAttributesExW`. Fails with
/// [`Win32Error::ERROR_FILE_NOT_FOUND`]/[`Win32Error::ERROR_PATH_NOT_FOUND`]
/// (matching the real call's own documented behavior) if nothing exists at
/// `path`, rather than this wrapper inventing a distinct "not found" result.
pub fn stat(path: &str) -> Result<FileInfo, Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    let mut data = Win32FileAttributeData::default();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; `data` is a
    // valid, correctly-sized out-pointer matching what
    // `GetFileExInfoStandard` requires.
    let ok = unsafe {
        GetFileAttributesExW(
            wide.as_ptr(),
            GET_FILE_EX_INFO_STANDARD,
            (&mut data as *mut Win32FileAttributeData).cast(),
        )
    };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(FileInfo {
        attributes: data.file_attributes,
        creation_time: filetime_to_timespec(data.creation_time),
        last_access_time: filetime_to_timespec(data.last_access_time),
        last_write_time: filetime_to_timespec(data.last_write_time),
        size: (u64::from(data.file_size_high) << 32) | u64::from(data.file_size_low),
    })
}

/// `GetCompressedFileSizeW`'s sentinel return value on failure — but also
/// a legitimate value if the file's actual low-order 32 bits happen to be
/// all-ones, so a `GetLastError()` check (below) is required to
/// disambiguate, per this call's own documented contract.
const INVALID_FILE_SIZE: u32 = 0xFFFF_FFFF;

/// The on-disk (compressed) size of the file at `path`, in bytes —
/// `GetCompressedFileSizeW`. For a file with no NTFS compression (the
/// common case), this equals [`stat`]'s logical `size`; it's meaningfully
/// smaller only for a file NTFS is actually compressing on disk.
pub fn compressed_file_size(path: &str) -> Result<u64, Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    let mut size_high: u32 = 0;
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; `size_high`
    // is a valid out-pointer.
    let size_low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut size_high) };
    if size_low == INVALID_FILE_SIZE {
        let err = Win32Error::last();
        if err.code() != 0 {
            return Err(err);
        }
    }
    Ok((u64::from(size_high) << 32) | u64::from(size_low))
}

/// `fstat` an already-open handle — `GetFileInformationByHandle`. Reports
/// more than [`stat`] does (volume serial number, link count, file index)
/// since an open handle lets Windows answer a couple of questions a bare
/// path can't.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle to a file (not a
/// console, pipe, or other non-file handle type — `GetFileInformationByHandle`
/// only supports file handles).
pub unsafe fn stat_by_handle(handle: RawHandle) -> Result<FileInfoByHandle, Win32Error> {
    let mut info = ByHandleFileInformation::default();
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `info` is a valid, correctly-sized out-pointer.
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    Ok(FileInfoByHandle {
        attributes: info.file_attributes,
        creation_time: filetime_to_timespec(info.creation_time),
        last_access_time: filetime_to_timespec(info.last_access_time),
        last_write_time: filetime_to_timespec(info.last_write_time),
        size: (u64::from(info.file_size_high) << 32) | u64::from(info.file_size_low),
        volume_serial_number: info.volume_serial_number,
        link_count: info.number_of_links,
        file_index: (u64::from(info.file_index_high) << 32) | u64::from(info.file_index_low),
    })
}

/// Advisory-lock a `length`-byte region of `handle` starting at `offset` —
/// `LockFileEx`, the Windows analog of `flock` at the file level (an
/// alternative to [`crate::handle::create_mutex`]'s named-mutex option for
/// the same use case, e.g. guarding concurrent writes to a shared history
/// file). `exclusive` requests a write lock; `false` requests a shared
/// (read) lock. A handle opened without `FILE_FLAG_OVERLAPPED` blocks
/// synchronously until the lock is granted, per `LockFileEx`'s own
/// documented behavior.
///
/// # Safety
///
/// `handle` must be a currently-open, valid file handle.
pub unsafe fn lock_file(
    handle: RawHandle,
    exclusive: bool,
    offset: u64,
    length: u64,
) -> Result<(), Win32Error> {
    let flags = if exclusive {
        LOCKFILE_EXCLUSIVE_LOCK
    } else {
        0
    };
    let mut overlapped = Overlapped {
        pointer: offset as usize as *mut core::ffi::c_void,
        ..Overlapped::default()
    };
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `overlapped` is a valid, correctly-initialized out-pointer
    // (its `Offset`/`OffsetHigh` union slot carries `offset`, per this
    // call's own documented contract).
    let ok = unsafe {
        LockFileEx(
            handle,
            flags,
            0,
            (length & 0xFFFF_FFFF) as u32,
            (length >> 32) as u32,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Release a lock previously taken by [`lock_file`] over the same
/// `offset`/`length` region of `handle` — `UnlockFileEx`.
///
/// # Safety
///
/// `handle` must be a currently-open, valid file handle.
pub unsafe fn unlock_file(handle: RawHandle, offset: u64, length: u64) -> Result<(), Win32Error> {
    let mut overlapped = Overlapped {
        pointer: offset as usize as *mut core::ffi::c_void,
        ..Overlapped::default()
    };
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `overlapped` is a valid, correctly-initialized out-pointer.
    let ok = unsafe {
        UnlockFileEx(
            handle,
            0,
            (length & 0xFFFF_FFFF) as u32,
            (length >> 32) as u32,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Copy `from` to `to` — `CopyFileW`, the primitive behind a `cp` builtin.
/// `fail_if_exists`, true, refuses to overwrite an already-existing `to`
/// (reporting [`Win32Error::ERROR_FILE_EXISTS`]); false overwrites it, the
/// same choice `CopyFileW`'s own `bFailIfExists` parameter makes — this
/// crate doesn't decide that policy itself.
pub fn copy_file(from: &str, to: &str, fail_if_exists: bool) -> Result<(), Win32Error> {
    let from_wide: Vec<u16> = from.encode_utf16().chain(core::iter::once(0)).collect();
    let to_wide: Vec<u16> = to.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `from_wide`/`to_wide` are valid, NUL-terminated UTF-16 strings.
    let ok = unsafe {
        CopyFileW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            i32::from(fail_if_exists),
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Move (rename) `from` to `to` — `MoveFileExW`, the primitive behind an
/// `mv` builtin. `flags` is the raw `MOVEFILE_*` bitmask above (e.g.
/// [`MOVEFILE_REPLACE_EXISTING`]/[`MOVEFILE_COPY_ALLOWED`]) — this function
/// is a thin, policy-free wrapper, the same as this crate's other raw
/// bitmask parameters. Unlike `std::fs::rename` on Windows, passing
/// `MOVEFILE_COPY_ALLOWED` lets this succeed across volumes (falling back
/// to a copy-then-delete internally) rather than failing.
pub fn move_file(from: &str, to: &str, flags: u32) -> Result<(), Win32Error> {
    let from_wide: Vec<u16> = from.encode_utf16().chain(core::iter::once(0)).collect();
    let to_wide: Vec<u16> = to.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `from_wide`/`to_wide` are valid, NUL-terminated UTF-16 strings;
    // `flags` is a plain bitmask, not a pointer.
    let ok = unsafe { MoveFileExW(from_wide.as_ptr(), to_wide.as_ptr(), flags) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Delete the file at `path` — `DeleteFileW`, the primitive behind an `rm`
/// builtin. Only removes files, not directories, matching `DeleteFileW`'s
/// own scope (`RemoveDirectoryW`, out of this crate's current scope, is the
/// directory-removal counterpart).
pub fn delete_file(path: &str) -> Result<(), Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string.
    let ok = unsafe { DeleteFileW(wide.as_ptr()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Create a directory at `path` — `CreateDirectoryW`, the primitive behind
/// an `mkdir` builtin. Only creates the final path component (unlike `mkdir
/// -p`); every parent directory must already exist.
pub fn create_directory(path: &str) -> Result<(), Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
    // `security_attributes = NULL` requests default (non-inheritable)
    // security attributes, a documented valid input.
    let ok = unsafe { CreateDirectoryW(wide.as_ptr(), core::ptr::null()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Remove the directory at `path` — `RemoveDirectoryW`, the primitive
/// behind an `rmdir` builtin. `path` must name an empty directory; Windows
/// refuses to remove one with any contents (no `rm -rf`-style recursive
/// behavior here).
pub fn remove_directory(path: &str) -> Result<(), Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string.
    let ok = unsafe { RemoveDirectoryW(wide.as_ptr()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Create a symbolic link at `link_path` pointing at `target_path` —
/// `CreateSymbolicLinkW`, `ln -s`'s Windows counterpart. `target_is_directory`
/// must say whether `target_path` names a directory: unlike Unix, Windows
/// needs to know this up front rather than discovering it by following the
/// link, since a directory symlink and a file symlink are different reparse
/// point subtypes.
///
/// Always requests `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE`, so this
/// succeeds without elevation given Developer Mode enabled or the
/// `SeCreateSymbolicLinkPrivilege` user right — without it, creating a
/// symlink at all requires an elevated (administrator) process.
pub fn create_symlink(
    link_path: &str,
    target_path: &str,
    target_is_directory: bool,
) -> Result<(), Win32Error> {
    let link_wide: Vec<u16> = link_path
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let target_wide: Vec<u16> = target_path
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE
        | if target_is_directory {
            SYMBOLIC_LINK_FLAG_DIRECTORY
        } else {
            0
        };
    // SAFETY: `link_wide`/`target_wide` are valid, NUL-terminated UTF-16
    // strings; `flags` is a plain bitmask, not a pointer.
    let ok = unsafe { CreateSymbolicLinkW(link_wide.as_ptr(), target_wide.as_ptr(), flags) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// Create a hard link at `link_path` pointing at the same file content as
/// `target_path` — `CreateHardLinkW`, `ln`'s (without `-s`) Windows
/// counterpart, the non-symbolic counterpart to [`create_symlink`]. Unlike
/// a symlink, a hard link is indistinguishable from the original file
/// (both names refer to the same underlying data); `target_path` must
/// already exist and both paths must be on the same volume — a
/// documented `CreateHardLinkW` restriction, not something this wrapper
/// checks itself.
pub fn create_hard_link(link_path: &str, target_path: &str) -> Result<(), Win32Error> {
    let link_wide: Vec<u16> = link_path
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let target_wide: Vec<u16> = target_path
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    // SAFETY: `link_wide`/`target_wide` are valid, NUL-terminated UTF-16
    // strings; `security_attributes = NULL` requests default security
    // attributes, a documented valid input.
    let ok =
        unsafe { CreateHardLinkW(link_wide.as_ptr(), target_wide.as_ptr(), core::ptr::null()) };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(())
    }
}

/// The fully resolved path an open handle refers to — `GetFinalPathNameByHandleW`,
/// walking through any symlink/junction along the way (unlike [`readlink`],
/// which reports a single link's own stored target without following it).
/// Rust's own `std::fs::canonicalize` uses this exact call on Windows for
/// the same reason. Returns a `\\?\`-prefixed path (`GetFinalPathNameByHandleW`'s
/// default normalization), not a plain drive-letter one.
///
/// # Safety
///
/// `handle` must be a currently-open, valid file handle.
pub unsafe fn final_path(handle: RawHandle) -> Result<alloc::string::String, Win32Error> {
    let mut buf: Vec<u16> = alloc::vec![0u16; 260];
    // At most two attempts: an initial try, then one retry sized exactly to
    // whatever `GetFinalPathNameByHandleW` reports as actually required —
    // matching `path::search_path`'s own growing-buffer pattern.
    for _ in 0..2 {
        // SAFETY: `handle` is caller-supplied per this function's own
        // safety contract; `buf` is a valid, `buf.len()`-element writable
        // buffer.
        let needed =
            unsafe { GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, 0) };
        if needed == 0 {
            return Err(Win32Error::last());
        }
        if (needed as usize) > buf.len() {
            buf.resize(needed as usize, 0);
            continue;
        }
        return Ok(alloc::string::String::from_utf16_lossy(
            &buf[..needed as usize],
        ));
    }
    // Unreachable in practice, matching `path::search_path`'s own reasoning
    // for this exact fallback.
    Err(Win32Error::ERROR_INSUFFICIENT_BUFFER)
}

/// Read a symlink's own stored target, without following it — the Windows
/// analog of Unix `readlink`. Only understands `IO_REPARSE_TAG_SYMLINK`
/// (an ordinary symlink); junctions/mount points and vendor-specific
/// reparse points report [`Win32Error::ERROR_NOT_SUPPORTED`] rather than
/// this function misinterpreting their differently-shaped data — a
/// deliberate scope cut, not an oversight (nothing in this crate's current
/// scope needs junction support).
///
/// Returns the link's "print name" (the human-readable form Windows itself
/// shows for a symlink, e.g. in Explorer or `dir`), not its "substitute
/// name" (an NT-native, sometimes `\??\`-prefixed absolute form used
/// internally for resolution) — the print name is the closer analog of
/// what Unix `readlink` reports, since it's the same string
/// [`create_symlink`]'s own `target_path` argument produces.
pub fn readlink(path: &str) -> Result<alloc::string::String, Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string;
    // `desired_access = 0` (query-only, no read/write) and
    // `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` are
    // documented-valid inputs for opening a reparse point (file or
    // directory) without following it or requiring its target to exist.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle as isize == -1 {
        return Err(Win32Error::last());
    }

    let mut buf: Vec<u8> = alloc::vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE];
    let mut bytes_returned: u32 = 0;
    // SAFETY: `handle` was just successfully opened above; `buf` is a
    // valid, `buf.len()`-byte writable buffer, sized to Windows' own
    // documented maximum for this request so it's never too small; the
    // request takes no input buffer (`in_buffer = NULL`, `in_buffer_size =
    // 0`), a documented-valid combination for `FSCTL_GET_REPARSE_POINT`.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            core::ptr::null(),
            0,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut bytes_returned,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = Win32Error::last();
        // SAFETY: `handle` is valid and not used again after this.
        let _ = unsafe { crate::handle::close(handle) };
        return Err(err);
    }
    // SAFETY: `handle` is valid and not used again after this point.
    let _ = unsafe { crate::handle::close(handle) };

    // SAFETY: a successful `DeviceIoControl` call guarantees at least this
    // fixed 20-byte header is initialized; `read_unaligned` doesn't require
    // `buf`'s allocation to happen to satisfy the header's own alignment.
    let header: ReparseDataBufferSymlinkHeader =
        unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };
    if header.reparse_tag != IO_REPARSE_TAG_SYMLINK {
        return Err(Win32Error::ERROR_NOT_SUPPORTED);
    }

    // `PathBuffer` starts immediately after this fixed header;
    // `*_offset`/`*_length` are documented as byte offsets/lengths relative
    // to the start of `PathBuffer`, not the whole struct.
    const PATH_BUFFER_START: usize = 20;
    let print_name_start = PATH_BUFFER_START + header.print_name_offset as usize;
    let print_name_end = print_name_start + header.print_name_length as usize;
    let print_name_bytes = &buf[print_name_start..print_name_end];
    // Reconstructed manually from raw bytes (rather than casting to a
    // `[u16]` slice) since `buf`'s allocation isn't guaranteed 2-byte
    // aligned — the same reason `job::process_ids` reads its own
    // variable-length buffer field-by-field instead of casting.
    let print_name_units: Vec<u16> = print_name_bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Ok(char::decode_utf16(print_name_units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

/// One directory entry from [`read_dir`] — `WIN32_FIND_DATAW`'s fields.
/// `file_name` is the bare entry name (`"foo.txt"`, `"."`, `".."`), not a
/// full path, matching Unix `readdir`'s own `d_name` convention: joining it
/// back onto the directory being listed is the caller's job.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub file_name: alloc::string::String,
    pub file_attributes: u32,
    pub file_size: u64,
    pub creation_time: Timespec,
    pub last_access_time: Timespec,
    pub last_write_time: Timespec,
}

fn dir_entry_from_find_data(data: &FindDataW) -> DirEntry {
    DirEntry {
        file_name: decode_find_file_name(&data.file_name),
        file_attributes: data.file_attributes,
        file_size: (u64::from(data.file_size_high) << 32) | u64::from(data.file_size_low),
        creation_time: filetime_to_timespec(data.creation_time),
        last_access_time: filetime_to_timespec(data.last_access_time),
        last_write_time: filetime_to_timespec(data.last_write_time),
    }
}

/// Enumerates a directory's contents — `FindFirstFileW`/`FindNextFileW`,
/// closing the search handle via `FindClose` on drop (including on an early
/// `break`, unlike a caller that forgot to check for one). Windows always
/// reports `.`/`..` as real entries the way Unix `readdir` does (unlike
/// this crate's other iterators, nothing here filters them — deciding
/// whether to skip them is the caller's policy, the same way this crate
/// exposes raw `FILE_ATTRIBUTE_*` bits without deciding what they mean).
#[derive(Debug)]
pub struct ReadDir {
    handle: RawHandle,
    // `FindFirstFileW` already produced the first entry by the time this
    // struct exists — `next()` returns it before ever calling
    // `FindNextFileW`, the same "the opening call already returned data"
    // shape `process::list_processes`'s `Process32FirstW` loop has.
    pending: Option<FindDataW>,
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry, Win32Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(data) = self.pending.take() {
            return Some(Ok(dir_entry_from_find_data(&data)));
        }
        let mut data = FindDataW::default();
        // SAFETY: `self.handle` is a valid, currently-open search handle
        // (from `read_dir`, not yet closed); `data` is a valid,
        // correctly-sized out-pointer.
        let found = unsafe { FindNextFileW(self.handle, &mut data) };
        if found == 0 {
            // `Process32NextW`'s own end-of-enumeration convention, applied
            // the same way here: a `FALSE` return with
            // `GetLastError() == ERROR_NO_MORE_FILES` means "nothing left,"
            // not a real error.
            let err = Win32Error::last();
            return if err == Win32Error::ERROR_NO_MORE_FILES {
                None
            } else {
                Some(Err(err))
            };
        }
        Some(Ok(dir_entry_from_find_data(&data)))
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a valid, currently-open search handle,
        // closed exactly once here and never used again after.
        let _ = unsafe { FindClose(self.handle) };
    }
}

/// Start enumerating `pattern` — `FindFirstFileW`. `pattern` may include `?`/
/// `*` wildcards (Windows' own, resolved by this call directly, not this
/// crate's `*` glob semantics elsewhere) — pass e.g. `"C:\\dir\\*"` to list
/// every entry in a directory, matching the standard idiom every Win32
/// directory-listing example uses.
pub fn read_dir(pattern: &str) -> Result<ReadDir, Win32Error> {
    let wide: Vec<u16> = pattern.encode_utf16().chain(core::iter::once(0)).collect();
    let mut data = FindDataW::default();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; `data` is a
    // valid, correctly-sized out-pointer.
    let handle = unsafe { FindFirstFileW(wide.as_ptr(), &mut data) };
    if handle.is_null() || handle as isize == INVALID_HANDLE_VALUE {
        return Err(Win32Error::last());
    }
    Ok(ReadDir {
        handle,
        pending: Some(data),
    })
}

/// Open an existing regular file for content I/O — `CreateFileW` with
/// [`OPEN_EXISTING`], the Windows counterpart of `rusty_libc::fd::open`
/// with `O_RDONLY`/`O_WRONLY`/`O_RDWR` and no `O_CREAT`. `desired_access`
/// is [`GENERIC_READ`]/[`GENERIC_WRITE`] (or both ORed).
pub fn open_file(path: &str, desired_access: u32) -> Result<RawHandle, Win32Error> {
    create_file_impl(path, desired_access, OPEN_EXISTING)
}

/// Create a new regular file (or open-and-truncate an existing one) for
/// content I/O — `CreateFileW` with [`CREATE_ALWAYS`], the Windows
/// counterpart of `rusty_libc::fd::open` with `O_CREAT | O_TRUNC`.
pub fn create_file(path: &str, desired_access: u32) -> Result<RawHandle, Win32Error> {
    create_file_impl(path, desired_access, CREATE_ALWAYS)
}

/// Open a regular file, creating it if it doesn't already exist (without
/// truncating one that does) — `CreateFileW` with [`OPEN_ALWAYS`], the
/// Windows counterpart of `rusty_libc::fd::open` with `O_CREAT` and no
/// `O_TRUNC`.
pub fn open_or_create_file(path: &str, desired_access: u32) -> Result<RawHandle, Win32Error> {
    create_file_impl(path, desired_access, OPEN_ALWAYS)
}

fn create_file_impl(
    path: &str,
    desired_access: u32,
    creation_disposition: u32,
) -> Result<RawHandle, Win32Error> {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string; sharing both
    // read and write with other opens matches Unix `open`'s default (no
    // exclusive lock); `security_attributes = NULL` and
    // `FILE_ATTRIBUTE_NORMAL` are documented-valid defaults for a plain
    // file; `template_file = NULL` is ignored except for `CREATE_NEW`
    // (which this module never passes as `creation_disposition`).
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            creation_disposition,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle as isize == -1 {
        Err(Win32Error::last())
    } else {
        Ok(handle)
    }
}

/// Read up to `buf.len()` bytes from `handle` — `ReadFile` against a
/// regular file handle from [`open_file`]/[`create_file`]/
/// [`open_or_create_file`]. Returns `0` at end-of-file.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle opened with
/// [`GENERIC_READ`].
pub unsafe fn read_file(handle: RawHandle, buf: &mut [u8]) -> Result<usize, Win32Error> {
    let mut bytes_read: u32 = 0;
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte writable buffer;
    // `bytes_read` is a valid out-pointer; `lpOverlapped = NULL` requests a
    // synchronous (blocking) read, a documented valid input for a handle
    // opened without `FILE_FLAG_OVERLAPPED`.
    let ok = unsafe {
        ReadFile(
            handle,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut bytes_read,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(bytes_read as usize)
    }
}

/// Write `buf` to `handle` — `WriteFile` against a regular file handle from
/// [`create_file`]/[`open_or_create_file`]/[`open_file`]. May write fewer
/// bytes than `buf.len()` (a short write); callers that need all of `buf`
/// written should loop.
///
/// # Safety
///
/// `handle` must be a currently-open, valid handle opened with
/// [`GENERIC_WRITE`].
pub unsafe fn write_file(handle: RawHandle, buf: &[u8]) -> Result<usize, Win32Error> {
    let mut bytes_written: u32 = 0;
    // SAFETY: `handle` is caller-supplied per this function's own safety
    // contract; `buf` is a valid, `buf.len()`-byte readable buffer;
    // `bytes_written` is a valid out-pointer; `lpOverlapped = NULL` requests
    // a synchronous (blocking) write, a documented valid input for a handle
    // opened without `FILE_FLAG_OVERLAPPED`.
    let ok = unsafe {
        WriteFile(
            handle,
            buf.as_ptr(),
            buf.len() as u32,
            &mut bytes_written,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(Win32Error::last())
    } else {
        Ok(bytes_written as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_reports_a_well_known_directory_as_a_directory() {
        let info = stat("C:\\Windows").expect("GetFileAttributesExW should succeed");
        assert!(
            info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            "C:\\Windows should be reported as a directory"
        );
    }

    #[test]
    fn stat_reports_none_for_a_nonexistent_path() {
        let err = stat("C:\\this-path-should-not-exist-rusty-win32-test\\nope")
            .expect_err("a nonexistent path should fail, not succeed");
        assert!(
            err == Win32Error::ERROR_FILE_NOT_FOUND || err == Win32Error::ERROR_PATH_NOT_FOUND,
            "expected a not-found error, got {err:?}"
        );
    }

    #[test]
    fn stat_reports_a_plausible_size_and_non_directory_attributes_for_a_real_file() {
        let path = std::env::temp_dir().join("rusty_win32_fs_stat_test.txt");
        std::fs::write(&path, b"rusty_win32").expect("writing the test file should succeed");

        let info = stat(path.to_str().unwrap()).expect("GetFileAttributesExW should succeed");
        assert_eq!(info.size, "rusty_win32".len() as u64);
        assert_eq!(
            info.attributes & FILE_ATTRIBUTE_DIRECTORY,
            0,
            "a plain file must not carry the directory attribute"
        );

        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn compressed_file_size_matches_stats_logical_size_for_an_uncompressed_file() {
        let path = std::env::temp_dir().join("rusty_win32_fs_compressed_size_test.txt");
        std::fs::write(&path, b"rusty_win32").expect("writing the test file should succeed");

        let logical_size = stat(path.to_str().unwrap())
            .expect("GetFileAttributesExW should succeed")
            .size;
        let on_disk_size = compressed_file_size(path.to_str().unwrap())
            .expect("GetCompressedFileSizeW should succeed");
        assert_eq!(
            on_disk_size, logical_size,
            "an uncompressed file's compressed size should equal its logical size"
        );

        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn stat_by_handle_reports_a_plausible_size_and_link_count_for_a_real_file() {
        use std::os::windows::io::AsRawHandle;

        let path = std::env::temp_dir().join("rusty_win32_fs_stat_by_handle_test.txt");
        std::fs::write(&path, b"rusty_win32_fs").expect("writing the test file should succeed");

        let file = std::fs::File::open(&path).expect("opening the test file should succeed");
        // SAFETY: `file`'s raw handle is a currently-open, valid handle to
        // a real file, owned by `file` for the duration of this call.
        let info = unsafe { stat_by_handle(file.as_raw_handle() as RawHandle) }
            .expect("GetFileInformationByHandle should succeed");
        assert_eq!(info.size, "rusty_win32_fs".len() as u64);
        assert_eq!(
            info.attributes & FILE_ATTRIBUTE_DIRECTORY,
            0,
            "a plain file must not carry the directory attribute"
        );
        assert!(
            info.link_count >= 1,
            "an ordinary file should report at least one link"
        );

        drop(file);
        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn create_symlink_then_readlink_round_trips() {
        let target = std::env::temp_dir().join("rusty_win32_fs_symlink_target.txt");
        let link = std::env::temp_dir().join("rusty_win32_fs_symlink_link.txt");
        // Clean up any leftovers from a previous failed run before starting,
        // so this test is re-runnable rather than failing on "already
        // exists" the second time.
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);

        std::fs::write(&target, b"rusty_win32 symlink target")
            .expect("writing the target file should succeed");
        let target_str = target.to_str().unwrap();
        let link_str = link.to_str().unwrap();

        match create_symlink(link_str, target_str, false) {
            Ok(()) => {}
            Err(Win32Error::ERROR_PRIVILEGE_NOT_HELD) => {
                let _ = std::fs::remove_file(&target);
                return;
            }
            Err(e) => panic!(
                "CreateSymbolicLinkW failed: {e:?} (expected success or ERROR_PRIVILEGE_NOT_HELD)"
            ),
        }

        let reported_target =
            readlink(link_str).expect("DeviceIoControl(FSCTL_GET_REPARSE_POINT) should succeed");
        assert_eq!(
            reported_target, target_str,
            "readlink should report exactly the target create_symlink was given"
        );

        // `GetFileAttributesExW` on a symlink path reports the reparse
        // point's own attributes without following it (Windows' `lstat`-
        // like behavior for this call) — the reparse-point bit must be
        // set, and its reported size is the reparse point data's own size,
        // not the target's content size.
        let link_info = stat(link_str).expect("GetFileAttributesExW on the symlink should succeed");
        assert!(
            link_info.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
            "a symlink path should report the reparse-point attribute"
        );

        std::fs::remove_file(&link).expect("cleaning up the symlink should succeed");
        std::fs::remove_file(&target).expect("cleaning up the target file should succeed");
    }

    #[test]
    fn readlink_fails_for_a_plain_file() {
        // A plain file isn't a reparse point at all, so
        // `DeviceIoControl(FSCTL_GET_REPARSE_POINT)` itself fails with
        // `ERROR_NOT_A_REPARSE_POINT` — this exercises that path, not
        // `readlink`'s own `ERROR_NOT_SUPPORTED` tag-mismatch branch (which
        // needs an actual non-symlink reparse point, e.g. a junction, out
        // of this test's scope to construct).
        let path = std::env::temp_dir().join("rusty_win32_fs_not_a_symlink.txt");
        std::fs::write(&path, b"plain file, not a reparse point")
            .expect("writing the test file should succeed");

        let err = readlink(path.to_str().unwrap())
            .expect_err("readlink on a plain file should fail, not succeed");
        assert_eq!(err, Win32Error::ERROR_NOT_A_REPARSE_POINT);

        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn final_path_resolves_to_the_files_own_name() {
        use std::os::windows::io::AsRawHandle;

        let path = std::env::temp_dir().join("rusty_win32_fs_final_path_test.txt");
        std::fs::write(&path, b"rusty_win32").expect("writing the test file should succeed");

        let file = std::fs::File::open(&path).expect("opening the test file should succeed");
        // SAFETY: `file`'s raw handle is a currently-open, valid file
        // handle, owned by `file` for the duration of this call.
        let resolved = unsafe { final_path(file.as_raw_handle() as RawHandle) }
            .expect("GetFinalPathNameByHandleW should succeed");
        assert!(
            resolved.ends_with("rusty_win32_fs_final_path_test.txt"),
            "resolved path {resolved:?} should end with the file's own name"
        );

        drop(file);
        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn read_dir_lists_files_created_in_a_temp_directory() {
        let dir = std::env::temp_dir().join("rusty_win32_read_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        std::fs::write(dir.join("a.txt"), b"hello").expect("writing a.txt should succeed");
        std::fs::write(dir.join("b.txt"), b"world!!").expect("writing b.txt should succeed");

        let pattern = alloc::format!("{}\\*", dir.display());
        let entries: alloc::vec::Vec<DirEntry> = read_dir(&pattern)
            .expect("FindFirstFileW should succeed")
            .collect::<Result<alloc::vec::Vec<_>, _>>()
            .expect("FindNextFileW should succeed for every entry");

        let names: alloc::vec::Vec<&str> = entries.iter().map(|e| e.file_name.as_str()).collect();
        assert!(names.contains(&"."), "got: {names:?}");
        assert!(names.contains(&".."), "got: {names:?}");
        assert!(names.contains(&"a.txt"), "got: {names:?}");
        assert!(names.contains(&"b.txt"), "got: {names:?}");

        let a = entries
            .iter()
            .find(|e| e.file_name == "a.txt")
            .expect("a.txt should be listed");
        assert_eq!(a.file_size, 5);
        let b = entries
            .iter()
            .find(|e| e.file_name == "b.txt")
            .expect("b.txt should be listed");
        assert_eq!(b.file_size, 7);

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn read_dir_fails_for_a_nonexistent_directory() {
        // The parent directory itself is missing, so `FindFirstFileW` reports
        // `ERROR_PATH_NOT_FOUND`, not `ERROR_FILE_NOT_FOUND` (that one's
        // reserved for an existing directory with no entries matching the
        // pattern).
        let err = read_dir(r"C:\this-directory-should-not-exist-rusty-win32-test\*").unwrap_err();
        assert_eq!(err, Win32Error::ERROR_PATH_NOT_FOUND);
    }

    #[test]
    fn copy_file_copies_content_to_a_new_path() {
        let dir = std::env::temp_dir().join("rusty_win32_copy_file_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let from = dir.join("source.txt");
        let to = dir.join("dest.txt");
        std::fs::write(&from, b"hello copy").expect("writing the source file should succeed");

        copy_file(from.to_str().unwrap(), to.to_str().unwrap(), false)
            .expect("CopyFileW should succeed");

        let copied = std::fs::read(&to).expect("the copied file should exist and be readable");
        assert_eq!(copied, b"hello copy");

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn copy_file_fails_when_fail_if_exists_and_destination_already_exists() {
        let dir = std::env::temp_dir().join("rusty_win32_copy_file_exists_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let from = dir.join("source.txt");
        let to = dir.join("dest.txt");
        std::fs::write(&from, b"hello").expect("writing the source file should succeed");
        std::fs::write(&to, b"already here").expect("writing the destination file should succeed");

        let err = copy_file(from.to_str().unwrap(), to.to_str().unwrap(), true).unwrap_err();
        assert_eq!(err, Win32Error::ERROR_FILE_EXISTS);

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn move_file_renames_the_file() {
        let dir = std::env::temp_dir().join("rusty_win32_move_file_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let from = dir.join("source.txt");
        let to = dir.join("dest.txt");
        std::fs::write(&from, b"hello move").expect("writing the source file should succeed");

        move_file(from.to_str().unwrap(), to.to_str().unwrap(), 0)
            .expect("MoveFileExW should succeed");

        assert!(!from.exists(), "the source path should no longer exist");
        let moved = std::fs::read(&to).expect("the destination file should exist and be readable");
        assert_eq!(moved, b"hello move");

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn move_file_fails_without_replace_existing_when_destination_already_exists() {
        let dir = std::env::temp_dir().join("rusty_win32_move_file_exists_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let from = dir.join("source.txt");
        let to = dir.join("dest.txt");
        std::fs::write(&from, b"hello").expect("writing the source file should succeed");
        std::fs::write(&to, b"already here").expect("writing the destination file should succeed");

        let err = move_file(from.to_str().unwrap(), to.to_str().unwrap(), 0).unwrap_err();
        assert_eq!(err, Win32Error::ERROR_ALREADY_EXISTS);

        move_file(
            from.to_str().unwrap(),
            to.to_str().unwrap(),
            MOVEFILE_REPLACE_EXISTING,
        )
        .expect("MoveFileExW should succeed with MOVEFILE_REPLACE_EXISTING");
        let moved = std::fs::read(&to).expect("the destination file should exist and be readable");
        assert_eq!(moved, b"hello");

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn delete_file_removes_an_existing_file() {
        let dir = std::env::temp_dir().join("rusty_win32_delete_file_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let path = dir.join("to_delete.txt");
        std::fs::write(&path, b"delete me").expect("writing the file should succeed");

        delete_file(path.to_str().unwrap()).expect("DeleteFileW should succeed");
        assert!(!path.exists(), "the file should no longer exist");

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn delete_file_fails_for_a_nonexistent_file() {
        let err = delete_file(r"C:\this-file-should-not-exist-rusty-win32-test.txt").unwrap_err();
        assert_eq!(err, Win32Error::ERROR_FILE_NOT_FOUND);
    }

    #[test]
    fn create_directory_then_remove_directory_round_trips() {
        let dir = std::env::temp_dir().join("rusty_win32_create_remove_dir_test");
        let _ = std::fs::remove_dir_all(&dir);

        create_directory(dir.to_str().unwrap()).expect("CreateDirectoryW should succeed");
        assert!(dir.is_dir(), "the directory should exist after creation");

        remove_directory(dir.to_str().unwrap()).expect("RemoveDirectoryW should succeed");
        assert!(!dir.exists(), "the directory should no longer exist");
    }

    #[test]
    fn create_directory_fails_when_it_already_exists() {
        let dir = std::env::temp_dir().join("rusty_win32_create_dir_exists_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");

        let err = create_directory(dir.to_str().unwrap()).unwrap_err();
        assert_eq!(err, Win32Error::ERROR_ALREADY_EXISTS);

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn create_hard_link_shares_content_and_increases_link_count() {
        use std::os::windows::io::AsRawHandle;

        let dir = std::env::temp_dir().join("rusty_win32_hard_link_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let target = dir.join("target.txt");
        let link = dir.join("link.txt");
        std::fs::write(&target, b"shared content").expect("writing the target file should succeed");

        create_hard_link(link.to_str().unwrap(), target.to_str().unwrap())
            .expect("CreateHardLinkW should succeed");

        let via_link = std::fs::read(&link).expect("reading through the hard link should succeed");
        assert_eq!(via_link, b"shared content");

        let file = std::fs::File::open(&target).expect("opening the target file should succeed");
        // SAFETY: `file`'s raw handle is a currently-open, valid handle to
        // a real file, owned by `file` for the duration of this call.
        let info = unsafe { stat_by_handle(file.as_raw_handle() as RawHandle) }
            .expect("GetFileInformationByHandle should succeed");
        assert_eq!(
            info.link_count, 2,
            "a fresh hard link should bring the count to 2"
        );

        drop(file);
        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn create_hard_link_fails_for_a_nonexistent_target() {
        let dir = std::env::temp_dir().join("rusty_win32_hard_link_missing_target_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("creating the test directory should succeed");
        let link = dir.join("link.txt");
        let target = dir.join("this-target-should-not-exist.txt");

        let err = create_hard_link(link.to_str().unwrap(), target.to_str().unwrap()).unwrap_err();
        assert_eq!(err, Win32Error::ERROR_FILE_NOT_FOUND);

        std::fs::remove_dir_all(&dir).expect("cleaning up the test directory should succeed");
    }

    #[test]
    fn lock_file_then_unlock_file_round_trips() {
        use std::os::windows::io::AsRawHandle;

        let path = std::env::temp_dir().join("rusty_win32_fs_lock_file_test.txt");
        std::fs::write(&path, b"0123456789").expect("writing the test file should succeed");

        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .open(&path)
            .expect("opening the test file should succeed");
        let handle = file.as_raw_handle() as RawHandle;

        // SAFETY: `handle` is a currently-open, valid file handle owned by
        // `file` for the duration of this test.
        unsafe { lock_file(handle, true, 0, 10) }.expect("LockFileEx should succeed");
        // SAFETY: same handle, same region just locked above.
        unsafe { unlock_file(handle, 0, 10) }.expect("UnlockFileEx should succeed");

        drop(file);
        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }

    #[test]
    fn lock_file_allows_simultaneous_shared_locks_from_different_handles() {
        use std::os::windows::io::AsRawHandle;

        let path = std::env::temp_dir().join("rusty_win32_fs_lock_file_shared_test.txt");
        std::fs::write(&path, b"0123456789").expect("writing the test file should succeed");

        let file_a = std::fs::File::open(&path).expect("opening the test file should succeed");
        let file_b = std::fs::File::open(&path)
            .expect("opening a second handle to the test file should succeed");
        let handle_a = file_a.as_raw_handle() as RawHandle;
        let handle_b = file_b.as_raw_handle() as RawHandle;

        // SAFETY: both handles are currently-open, valid file handles owned
        // by `file_a`/`file_b` for the duration of this test.
        unsafe { lock_file(handle_a, false, 0, 10) }
            .expect("LockFileEx should succeed for the first shared lock");
        unsafe { lock_file(handle_b, false, 0, 10) }.expect(
            "LockFileEx should succeed for a second, simultaneous shared lock over the same region",
        );

        // SAFETY: same handles, same regions just locked above.
        unsafe {
            unlock_file(handle_a, 0, 10).unwrap();
            unlock_file(handle_b, 0, 10).unwrap();
        }

        drop(file_a);
        drop(file_b);
        std::fs::remove_file(&path).expect("cleaning up the test file should succeed");
    }
}
