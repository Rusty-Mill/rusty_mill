//! Permitted raw ntdll surface — a Track-P-style admission (RFC v2 §2).
//!
//! Written rationale, as the tier doctrine requires: Win32 has no
//! handle-relative open — `CreateFileW` resolves every path from the
//! process-global namespace, which makes the capability-style `Dir` model
//! (RFC v2 §5.3) unimplementable at the Win32 layer. `NtCreateFile`'s
//! `OBJECT_ATTRIBUTES.RootDirectory` is the OS primitive for "open relative
//! to this handle" — the direct analog of Linux `openat` — and the RFC
//! names it explicitly as the legitimate ntdll admission for this crate.
//! This is the only API admitted here; each further ntdll export is its own
//! reviewed decision with its own rationale.

pub use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
pub use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
    FILE_OPEN_IF, FILE_OPEN_REPARSE_POINT, FILE_OVERWRITE, FILE_OVERWRITE_IF,
    FILE_SYNCHRONOUS_IO_NONALERT,
};
pub use windows_sys::Win32::System::Kernel::OBJ_CASE_INSENSITIVE;

// R2-equivalent containment (Rusty-Mill fs slice), Linux `openat2
// RESOLVE_NO_SYMLINKS`'s NT counterpart: `OBJ_DONT_REPARSE`
// (`OBJECT_ATTRIBUTES.Attributes`, Windows 10 1607+/SDK 10.0.14393+)
// rejects *any* reparse point NtCreateFile would otherwise transparently
// resolve — every intermediate path component, not just the terminal
// one — failing the call with `STATUS_REPARSE_POINT_ENCOUNTERED`
// (`win32_surface`'s own admission) instead. Scoped in `sys::nt` to
// `open_dir`/`create_dir`'s directory-anchored opens only, mirroring the
// Linux side's own scope decision for the identical reason (see
// `sys::fdio::openat_r2`'s doc comment) — `docs/behavior/fs.md` already
// promises `open`/`access` follow a terminal symlink transparently, and
// this flag does not distinguish "terminal" from "intermediate" the way
// a plain `O_NOFOLLOW`/`FILE_OPEN_REPARSE_POINT` would, so applying it
// to those two would silently break that promise rather than extend it.
pub use windows_sys::Win32::System::Kernel::OBJ_DONT_REPARSE;

// Second ntdll admission (D11, convergence roadmap Phase 3): a live
// windows-latest CI run proved `SetFileInformationByHandle` (the Win32
// kernel32 wrapper) rejects a non-null `FILE_RENAME_INFO.RootDirectory`
// with ERROR_INVALID_PARAMETER for the classic `FileRenameInfo` class —
// handle-relative rename is a Win32-layer restriction, not an NT one.
// `NtSetInformationFile` with `FileRenameInformation` is the direct NT
// analog of `NtCreateFile`'s own `OBJECT_ATTRIBUTES.RootDirectory`
// admission above: the OS primitive this capability model actually
// needs, one layer down from where the Win32 wrapper stops it.
pub use windows_sys::Wdk::Storage::FileSystem::{
    FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION, FILE_RENAME_INFORMATION_0,
};

// Third admission (symlink slice): not an ntdll function this time, but
// the same discipline — `REPARSE_DATA_BUFFER` is the kernel-defined
// (winioctl.h) layout `FSCTL_SET_REPARSE_POINT`/`FSCTL_GET_REPARSE_POINT`
// pass through `DeviceIoControl` (a Win32 call, `ffi::win32_surface`);
// windows-sys carries the struct here under Wdk rather than Win32 since
// it originates from the driver-facing headers. `SYMLINK_FLAG_RELATIVE`
// is the one bit distinguishing a relative-target symlink from an
// absolute one in `REPARSE_DATA_BUFFER_0_2`.
pub use windows_sys::Wdk::Storage::FileSystem::{
    REPARSE_DATA_BUFFER, REPARSE_DATA_BUFFER_0, REPARSE_DATA_BUFFER_0_2, SYMLINK_FLAG_RELATIVE,
};
