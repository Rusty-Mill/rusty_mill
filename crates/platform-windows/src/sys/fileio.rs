//! Handle-based file operations: read/write, metadata and enumeration via
//! `GetFileInformationByHandleEx`, deletion via `SetFileInformationByHandle`
//! (delete-on-close), and the single ambient-path entry point that opens a
//! root directory capability.

#![allow(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::time::{Duration, SystemTime};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::fs::FileType;

use crate::ffi::nt_surface as nt;
use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::sys::handle::OwnedWinHandle;
use crate::sys::nt as ntsys;
use crate::util::wide::{from_wide, to_wide_nt_component, to_wide_nul, to_wide_raw};

/// Open an absolute directory path as a root capability handle. The only
/// place an ambient path enters this backend (RFC v2 §5.3) — everything
/// after is relative to a handle via `sys::nt`.
pub fn open_ambient_dir(path: &OsStr) -> Result<OwnedWinHandle> {
    let wide = to_wide_nul(path);
    // SAFETY: `wide` is a valid NUL-terminated UTF-16 buffer outliving the
    // call; all other arguments are documented-valid constants or null.
    let handle = unsafe {
        w::CreateFileW(
            wide.as_ptr(),
            w::FILE_LIST_DIRECTORY | w::FILE_READ_ATTRIBUTES | w::FILE_TRAVERSE | w::SYNCHRONIZE,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE | w::FILE_SHARE_DELETE,
            std::ptr::null(),
            w::OPEN_EXISTING,
            // Required to open a directory handle at all; without it
            // CreateFileW refuses directories by design.
            w::FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    OwnedWinHandle::from_raw(handle).ok_or_else(|| errmap::last_win32_err("CreateFileW", path))
}

/// `ReadFile` into `buf` — Track W: `rusty_win32::fs::read_file` (D-15).
///
/// The first replaced call family, chosen to mirror Track P's own first
/// slice (`platform-linux`'s `sys::fdio::read`/`write`) so the two
/// adoptions stay comparable: the smallest possible surface, exercised by
/// essentially every test in the suite, with a `Result<usize>` contract
/// that leaves nothing to interpret.
///
/// Same `ReadFile` call underneath, from the other side of the tier
/// doctrine: windows-sys' metadata-generated declaration versus
/// rusty_win32's hand-written `extern "system"` one. What differs is the
/// error path — see [`errmap::trackw_err`] — and the length clamp below.
#[cfg(feature = "track-w")]
pub fn read(handle: &OwnedWinHandle, buf: &mut [u8]) -> Result<usize> {
    // `rusty_win32::fs::read_file` passes `buf.len() as u32` straight to
    // `ReadFile`, so an oversized slice would *wrap* rather than saturate
    // (a 4 GiB + 1 buffer reading one byte, or zero). Clamp on this side to
    // keep the two arms' contract identical to the `u32::try_from(…)
    // .unwrap_or(u32::MAX)` the windows-sys arm does inline. Short reads
    // are already this function's contract, so a clamp is not a behavior
    // change — only a bound on how much one call can move.
    let len = buf.len().min(u32::MAX as usize);
    // SAFETY: `read_file`'s contract is a currently-open, valid, readable
    // handle. `OwnedWinHandle`'s own type invariant is exactly that — it is
    // constructed only from a freshly-returned valid handle and closes it
    // once, on drop — and `&self` here proves it has not dropped yet.
    match unsafe { rusty_win32::fs::read_file(handle.as_raw(), &mut buf[..len]) } {
        Ok(n) => Ok(n),
        // A pipe whose write side has fully closed reports BROKEN_PIPE on
        // read; that IS end-of-file for pipes — mirroring unix read()
        // returning 0. Same rule as the windows-sys arm below, restated
        // against the code the wrapper returned rather than a fresh
        // `GetLastError` read.
        Err(e) if e.code() == w::ERROR_BROKEN_PIPE => Ok(0),
        Err(e) => Err(errmap::trackw_err("ReadFile", e)),
    }
}

/// `ReadFile` into `buf`.
#[cfg(not(feature = "track-w"))]
pub fn read(handle: &OwnedWinHandle, buf: &mut [u8]) -> Result<usize> {
    let mut n: u32 = 0;
    let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
    // SAFETY: `buf` is a valid writable region of at least `len` bytes and
    // `n` a valid out-pointer, both outliving the call; the handle is open
    // and synchronous (no OVERLAPPED), so the null overlapped is valid.
    let ok = unsafe {
        w::ReadFile(
            handle.as_raw(),
            buf.as_mut_ptr(),
            len,
            &mut n,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        // SAFETY: `GetLastError` takes no arguments and has no
        // preconditions.
        let code = unsafe { w::GetLastError() };
        if code == w::ERROR_BROKEN_PIPE {
            // A pipe whose write side has fully closed reports
            // BROKEN_PIPE on read; that IS end-of-file for pipes —
            // mirroring unix read() returning 0.
            return Ok(0);
        }
        return Err(errmap::last_win32_err("ReadFile", OsStr::new("")));
    }
    Ok(n as usize)
}

/// `WriteFile` from `buf` — Track W: `rusty_win32::fs::write_file` (D-15).
#[cfg(feature = "track-w")]
pub fn write(handle: &OwnedWinHandle, buf: &[u8]) -> Result<usize> {
    // Clamped for the same reason as `read` above.
    let len = buf.len().min(u32::MAX as usize);
    // SAFETY: as in `read` above — `OwnedWinHandle`'s type invariant is
    // `write_file`'s safety contract, and `&self` proves it is still open.
    unsafe { rusty_win32::fs::write_file(handle.as_raw(), &buf[..len]) }
        .map_err(|e| errmap::trackw_err("WriteFile", e))
}

/// `WriteFile` from `buf`.
#[cfg(not(feature = "track-w"))]
pub fn write(handle: &OwnedWinHandle, buf: &[u8]) -> Result<usize> {
    let mut n: u32 = 0;
    let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
    // SAFETY: `buf` is a valid readable region of at least `len` bytes and
    // `n` a valid out-pointer, both outliving the call; the handle is open
    // and synchronous, so the null overlapped is valid.
    let ok = unsafe {
        w::WriteFile(
            handle.as_raw(),
            buf.as_ptr(),
            len,
            &mut n,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("WriteFile", OsStr::new("")));
    }
    Ok(n as usize)
}

fn file_type_of_attributes(attrs: u32) -> FileType {
    // Any reparse point is classified Symlink for now — distinguishing
    // IO_REPARSE_TAG_SYMLINK from junctions and other tags is deferred
    // until a consumer needs the distinction (consumer gate, RFC v2 §3).
    if attrs & w::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        FileType::Symlink
    } else if attrs & w::FILE_ATTRIBUTE_DIRECTORY != 0 {
        FileType::Dir
    } else {
        FileType::File
    }
}

/// Windows `FILETIME` (100-nanosecond intervals since 1601-01-01 UTC)
/// converted to [`SystemTime`] — the same `1601`-vs-`1970` epoch
/// translation `FileBasicInfo::LastWriteTime` always needs, since
/// `platform::fs::Metadata::modified` is epoch-agnostic (`SystemTime`),
/// not a raw Windows tick count.
fn filetime_to_system_time(filetime: i64) -> SystemTime {
    // 100ns intervals between the FILETIME epoch (1601) and the Unix
    // epoch (1970).
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    let unix_100ns = filetime - EPOCH_DIFF_100NS;
    if unix_100ns >= 0 {
        let secs = (unix_100ns / 10_000_000) as u64;
        let nanos = ((unix_100ns % 10_000_000) * 100) as u32;
        SystemTime::UNIX_EPOCH + Duration::new(secs, nanos)
    } else {
        let abs = (-unix_100ns) as u64;
        let secs = abs / 10_000_000;
        let nanos = ((abs % 10_000_000) * 100) as u32;
        SystemTime::UNIX_EPOCH - Duration::new(secs, nanos)
    }
}

/// (file type, size, link count, mtime) for an open handle, via
/// `FileBasicInfo`'s attributes/`LastWriteTime` and
/// `file_standard_info`'s `EndOfFile`/`NumberOfLinks`.
pub fn metadata_by_handle(
    handle: &OwnedWinHandle,
    path: &OsStr,
) -> Result<(FileType, u64, u64, SystemTime)> {
    // SAFETY: `info` is a valid out-buffer of exactly the queried class's
    // size, outliving the call; the handle is open with at least
    // FILE_READ_ATTRIBUTES access.
    let basic: w::FILE_BASIC_INFO = unsafe {
        let mut info = std::mem::zeroed::<w::FILE_BASIC_INFO>();
        let ok = w::GetFileInformationByHandleEx(
            handle.as_raw(),
            w::FileBasicInfo,
            (&mut info as *mut w::FILE_BASIC_INFO).cast(),
            std::mem::size_of::<w::FILE_BASIC_INFO>() as u32,
        );
        if ok == 0 {
            return Err(errmap::last_win32_err("GetFileInformationByHandleEx", path));
        }
        info
    };
    let file_type = file_type_of_attributes(basic.FileAttributes);
    let (raw_len, nlink) = file_standard_info(handle, path)?;
    let len = if file_type == FileType::Dir {
        // Directories report no meaningful byte length; pin 0 across
        // backends rather than exposing an allocation-size accident.
        0
    } else {
        raw_len
    };
    let modified = filetime_to_system_time(basic.LastWriteTime);
    Ok((file_type, len, nlink, modified))
}

/// `(dwVolumeSerialNumber, fileIndex)` via `GetFileInformationByHandle`
/// — `test -ef`'s donor material (D11, faccessat slice's sibling),
/// wrapped into the opaque `platform::fs::FileId` by the `Dir` impl. The
/// same legacy 32-bit-serial + 64-bit-index pair
/// `std::os::windows::fs::MetadataExt::file_index` historically exposed
/// — good enough for equality comparison, this type's only contract.
pub fn file_id_by_handle(handle: &OwnedWinHandle, path: &OsStr) -> Result<(u64, u64)> {
    // SAFETY: `info` is a valid out-buffer of exactly
    // `BY_HANDLE_FILE_INFORMATION`'s size, outliving the call; the
    // handle is open with at least `FILE_READ_ATTRIBUTES` access.
    let info: w::BY_HANDLE_FILE_INFORMATION = unsafe {
        let mut info = std::mem::zeroed::<w::BY_HANDLE_FILE_INFORMATION>();
        let ok = w::GetFileInformationByHandle(handle.as_raw(), &mut info);
        if ok == 0 {
            return Err(errmap::last_win32_err("GetFileInformationByHandle", path));
        }
        info
    };
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((u64::from(info.dwVolumeSerialNumber), index))
}

/// `(EndOfFile, NumberOfLinks)` via `FILE_STANDARD_INFO`.
///
/// Not in the curated surface (`ffi::win32_surface`); `EndOfFile` alone
/// is also available as `GetFileSizeEx`, but that widens the surface
/// for one field when the standard-info layout already carries both
/// values this backend needs (`EndOfFile` for `Metadata::len`,
/// `NumberOfLinks` for `Metadata::nlink`, coreutils gap backlog #65's
/// `ls -l` donor material) in a single call.
fn file_standard_info(handle: &OwnedWinHandle, path: &OsStr) -> Result<(u64, u64)> {
    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }
    const FILE_STANDARD_INFO_CLASS: i32 = 1; // FileStandardInfo
                                             // SAFETY: `info` is a valid out-buffer at least as large as the
                                             // documented FILE_STANDARD_INFO layout, outliving the call; the handle
                                             // is open with at least FILE_READ_ATTRIBUTES access.
    let info: FileStandardInfo = unsafe {
        let mut info = std::mem::zeroed::<FileStandardInfo>();
        let ok = w::GetFileInformationByHandleEx(
            handle.as_raw(),
            FILE_STANDARD_INFO_CLASS,
            (&mut info as *mut FileStandardInfo).cast(),
            std::mem::size_of::<FileStandardInfo>() as u32,
        );
        if ok == 0 {
            return Err(errmap::last_win32_err("GetFileInformationByHandleEx", path));
        }
        info
    };
    Ok((info.end_of_file as u64, u64::from(info.number_of_links)))
}

/// Enumerate a directory handle's entries, excluding `.`/`..`. The handle
/// carries per-handle enumeration state, so callers pass a fresh handle
/// (the Linux backend re-opens `"."` for the same reason).
pub fn read_dir_entries(handle: &OwnedWinHandle) -> Result<Vec<(OsString, FileType)>> {
    let mut out = Vec::new();
    // Large enough for many entries per call; the kernel packs as many
    // FILE_FULL_DIR_INFO records as fit.
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // SAFETY: `buf` is a valid writable region of its stated size,
        // outliving the call; the handle is a directory opened with
        // FILE_LIST_DIRECTORY access.
        let ok = unsafe {
            w::GetFileInformationByHandleEx(
                handle.as_raw(),
                w::FileFullDirectoryInfo,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if ok == 0 {
            // SAFETY: `GetLastError` takes no arguments and has no
            // preconditions.
            let code = unsafe { w::GetLastError() };
            if code == w::ERROR_NO_MORE_FILES {
                return Ok(out);
            }
            return Err(errmap::last_win32_err(
                "GetFileInformationByHandleEx",
                OsStr::new(""),
            ));
        }
        let mut offset = 0usize;
        loop {
            // SAFETY: on success the kernel wrote a chain of
            // FILE_FULL_DIR_INFO records into `buf` starting at offset 0,
            // each 8-aligned, linked by NextEntryOffset, with
            // FileNameLength bytes of UTF-16 name immediately at FileName;
            // `offset` only ever takes values from that chain, so the
            // reads below stay inside the initialized region.
            let (name, attrs, next) = unsafe {
                let entry = buf.as_ptr().add(offset).cast::<w::FILE_FULL_DIR_INFO>();
                let name_len = (*entry).FileNameLength as usize / 2;
                let name_ptr = std::ptr::addr_of!((*entry).FileName).cast::<u16>();
                let name_units: Vec<u16> = std::slice::from_raw_parts(name_ptr, name_len).to_vec();
                (
                    name_units,
                    (*entry).FileAttributes,
                    (*entry).NextEntryOffset as usize,
                )
            };
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                out.push((from_wide(&name), file_type_of_attributes(attrs)));
            }
            if next == 0 {
                break;
            }
            offset += next;
        }
    }
}

/// Mark an open handle's file for deletion at last close (the disposition
/// path both `remove_file` and `remove_dir` take; a non-empty directory is
/// refused here by the OS with ERROR_DIR_NOT_EMPTY).
pub fn mark_delete(handle: &OwnedWinHandle, path: &OsStr) -> Result<()> {
    let info = w::FILE_DISPOSITION_INFO { DeleteFile: 1 };
    // SAFETY: `info` is a valid, fully initialized FILE_DISPOSITION_INFO
    // outliving the call; the handle is open with DELETE access.
    let ok = unsafe {
        w::SetFileInformationByHandle(
            handle.as_raw(),
            w::FileDispositionInfo,
            (&info as *const w::FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<w::FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("SetFileInformationByHandle", path));
    }
    Ok(())
}

/// `FlushFileBuffers` — durability (`File::sync_all`).
pub fn sync_all(handle: &OwnedWinHandle) -> Result<()> {
    // SAFETY: the handle is a live, open, writable file handle.
    let ok = unsafe { w::FlushFileBuffers(handle.as_raw()) };
    if ok == 0 {
        return Err(errmap::last_win32_err("FlushFileBuffers", OsStr::new("")));
    }
    Ok(())
}

/// Rename `from` (opened with DELETE access, relative to `dir`) to `to`
/// (a name relative to that same `dir`), via `NtSetInformationFile` +
/// `FILE_RENAME_INFORMATION`'s `RootDirectory` field — the
/// handle-relative rename this backend's capability model needs (D11,
/// convergence roadmap Phase 3), since there is no ambient path to hand
/// `MoveFileExW`.
///
/// **Not** the Win32 `SetFileInformationByHandle`: a real windows-latest
/// CI run proved that wrapper rejects a non-null `RootDirectory` for the
/// classic `FileRenameInfo` class with `ERROR_INVALID_PARAMETER` —
/// handle-relative rename turns out to be a Win32-layer restriction, not
/// an NT one (a second, narrower ntdll admission, `ffi::nt_surface`).
///
/// `FILE_RENAME_INFORMATION` ends in a flexible array member
/// (`FileName`) at an offset **not** equal to `size_of::<…>()` — the
/// struct's own alignment (8, from the `HANDLE` field) pads its *end*,
/// past `FileName`, so `size_of` overshoots by that padding (the first
/// bug this function shipped with, before the Win32-vs-NT one). The
/// offset is computed via `addr_of!` pointer subtraction on a zeroed
/// instance — the compiler's own layout, not a hand-derived guess, and
/// available without `offset_of!` (unavailable at this workspace's 1.75
/// MSRV; stabilized in 1.77).
pub fn rename(dir: &OwnedWinHandle, from: &OsStr, to: &OsStr, replace: bool) -> Result<()> {
    // The target may be a file or a directory; try the file disposition
    // first (the common case) and fall back to the directory one rather
    // than requiring the caller to know which (mirroring the Linux
    // backend, where renameat2 doesn't care either).
    let handle = ntsys::open_relative(
        dir,
        from,
        w::DELETE | w::SYNCHRONIZE,
        nt::FILE_OPEN,
        nt::FILE_NON_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
    )
    .or_else(|_| {
        ntsys::open_relative(
            dir,
            from,
            w::DELETE | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )
    })?;

    let name = to_wide_nt_component(to);
    let name_bytes = (name.len() * 2) as u32;

    // SAFETY: an all-zero bit pattern is a valid `FILE_RENAME_INFORMATION`
    // (BOOLEAN/HANDLE/u32/u16 are all valid at all-zeroes); never read
    // through the union, only used to compute `FileName`'s real offset.
    let probe: nt::FILE_RENAME_INFORMATION = unsafe { std::mem::zeroed() };
    let base = std::ptr::addr_of!(probe) as usize;
    let name_field = std::ptr::addr_of!(probe.FileName) as usize;
    let name_offset = name_field - base;

    // SAFETY: an all-zero bit pattern is a valid `FILE_RENAME_INFORMATION`;
    // every field is then overwritten below before this value is read as
    // bytes.
    let mut header: nt::FILE_RENAME_INFORMATION = unsafe { std::mem::zeroed() };
    header.Anonymous = nt::FILE_RENAME_INFORMATION_0 {
        ReplaceIfExists: u8::from(replace),
    };
    header.RootDirectory = dir.as_raw();
    header.FileNameLength = name_bytes;

    let mut buf = vec![0u8; name_offset + name_bytes as usize];
    // SAFETY: `buf` is at least `name_offset` bytes (allocated above);
    // copying exactly the header portion up to (not including)
    // `FileName` is in-bounds on both sides and leaves `FileName`
    // itself untouched — it is overwritten fully by the loop below.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&header as *const nt::FILE_RENAME_INFORMATION).cast::<u8>(),
            buf.as_mut_ptr(),
            name_offset,
        );
    }
    // `FILE_RENAME_INFORMATION`'s `FileName` is raw UTF-16 code units,
    // not a `u16` slice the ABI guarantees any transmute safety for;
    // write each unit's bytes explicitly rather than reinterpreting the
    // slice.
    for (i, unit) in name.iter().enumerate() {
        let at = name_offset + i * 2;
        buf[at..at + 2].copy_from_slice(&unit.to_le_bytes());
    }

    // SAFETY: `IO_STATUS_BLOCK` is plain-old-data for which all-zeroes is
    // a valid starting value, overwritten by the call below.
    let mut iosb: w::IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    // SAFETY: `iosb` is a valid out-pointer; `buf` holds a fully
    // populated `FILE_RENAME_INFORMATION` at its start followed by the
    // wide filename at its real, compiler-computed offset, with its
    // length passed exactly as `buf.len()`; `handle` is open with
    // DELETE access.
    let status = unsafe {
        nt::NtSetInformationFile(
            handle.as_raw(),
            &mut iosb,
            buf.as_ptr().cast(),
            buf.len() as u32,
            nt::FileRenameInformation,
        )
    };
    if status != w::STATUS_SUCCESS {
        return Err(errmap::nt_err(status, "NtSetInformationFile", to));
    }
    Ok(())
}

/// Compute the NT substitute name for a symlink target: the wire form
/// the filesystem's reparse-point engine resolves, distinct from the
/// print name (what a consumer typed, and what `read_link` hands back
/// unchanged). A relative target is stored as-is with
/// `SYMLINK_FLAG_RELATIVE`; an absolute one — a drive path (`C:\...`) or
/// a UNC path (`\\server\share`) — needs the `\??\` NT-namespace prefix
/// (`\??\C:\...`, `\??\UNC\server\share`) that `CreateSymbolicLinkW`
/// itself adds internally before reaching this same FSCTL. Returns
/// `(substitute_name, is_absolute)`.
fn nt_substitute_name(target: &[u16]) -> (Vec<u16>, bool) {
    let colon = u16::from(b':');
    let backslash = u16::from(b'\\');
    if target.len() >= 2 && target[1] == colon {
        let mut s: Vec<u16> = "\\??\\".encode_utf16().collect();
        s.extend_from_slice(target);
        (s, true)
    } else if target.len() >= 2 && target[0] == backslash && target[1] == backslash {
        let mut s: Vec<u16> = "\\??\\UNC".encode_utf16().collect();
        s.extend_from_slice(&target[1..]); // one leading "\" survives from target
        (s, true)
    } else {
        (target.to_vec(), false)
    }
}

/// The `REPARSE_DATA_BUFFER` header's real byte size (up to, not
/// including, the `Anonymous` union) and the real byte offset of the
/// symlink path buffer within it — both `addr_of!`-derived on a zeroed
/// probe rather than hand-computed, the same technique `rename`'s
/// `FILE_RENAME_INFORMATION` offset uses (`offset_of!` is unavailable at
/// this workspace's 1.75 MSRV).
fn reparse_offsets() -> (usize, usize) {
    // SAFETY: an all-zero bit pattern is a valid `REPARSE_DATA_BUFFER`
    // (the union's largest member is plain integer fields and a
    // flexible `u16` array — all valid at all-zeroes); never read
    // through the union, only used to compute real field offsets.
    unsafe {
        let probe: nt::REPARSE_DATA_BUFFER = std::mem::zeroed();
        let base = std::ptr::addr_of!(probe) as usize;
        let payload_offset = std::ptr::addr_of!(probe.Anonymous) as usize - base;
        let path_buffer_offset =
            std::ptr::addr_of!(probe.Anonymous.SymbolicLinkReparseBuffer.PathBuffer) as usize
                - base;
        (payload_offset, path_buffer_offset)
    }
}

/// Create `link_name` (relative to `dir`) as an NT reparse-point symlink
/// storing `target` (D11, symlink slice). `is_dir` selects the
/// file-vs-directory reparse-point type — an NT/FSCTL requirement with
/// no POSIX analog; see [`platform::fs::Dir::symlink`]'s doc comment for
/// how the caller decides it.
///
/// `FSCTL_SET_REPARSE_POINT`'s `REPARSE_DATA_BUFFER` is the standard,
/// widely documented NT symlink recipe (the same one `mklink`/
/// `CreateSymbolicLinkW` ultimately drive): the print name is `target`
/// unchanged (`to_wide_raw` — no separator normalization, so
/// `read_link` gets an exact byte round trip, matching Linux's
/// `readlinkat` never normalizing either); `nt_substitute_name` picks
/// the wire form the filesystem driver actually resolves from a
/// **separately** `\`-normalized copy (`to_wide_nt_component`), since
/// that copy is discarded after computing the substitute name and never
/// itself returned to a caller. Both are packed substitute-then-print
/// into the flexible `PathBuffer` at its real, compiler-computed offset.
pub fn symlink(
    dir: &OwnedWinHandle,
    link_name: &OsStr,
    target: &OsStr,
    is_dir: bool,
) -> Result<()> {
    let handle = ntsys::open_relative(
        dir,
        link_name,
        w::FILE_GENERIC_WRITE | w::SYNCHRONIZE,
        nt::FILE_CREATE,
        (if is_dir {
            nt::FILE_DIRECTORY_FILE
        } else {
            nt::FILE_NON_DIRECTORY_FILE
        }) | nt::FILE_SYNCHRONOUS_IO_NONALERT
            | nt::FILE_OPEN_REPARSE_POINT,
    )?;

    let print_name = to_wide_raw(target);
    let (substitute_name, is_absolute) = nt_substitute_name(&to_wide_nt_component(target));
    let flags = if is_absolute {
        0u32
    } else {
        nt::SYMLINK_FLAG_RELATIVE
    };

    let sub_bytes = (substitute_name.len() * 2) as u16;
    let print_bytes = (print_name.len() * 2) as u16;
    let (payload_offset, path_buffer_offset) = reparse_offsets();
    debug_assert_eq!(path_buffer_offset - payload_offset, 12);

    let total_len = path_buffer_offset + sub_bytes as usize + print_bytes as usize;
    let mut buf = vec![0u8; total_len];

    buf[0..4].copy_from_slice(&w::IO_REPARSE_TAG_SYMLINK.to_le_bytes());
    let reparse_data_length = (total_len - payload_offset) as u16;
    buf[4..6].copy_from_slice(&reparse_data_length.to_le_bytes());
    // Reserved @ [6..8] stays zero.

    let p = payload_offset;
    buf[p..p + 2].copy_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
    buf[p + 2..p + 4].copy_from_slice(&sub_bytes.to_le_bytes()); // SubstituteNameLength
    buf[p + 4..p + 6].copy_from_slice(&sub_bytes.to_le_bytes()); // PrintNameOffset
    buf[p + 6..p + 8].copy_from_slice(&print_bytes.to_le_bytes()); // PrintNameLength
    buf[p + 8..p + 12].copy_from_slice(&flags.to_le_bytes()); // Flags

    for (i, unit) in substitute_name.iter().enumerate() {
        let at = path_buffer_offset + i * 2;
        buf[at..at + 2].copy_from_slice(&unit.to_le_bytes());
    }
    for (i, unit) in print_name.iter().enumerate() {
        let at = path_buffer_offset + sub_bytes as usize + i * 2;
        buf[at..at + 2].copy_from_slice(&unit.to_le_bytes());
    }

    let mut bytes_returned: u32 = 0;
    // SAFETY: `buf` holds a fully populated `REPARSE_DATA_BUFFER` with
    // its `SymbolicLinkReparseBuffer` variant, sized exactly `buf.len()`,
    // outliving the call; no output buffer is requested (null/0); the
    // handle was just created with write access for this purpose.
    let ok = unsafe {
        w::DeviceIoControl(
            handle.as_raw(),
            w::FSCTL_SET_REPARSE_POINT,
            buf.as_ptr().cast(),
            buf.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("DeviceIoControl", link_name));
    }
    Ok(())
}

/// Read the stored target of the reparse point at `rel` (`FSCTL_GET_REPARSE_POINT`,
/// the `symlink` companion). Returns the print name — the original,
/// unprefixed bytes `symlink` was given, an exact round trip — not the
/// substitute name, which carries the `\??\` NT-namespace prefix for
/// absolute targets. `rel` must be a reparse point of the symlink tag;
/// anything else is `InvalidInput`, mirroring Linux's `readlinkat`
/// refusing a non-symlink with `EINVAL`.
pub fn read_link(dir: &OwnedWinHandle, rel: &OsStr) -> Result<OsString> {
    let handle = ntsys::open_relative(
        dir,
        rel,
        w::FILE_READ_ATTRIBUTES | w::SYNCHRONIZE,
        nt::FILE_OPEN,
        nt::FILE_SYNCHRONOUS_IO_NONALERT | nt::FILE_OPEN_REPARSE_POINT,
    )?;

    let mut buf = vec![0u8; w::MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize];
    let mut bytes_returned: u32 = 0;
    // SAFETY: `buf` is a valid writable region of its stated size,
    // outliving the call; `bytes_returned` a valid out-pointer; the
    // handle is open on a reparse point with read-attributes access.
    let ok = unsafe {
        w::DeviceIoControl(
            handle.as_raw(),
            w::FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(errmap::last_win32_err("DeviceIoControl", rel));
    }
    if (bytes_returned as usize) < 8 {
        return Err(
            PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "read_link").with_path(rel),
        );
    }

    // Direct indexing, not `try_into().expect(...)`: `buf` is a fixed
    // `MAXIMUM_REPARSE_DATA_BUFFER_SIZE`-byte allocation, so these
    // offsets are always in bounds and the array-length conversion
    // can't fail — no fallible operation to unwrap.
    let tag = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if tag != w::IO_REPARSE_TAG_SYMLINK {
        return Err(
            PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "read_link").with_path(rel),
        );
    }

    let (payload_offset, path_buffer_offset) = reparse_offsets();
    let p = payload_offset;
    let print_name_offset = u16::from_le_bytes([buf[p + 4], buf[p + 5]]) as usize;
    let print_name_length = u16::from_le_bytes([buf[p + 6], buf[p + 7]]) as usize;

    let start = path_buffer_offset + print_name_offset;
    let end = start + print_name_length;
    let units: Vec<u16> = buf[start..end]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(from_wide(&units))
}
