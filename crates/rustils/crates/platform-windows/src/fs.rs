//! `Dir`/`File` trait impls over the sys layer — the Linux backend's
//! mirror, with `NtCreateFile` handle-relative opens standing in for the
//! `openat` family (RFC v2 §5.3).
//!
//! **Resolution safety (Rusty-Mill R-scale):** `open`/`metadata`/
//! `access`/`unix_mode`/`file_id`/`read_link`/`rename`/
//! `rename_no_replace`/`remove_file`/`remove_dir`/`read_dir` stay R1
//! ("link-confined, not atomic" — anchored to the capability handle via
//! `RootDirectory`, but a reparse point anywhere in resolution,
//! intermediate or terminal, is transparently followed; `open`/`access`
//! deliberately kept this way — `docs/behavior/fs.md`'s promise).
//! `open_dir`/`create_dir` are **R2-equivalent**: `sys::nt::
//! open_relative_r2` adds `OBJ_DONT_REPARSE` to `NtCreateFile`'s
//! `OBJECT_ATTRIBUTES.Attributes`, rejecting any reparse point
//! encountered while resolving `rel` (`STATUS_REPARSE_POINT_ENCOUNTERED`
//! → `ErrorKind::FilesystemLoop`) rather than following it — the same
//! containment `sys::fdio::openat_r2`'s `RESOLVE_NO_SYMLINKS` gives the
//! Linux backend, unconditionally rather than kernel-version-gated (no
//! Windows analog to `openat2`'s `ENOSYS` fallback dance: `OBJ_DONT_REPARSE`
//! has shipped since Windows 10 1607/SDK 10.0.14393, well below anything
//! this workspace targets). No `RESOLVE_NO_XDEV` equivalent requested —
//! NT has no single-flag "refuse to cross a volume boundary" analog
//! admitted here, so this is R1's link-confinement made atomic, not full
//! R2 mount-confinement; described as "R2-equivalent" rather than R2
//! throughout this module's docs for that reason.
//!
//! **Durability (Rusty-Mill D-scale):** `File::sync_all`
//! (`FlushFileBuffers`) is D1. `Dir::write_atomic` stays **D1** on this
//! backend — no `sync_dir` override — pending research into whether
//! `FlushFileBuffers` on a directory handle has any documented,
//! verifiable effect on NTFS; see the `Dir for WindowsDir` impl block's
//! own doc comment for the full reasoning.

use std::ffi::{OsStr, OsString};
use std::os::windows::io::{AsHandle, BorrowedHandle, OwnedHandle};
use std::path::Path;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::fs::{
    AccessMode, AnonymousFile, Dir, DirEntry, File, FileId, FileType, Metadata, Mode, OpenOptions,
    UnixMode,
};

use crate::ffi::nt_surface as nt;
use crate::ffi::win32_surface as w;
use crate::sys::handle::OwnedWinHandle;
use crate::sys::{fileio, nt as ntsys};

/// A directory capability backed by a directory HANDLE. All operations are
/// handle-relative (`NtCreateFile` with `RootDirectory`) — the ambient
/// process namespace is consulted only by [`WindowsDir::open_ambient`]
/// (RFC v2 §5.3).
pub struct WindowsDir {
    handle: OwnedWinHandle,
}

impl WindowsDir {
    /// Open an absolute path as the root capability. This is the only
    /// place an absolute path enters the backend; everything after is
    /// relative to a capability.
    pub fn open_ambient(path: &Path) -> Result<Self> {
        let handle = fileio::open_ambient_dir(path.as_os_str())?;
        Ok(Self { handle })
    }
}

/// An open file backed by an [`OwnedWinHandle`]. Public for std interop
/// (RFC v2 §5.1); the [`Dir`] trait still hands out `Box<dyn File>`.
pub struct WindowsFile {
    // `pub(crate)`, not private: `sys::proc`'s `Stdio::File` spawn-time
    // wiring (rustils#51) needs the handle after downcasting through
    // `File::as_any` from a different module in this crate.
    pub(crate) handle: OwnedWinHandle,
}

// std interop (RFC v2 §5.1): delegation to OwnedWinHandle's conversions —
// handle ownership moves, no raw-handle juggling at this layer.

impl AsHandle for WindowsDir {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }
}

impl From<WindowsDir> for OwnedHandle {
    fn from(dir: WindowsDir) -> OwnedHandle {
        OwnedHandle::from(dir.handle)
    }
}

impl AsHandle for WindowsFile {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }
}

impl From<WindowsFile> for std::fs::File {
    fn from(file: WindowsFile) -> std::fs::File {
        std::fs::File::from(OwnedHandle::from(file.handle))
    }
}

impl From<std::fs::File> for WindowsFile {
    fn from(file: std::fs::File) -> Self {
        Self {
            handle: OwnedWinHandle::from(OwnedHandle::from(file)),
        }
    }
}

/// Any readable/writable handle works as a [`WindowsFile`] — pipe ends
/// included (the process backend hands captured-stdio ends out this way).
impl From<OwnedWinHandle> for WindowsFile {
    fn from(handle: OwnedWinHandle) -> Self {
        Self { handle }
    }
}

impl File for WindowsFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        fileio::read(&self.handle, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        fileio::write(&self.handle, buf)
    }

    fn flush(&mut self) -> Result<()> {
        // Synchronous WriteFile has no userspace buffer to flush;
        // durability is the distinct, explicit sync_all below —
        // mirroring the Linux backend.
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        fileio::sync_all(&self.handle)
    }

    fn try_clone(&self) -> Result<Box<dyn File>> {
        Ok(Box::new(WindowsFile {
            handle: crate::sys::handle::duplicate(&self.handle, false)?,
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// (access, disposition) for a file open — the Windows analog of the Linux
/// backend's `open_flags`.
fn open_params(opts: &OpenOptions) -> Result<(u32, u32)> {
    let mut access = w::SYNCHRONIZE;
    if opts.read {
        access |= w::FILE_GENERIC_READ;
    }
    if opts.append {
        // Append-only: grant everything generic write carries except
        // arbitrary-position writes — the OS then atomically appends.
        access |= (w::FILE_GENERIC_WRITE & !w::FILE_WRITE_DATA) | w::FILE_APPEND_DATA;
    } else if opts.write {
        access |= w::FILE_GENERIC_WRITE;
    }
    if !opts.read && !opts.write && !opts.append {
        return Err(PlatformError::new(
            ErrorKind::InvalidInput,
            OsCode::None,
            "open",
        ));
    }
    let disposition = match (opts.create_new, opts.create, opts.truncate) {
        (true, _, _) => nt::FILE_CREATE,
        (false, true, true) => nt::FILE_OVERWRITE_IF,
        (false, true, false) => nt::FILE_OPEN_IF,
        (false, false, true) => nt::FILE_OVERWRITE,
        (false, false, false) => nt::FILE_OPEN,
    };
    Ok((access, disposition))
}

/// No `sync_dir` override here — [`Dir::sync_dir`]'s default no-op
/// `Ok(())` stands, so [`Dir::write_atomic`] on this backend stays D1
/// ("content synchronized"), not D2, pending further research
/// (`docs/behavior/fs.md`). `FlushFileBuffers` (`sys::fileio::sync_all`,
/// `File::sync_all`'s own implementation) accepts a directory handle
/// opened with `FILE_FLAG_BACKUP_SEMANTICS` syntactically, but its
/// actual effect on a directory's own NTFS metadata is not documented by
/// Microsoft and has a known history of surprising, driver-dependent
/// behavior for non-regular-file handles (the "Revised notes on the
/// reliability of FlushFileBuffers" caveats apply to ordinary file
/// handles, not directory ones) — fabricating a D2 claim on an
/// unverified call would be a wrong claim, worse than the honest partial
/// gap this leaves. A future slice with access to a real Windows/NTFS
/// test rig (this workspace's Windows leg runs on `windows-latest` CI
/// but this repository has no committed test proving directory-handle
/// flush semantics one way or the other) can promote this once it has
/// live evidence rather than a syntax check.
impl Dir for WindowsDir {
    fn open(&self, rel: &OsStr, opts: &OpenOptions) -> Result<Box<dyn File>> {
        let (access, disposition) = open_params(opts)?;
        let handle = ntsys::open_relative(
            &self.handle,
            rel,
            access,
            disposition,
            nt::FILE_NON_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )?;
        Ok(Box::new(WindowsFile { handle }))
    }

    /// R2-equivalent containment (`docs/behavior/fs.md`): `OBJ_DONT_REPARSE`
    /// (`sys::nt::open_relative_r2`) rejects a reparse point anywhere in
    /// `rel`'s resolution — intermediate or terminal — where the old
    /// plain `open_relative` call would have followed it transparently.
    /// A real, deliberate behavior change (`docs/divergences.md`), scoped
    /// to `open_dir`/`create_dir` only to match Linux's own R2 scope
    /// decision (`sys::fdio::openat_r2`'s doc comment): `docs/behavior/
    /// fs.md` promises `open`/`access` follow a terminal symlink/reparse
    /// point transparently, and `OBJ_DONT_REPARSE` cannot distinguish
    /// "terminal" from "intermediate" the way `FILE_OPEN_REPARSE_POINT`
    /// does for `metadata`'s lstat-style calls, so applying it there
    /// would break that promise instead of extending it.
    fn open_dir(&self, rel: &OsStr) -> Result<Box<dyn Dir>> {
        let handle = ntsys::open_relative_r2(
            &self.handle,
            rel,
            w::FILE_LIST_DIRECTORY | w::FILE_READ_ATTRIBUTES | w::FILE_TRAVERSE | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )?;
        Ok(Box::new(WindowsDir { handle }))
    }

    /// R2-equivalent containment — see [`WindowsDir::open_dir`]'s own
    /// doc comment; the same scope decision applies here.
    fn create_dir(&self, rel: &OsStr) -> Result<()> {
        // The returned handle is dropped immediately: creation is the
        // operation, the capability is not retained.
        ntsys::open_relative_r2(
            &self.handle,
            rel,
            w::FILE_LIST_DIRECTORY | w::SYNCHRONIZE,
            nt::FILE_CREATE,
            nt::FILE_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )?;
        Ok(())
    }

    fn metadata(&self, rel: &OsStr) -> Result<Metadata> {
        // Attributes-only open; FILE_OPEN_REPARSE_POINT mirrors the Linux
        // backend's AT_SYMLINK_NOFOLLOW (report the link, don't follow it).
        let handle = ntsys::open_relative(
            &self.handle,
            rel,
            w::FILE_READ_ATTRIBUTES | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_SYNCHRONOUS_IO_NONALERT | nt::FILE_OPEN_REPARSE_POINT,
        )?;
        let (file_type, len, nlink, modified) = fileio::metadata_by_handle(&handle, rel)?;
        Ok(Metadata {
            file_type,
            len,
            nlink,
            modified,
        })
    }

    fn access(&self, rel: &OsStr, mode: AccessMode) -> Result<()> {
        // An empty mode is a vacuous yes: no probe at all, matching the
        // Linux backend's short-circuit (there, calling through with
        // bits == 0 would silently become an F_OK existence check
        // instead — the same trap applies here in spirit, so both
        // backends special-case it explicitly rather than relying on
        // "no access bits requested" happening to be harmless).
        if !(mode.read || mode.write || mode.execute) {
            return Ok(());
        }
        // No POSIX execute-bit analog for a regular file: FILE_READ_ATTRIBUTES
        // alone confirms existence (and is what an execute-only probe
        // resolves to), unioned with GENERIC read/write when requested
        // — a trial open, immediately dropped, is the actual operation
        // this probe predicts rather than a separate ACL query that
        // could disagree with it (this trait method's own doc comment,
        // divergence #005). No FILE_OPEN_REPARSE_POINT: like `open`,
        // this follows a terminal symlink.
        let mut access_mask = w::SYNCHRONIZE | w::FILE_READ_ATTRIBUTES;
        if mode.read {
            access_mask |= w::FILE_GENERIC_READ;
        }
        if mode.write {
            access_mask |= w::FILE_GENERIC_WRITE;
        }
        ntsys::open_relative(
            &self.handle,
            rel,
            access_mask,
            nt::FILE_OPEN,
            nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )?;
        Ok(())
    }

    fn unix_mode(&self, _rel: &OsStr) -> Result<Option<UnixMode>> {
        // No POSIX mode bits, setuid/setgid/sticky, or uid/gid ownership
        // concept on Windows at all (NTFS security descriptors are a
        // wholly different model) — `None` is the honest answer, not a
        // zeroed-out fabrication.
        Ok(None)
    }

    fn set_unix_mode(&self, _rel: &OsStr, _mode: Mode) -> Result<()> {
        // Same gap as `unix_mode` (divergence 006), on the write side
        // (divergence 009): no POSIX mode-bit model to set at all. A
        // silent `Ok(())` would misrepresent success — the caller asked
        // to change permissions and nothing changed — so this is a real
        // `Err`, matching the `Signal`/`GroupSpec::JoinGroup` precedent
        // in `process.rs` (divergence 008) rather than `unix_mode`'s own
        // `Ok(None)` shape, which is right only for a *query* with no
        // concept to answer.
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "set_unix_mode",
        ))
    }

    fn file_id(&self, rel: &OsStr) -> Result<FileId> {
        let handle = ntsys::open_relative(
            &self.handle,
            rel,
            w::FILE_READ_ATTRIBUTES | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_SYNCHRONOUS_IO_NONALERT | nt::FILE_OPEN_REPARSE_POINT,
        )?;
        let (serial, index) = fileio::file_id_by_handle(&handle, rel)?;
        Ok(FileId(serial, index))
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        // A fresh handle for enumeration: directory-enumeration state is
        // per-handle, and this capability's own handle must stay pristine
        // for further operations (the Linux backend re-opens "." for the
        // same reason). An empty relative name re-opens this directory.
        let handle = ntsys::open_relative(
            &self.handle,
            OsStr::new(""),
            w::FILE_LIST_DIRECTORY | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_DIRECTORY_FILE | nt::FILE_SYNCHRONOUS_IO_NONALERT,
        )?;
        Ok(fileio::read_dir_entries(&handle)?
            .into_iter()
            .map(|(name, file_type)| DirEntry { name, file_type })
            .collect())
    }

    fn remove_file(&self, rel: &OsStr) -> Result<()> {
        let handle = ntsys::open_relative(
            &self.handle,
            rel,
            w::DELETE | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_NON_DIRECTORY_FILE
                | nt::FILE_SYNCHRONOUS_IO_NONALERT
                | nt::FILE_OPEN_REPARSE_POINT,
        )?;
        fileio::mark_delete(&handle, rel)
        // Deletion completes when `handle` drops (delete-on-close).
    }

    fn remove_dir(&self, rel: &OsStr) -> Result<()> {
        let handle = ntsys::open_relative(
            &self.handle,
            rel,
            w::DELETE | w::SYNCHRONIZE,
            nt::FILE_OPEN,
            nt::FILE_DIRECTORY_FILE
                | nt::FILE_SYNCHRONOUS_IO_NONALERT
                | nt::FILE_OPEN_REPARSE_POINT,
        )?;
        // A non-empty directory is refused here by the OS itself
        // (ERROR_DIR_NOT_EMPTY → DirectoryNotEmpty).
        fileio::mark_delete(&handle, rel)
    }

    fn symlink(&self, target: &OsStr, link_name: &OsStr) -> Result<()> {
        // No POSIX analog: the reparse point must declare file-vs-
        // directory at creation. Best-effort decide by looking up
        // `target` relative to this same capability — an existing
        // directory there makes a directory-type link; anything else
        // (a file, a dangling target, an absolute target, or one
        // elsewhere entirely) falls back to file-type, matching the
        // heuristic other cross-platform symlink recipes use (see
        // `Dir::symlink`'s doc comment for the full divergence note).
        let is_dir = matches!(
            self.metadata(target),
            Ok(Metadata {
                file_type: FileType::Dir,
                ..
            })
        );
        fileio::symlink(&self.handle, link_name, target, is_dir)
    }

    fn read_link(&self, rel: &OsStr) -> Result<OsString> {
        fileio::read_link(&self.handle, rel)
    }

    fn rename(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        fileio::rename(&self.handle, from, to, true)
    }

    fn rename_no_replace(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        fileio::rename(&self.handle, from, to, false)
    }
}

/// [`AnonymousFile`] — `Unsupported` unconditionally. Windows's nearest
/// analog, an anonymous `CreateFileMappingW(INVALID_HANDLE_VALUE, ...)`
/// mapping, is a fixed-size shared-memory *mapping*
/// (`docs/design-discussion-msys-pgid-table.md` uses exactly this
/// primitive, for an unrelated purpose), not a byte-stream `File` that
/// can grow, `read`/`write` sequentially, and be handed to
/// `Dir::write_atomic`'s temp-file-shaped consumers the way a real
/// `memfd_create` fd can — no donor evidence for a shape that actually
/// fits this trait, matching [`crate::WindowsTun`]'s own posture for a
/// missing donor rather than forcing a mismatched primitive to answer.
#[derive(Debug, Default)]
pub struct WindowsAnonymousFile;

impl AnonymousFile for WindowsAnonymousFile {
    fn create_memfd(&self, _name: &str) -> Result<Box<dyn File>> {
        Err(PlatformError::new(
            ErrorKind::Unsupported,
            OsCode::None,
            "create_memfd",
        ))
    }
}
