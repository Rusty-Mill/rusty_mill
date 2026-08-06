//! `Dir`/`File` trait impls over the sys layer. No `unsafe` here.

use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
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

    fn open_dir(&self, rel: &OsStr) -> Result<Box<dyn Dir>> {
        let fd = fdio::openat(self.fd.as_raw_fd(), rel, c::O_RDONLY | c::O_DIRECTORY, 0)?;
        Ok(Box::new(LinuxDir { fd }))
    }

    fn create_dir(&self, rel: &OsStr) -> Result<()> {
        fdio::mkdirat(self.fd.as_raw_fd(), rel)
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
