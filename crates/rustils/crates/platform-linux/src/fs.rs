//! `Dir`/`File` trait impls over the sys layer. No `unsafe` here.
//!
//! **Resolution safety (Rusty-Mill R-scale):** `open`/`metadata`/
//! `access`/`unix_mode`/`file_id`/`set_unix_mode`/`read_link`/`rename`/
//! `rename_no_replace`/`remove_file`/`remove_dir`/`read_dir` resolve via
//! the plain `openat`/`*at` family (`sys::fdio`) — R1 ("link-confined,
//! not atomic"): every component is resolved by the kernel's ordinary
//! path walk, which follows an intermediate symlink transparently, and
//! `open`'s terminal component too (`docs/behavior/fs.md`'s documented
//! "open follows symlinks transparently" promise, deliberately left
//! unchanged here). `open_dir`/`create_dir` are **R2** on a 5.6+ kernel
//! (`sys::fdio::openat_r2`: raw `openat2` with `RESOLVE_NO_SYMLINKS |
//! RESOLVE_NO_XDEV` — every component, intermediate *and* terminal,
//! rejected if it's a symlink or would cross a filesystem/mount
//! boundary), falling back to the unchanged R1 `openat`/`mkdirat` path
//! on an older kernel (`ErrorKind::Unsupported` from `openat2`'s own
//! `ENOSYS`, never a hard failure). Not R3: no `RESOLVE_IN_ROOT`/root-
//! constraint semantics are requested, and this `Dir` contract does not
//! promise beneath-confinement today, so claiming R3 here would overstate
//! what's actually enforced.
//!
//! **Durability (Rusty-Mill D-scale):** `File::sync_all` (`fsync`) is D1
//! ("content synchronized"). `Dir::write_atomic`'s inherited default
//! (`crates/platform/src/fs.rs`) is **D2** on this backend:
//! `LinuxDir::sync_dir` (`fsync` on the capability's own `O_DIRECTORY`
//! fd, valid per `fsync(2)`'s own text) runs after the publishing
//! rename, so the directory-entry mutation itself — not just the
//! renamed file's content — is durable before `write_atomic` returns.

use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::fs::{
    AccessMode, AnonymousFile, Dir, DirEntry, File, FileId, Metadata, Mode, OpenOptions, UnixMode,
};

use crate::ffi::libc_surface as c;
use crate::sys::fdio;

/// A directory capability backed by an `O_DIRECTORY` file descriptor.
/// All operations are dirfd-relative (`openat` family) — the ambient cwd
/// is never consulted (RFC v2 §5.3).
pub struct LinuxDir {
    fd: OwnedFd,
}

impl LinuxDir {
    /// Open an absolute path as the root capability. This is the only
    /// place an absolute path enters the backend; everything after is
    /// relative to a capability.
    pub fn open_ambient(path: &Path) -> Result<Self> {
        let fd = fdio::openat(
            c::AT_FDCWD,
            path.as_os_str(),
            c::O_RDONLY | c::O_DIRECTORY,
            0,
        )?;
        Ok(Self { fd })
    }
}

/// An open file backed by an `OwnedFd`. Public for std interop (RFC v2
/// §5.1); the [`Dir`] trait still hands out `Box<dyn File>`.
pub struct LinuxFile {
    fd: OwnedFd,
}

// std interop (RFC v2 §5.1): handle types are adoptable incrementally,
// not a total buy-in island. All conversions are safe fd-ownership moves.

impl AsFd for LinuxDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl From<LinuxDir> for OwnedFd {
    fn from(dir: LinuxDir) -> OwnedFd {
        dir.fd
    }
}

/// The fd must reference a directory; operations on a capability built
/// from a non-directory fd fail with `NotADirectory` at call time.
impl From<OwnedFd> for LinuxDir {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

impl AsFd for LinuxFile {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl From<LinuxFile> for std::fs::File {
    fn from(file: LinuxFile) -> std::fs::File {
        std::fs::File::from(file.fd)
    }
}

impl From<std::fs::File> for LinuxFile {
    fn from(file: std::fs::File) -> Self {
        Self {
            fd: OwnedFd::from(file),
        }
    }
}

/// Any readable/writable fd works as a [`LinuxFile`] — pipe ends included
/// (the process backend hands captured-stdio ends out this way).
impl From<OwnedFd> for LinuxFile {
    fn from(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

impl File for LinuxFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        fdio::read(&self.fd, buf)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        fdio::write(&self.fd, buf)
    }

    fn flush(&mut self) -> Result<()> {
        // write(2) has no userspace buffer to flush; durability (fsync)
        // is the distinct, explicit sync_all below.
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        fdio::fsync(&self.fd)
    }

    fn try_clone(&self) -> Result<Box<dyn File>> {
        Ok(Box::new(LinuxFile {
            fd: fdio::dup_cloexec(&self.fd)?,
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Split `rel` into (parent, leaf) at its last `/`, ignoring any run of
/// trailing slashes first (`"a/b/"` splits the same as `"a/b"` — mkdir's
/// own tolerance for a trailing slash on the target, which the old
/// single-`mkdirat`-call implementation got for free from the kernel's
/// own path parser and this split must not regress). `None` for a
/// single-component (or empty, or all-slashes) `rel` — nothing to
/// resolve as a separate parent step.
fn split_last_component(rel: &OsStr) -> Option<(OsString, OsString)> {
    let bytes = rel.as_bytes();
    let end = bytes.iter().rposition(|&b| b != b'/').map(|i| i + 1)?;
    let trimmed = &bytes[..end];
    let pos = trimmed.iter().rposition(|&b| b == b'/')?;
    let parent = OsStr::from_bytes(&trimmed[..pos]).to_os_string();
    let leaf = OsStr::from_bytes(&trimmed[pos + 1..]).to_os_string();
    Some((parent, leaf))
}

fn open_flags(opts: &OpenOptions) -> Result<i32> {
    let mut flags = match (opts.read, opts.write || opts.append) {
        (true, true) => c::O_RDWR,
        (true, false) => c::O_RDONLY,
        (false, true) => c::O_WRONLY,
        (false, false) => {
            return Err(PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "open",
            ))
        }
    };
    if opts.append {
        flags |= c::O_APPEND;
    }
    if opts.create_new {
        flags |= c::O_CREAT | c::O_EXCL;
    } else if opts.create {
        flags |= c::O_CREAT;
    }
    if opts.truncate {
        flags |= c::O_TRUNC;
    }
    Ok(flags)
}

impl Dir for LinuxDir {
    fn open(&self, rel: &OsStr, opts: &OpenOptions) -> Result<Box<dyn File>> {
        let flags = open_flags(opts)?;
        let fd = fdio::openat(self.fd.as_raw_fd(), rel, flags, 0o666)?;
        Ok(Box::new(LinuxFile { fd }))
    }

    /// R2 on kernels with `openat2` (5.6+): every component of `rel` —
    /// intermediate and terminal alike — is rejected if it's a symlink,
    /// and resolution is refused if it would cross a filesystem/mount
    /// boundary (`docs/behavior/fs.md`). R1 (unchanged: symlinks
    /// followed, mounts crossed) on an older kernel — `sys::fdio::
    /// openat_r2` falls back transparently, never a hard failure.
    ///
    /// This *does* newly reject a terminal symlink where the old plain-
    /// `openat` call would have followed it — a real, deliberate
    /// behavior change, unlike `Dir::open` (left alone; see its own
    /// `open_flags`/call site — no such promise exists for `open_dir`
    /// in `docs/behavior/fs.md` today, only for `open`/`access`, so
    /// there is nothing documented to break).
    fn open_dir(&self, rel: &OsStr) -> Result<Box<dyn Dir>> {
        let fd = fdio::openat_r2(self.fd.as_raw_fd(), rel, c::O_RDONLY | c::O_DIRECTORY, 0)?;
        Ok(Box::new(LinuxDir { fd }))
    }

    /// R2 for the *parent* portion of a multi-component `rel` (e.g.
    /// `"a/b/newdir"`): the parent path is resolved once via
    /// `sys::fdio::openat_r2` (R2 on a 5.6+ kernel, R1 fallback
    /// otherwise) into an fd, and `mkdirat` then creates the leaf
    /// component relative to that already-resolved fd — no second path
    /// walk, so no reintroduced race. A single-component `rel` (the
    /// common case — no `/` at all) skips this entirely and calls
    /// `mkdirat` directly on `self`'s own fd: with zero intermediate
    /// components to traverse, and `mkdirat` refusing outright
    /// (`AlreadyExists`) if a symlink already occupies the leaf name,
    /// that case is inherently R2 already — nothing for `openat2` to
    /// add.
    fn create_dir(&self, rel: &OsStr) -> Result<()> {
        match split_last_component(rel) {
            None => fdio::mkdirat(self.fd.as_raw_fd(), rel),
            Some((parent, leaf)) => {
                let parent_fd = fdio::openat_r2(
                    self.fd.as_raw_fd(),
                    &parent,
                    c::O_RDONLY | c::O_DIRECTORY,
                    0,
                )?;
                fdio::mkdirat(parent_fd.as_raw_fd(), &leaf)
            }
        }
    }

    fn metadata(&self, rel: &OsStr) -> Result<Metadata> {
        let (file_type, len, nlink, modified) = fdio::statat(self.fd.as_raw_fd(), rel)?;
        Ok(Metadata {
            file_type,
            len,
            nlink,
            modified,
        })
    }

    fn access(&self, rel: &OsStr, mode: AccessMode) -> Result<()> {
        // An empty mode is a vacuous yes, not F_OK: bits == 0 is what
        // faccessat's mode parameter uses to mean "check existence
        // only" (F_OK's own value), so this can't fall through to the
        // syscall with bits left at 0 without silently becoming a
        // different check than the one documented.
        if !(mode.read || mode.write || mode.execute) {
            return Ok(());
        }
        let mut bits = 0;
        if mode.read {
            bits |= c::R_OK;
        }
        if mode.write {
            bits |= c::W_OK;
        }
        if mode.execute {
            bits |= c::X_OK;
        }
        fdio::access(self.fd.as_raw_fd(), rel, bits)
    }

    fn unix_mode(&self, rel: &OsStr) -> Result<Option<UnixMode>> {
        fdio::unix_mode(self.fd.as_raw_fd(), rel).map(Some)
    }

    fn set_unix_mode(&self, rel: &OsStr, mode: Mode) -> Result<()> {
        fdio::set_unix_mode(self.fd.as_raw_fd(), rel, mode)
    }

    fn file_id(&self, rel: &OsStr) -> Result<FileId> {
        let (dev, ino) = fdio::file_id(self.fd.as_raw_fd(), rel)?;
        Ok(FileId(dev, ino))
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        // A fresh fd for enumeration: fdopendir consumes its fd, and this
        // capability's own fd must stay valid for further operations.
        let fd = fdio::openat(
            self.fd.as_raw_fd(),
            OsStr::new("."),
            c::O_RDONLY | c::O_DIRECTORY,
            0,
        )?;
        Ok(fdio::read_dir(fd)?
            .into_iter()
            .map(|(name, file_type)| DirEntry { name, file_type })
            .collect())
    }

    fn remove_file(&self, rel: &OsStr) -> Result<()> {
        fdio::unlinkat(self.fd.as_raw_fd(), rel, false)
    }

    fn remove_dir(&self, rel: &OsStr) -> Result<()> {
        fdio::unlinkat(self.fd.as_raw_fd(), rel, true)
    }

    fn symlink(&self, target: &OsStr, link_name: &OsStr) -> Result<()> {
        fdio::symlink(self.fd.as_raw_fd(), target, link_name)
    }

    fn read_link(&self, rel: &OsStr) -> Result<OsString> {
        fdio::read_link(self.fd.as_raw_fd(), rel)
    }

    fn rename(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        fdio::rename(self.fd.as_raw_fd(), from, to)
    }

    fn rename_no_replace(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        fdio::rename_no_replace(self.fd.as_raw_fd(), from, to)
    }

    /// D2: `fsync` on this capability's own `O_DIRECTORY` fd — valid and
    /// meaningful per `fsync(2)`'s own text ("calling fsync() does not
    /// necessarily ensure that the entry in the directory containing the
    /// file has also reached disk. For that an explicit fsync() on a
    /// file descriptor for the directory is also needed"), which is
    /// exactly the gap [`Dir::write_atomic`]'s publishing `rename` left
    /// open before this landed. No fresh fd needed — unlike `read_dir`,
    /// which opens a throwaway fd because `fdopendir` consumes it, this
    /// takes no ownership and leaves `self.fd` exactly as usable
    /// afterward. Live-verified by strace, not just "the syscall
    /// returned 0" — see
    /// `write_atomic_fsyncs_the_directory_after_the_publishing_rename`
    /// in `tests/parity.rs`.
    fn sync_dir(&self) -> Result<()> {
        fdio::fsync(&self.fd)
    }
}

/// [`AnonymousFile`] over `memfd_create` — a unit struct, like
/// [`crate::LinuxCsprng`]: no capability, no state, nothing a `Dir`
/// would give it that it needs.
#[derive(Debug, Default)]
pub struct LinuxAnonymousFile;

impl AnonymousFile for LinuxAnonymousFile {
    fn create_memfd(&self, name: &str) -> Result<Box<dyn File>> {
        Ok(Box::new(LinuxFile::from(fdio::memfd_create(name)?)))
    }
}

#[cfg(test)]
mod anonymous_file_tests {
    use super::*;

    #[test]
    fn create_memfd_is_usable_through_the_public_file_trait() {
        // Content round-tripping is already verified at the raw-fd
        // level by `sys::fdio::memfd_tests` (seek-and-read-back, plus
        // the /proc "(deleted)" liveness check). This test's own job is
        // narrower: prove the public wiring — `LinuxAnonymousFile` →
        // `fdio::memfd_create` → `LinuxFile::from(OwnedFd)` → the
        // type-erased `Box<dyn File>` this trait hands back — actually
        // works end to end, not just each piece in isolation.
        let anon = LinuxAnonymousFile;
        let mut f = anon
            .create_memfd("rustils-fs-level-test")
            .expect("create_memfd should succeed");
        let n = f.write(b"payload").expect("write");
        assert_eq!(n, 7);
        f.sync_all().expect("sync_all should be a no-op success");
        let cloned = f.try_clone().expect("try_clone");
        drop(f);
        drop(cloned);
    }
}
