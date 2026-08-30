//! Handle-relative opens over `NtCreateFile` — the Linux backend's
//! `openat` counterpart, and the one place this crate calls ntdll (see
//! `ffi::nt_surface` for the admission rationale).

#![allow(unsafe_code)]

use std::ffi::OsStr;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::ffi::nt_surface as nt;
use crate::ffi::win32_surface as w;
use crate::sys::errmap;
use crate::sys::handle::OwnedWinHandle;
use crate::util::wide::to_wide_nt_component;

/// Open `rel` relative to the directory handle `root` — the `openat`
/// analog. An empty `rel` re-opens `root`'s own directory object (the NT
/// namespace's documented behavior for an empty `ObjectName` with
/// `RootDirectory` set), which enumeration uses the way the Linux backend
/// re-opens `"."`.
pub fn open_relative(
    root: &OwnedWinHandle,
    rel: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
) -> Result<OwnedWinHandle> {
    open_relative_attrs(
        root,
        rel,
        access,
        disposition,
        options,
        nt::OBJ_CASE_INSENSITIVE as u32,
    )
}

/// R2-equivalent containment (Rusty-Mill fs slice): the same
/// handle-relative open as [`open_relative`], with `OBJ_DONT_REPARSE`
/// added to `Attributes` — any reparse point `NtCreateFile` encounters
/// while resolving `rel`, intermediate or terminal, fails the call with
/// `STATUS_REPARSE_POINT_ENCOUNTERED` (mapped to `ErrorKind::
/// FilesystemLoop`) instead of being transparently followed. Used only
/// by [`crate::fs::WindowsDir::open_dir`]/`create_dir` — see those call
/// sites' own doc comments for why `open`/`access` are left on plain
/// [`open_relative`] instead.
pub fn open_relative_r2(
    root: &OwnedWinHandle,
    rel: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
) -> Result<OwnedWinHandle> {
    open_relative_attrs(
        root,
        rel,
        access,
        disposition,
        options,
        (nt::OBJ_CASE_INSENSITIVE | nt::OBJ_DONT_REPARSE) as u32,
    )
}

fn open_relative_attrs(
    root: &OwnedWinHandle,
    rel: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
    object_attributes: u32,
) -> Result<OwnedWinHandle> {
    let wide = to_wide_nt_component(rel);
    let byte_len = wide.len() * 2;
    if byte_len > u16::MAX as usize {
        return Err(
            PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "NtCreateFile")
                .with_path(rel),
        );
    }
    let name = w::UNICODE_STRING {
        Length: byte_len as u16,
        MaximumLength: byte_len as u16,
        Buffer: wide.as_ptr().cast_mut(),
    };
    let attrs = nt::OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<nt::OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: root.as_raw(),
        ObjectName: &name,
        // Match Win32's (and every consumer's) expectation on NTFS: name
        // lookup is case-insensitive. Case-sensitivity policy questions
        // belong to consumers (rush's shell-host layer), not here.
        // `object_attributes` additionally carries `OBJ_DONT_REPARSE` for
        // the `open_relative_r2` caller only.
        Attributes: object_attributes,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: w::HANDLE = std::ptr::null_mut();
    // SAFETY: IO_STATUS_BLOCK is plain-old-data (a status/pointer union
    // plus a usize) for which all-zeroes is a valid value; NtCreateFile
    // overwrites it before we ever read it.
    let mut iosb: w::IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };

    // SAFETY: `handle`/`iosb` are valid out-pointers; `attrs` points at a
    // fully initialized OBJECT_ATTRIBUTES whose `ObjectName` references
    // `name`, whose `Buffer` references `wide` — all three outlive the
    // call; `root` holds a valid directory handle for RootDirectory; the
    // null allocation-size and EA pointers are documented-valid.
    let status = unsafe {
        nt::NtCreateFile(
            &mut handle,
            access,
            &attrs,
            &mut iosb,
            std::ptr::null(),
            w::FILE_ATTRIBUTE_NORMAL,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE | w::FILE_SHARE_DELETE,
            disposition,
            options,
            std::ptr::null(),
            0,
        )
    };
    if status != w::STATUS_SUCCESS {
        return Err(errmap::nt_err(status, "NtCreateFile", rel));
    }
    OwnedWinHandle::from_raw(handle).ok_or_else(|| {
        PlatformError::new(ErrorKind::Other, OsCode::None, "NtCreateFile").with_path(rel)
    })
}
