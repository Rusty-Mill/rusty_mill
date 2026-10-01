//! Create directories and open files beneath a root directory without
//! following a symlink anywhere below it.
//!
//! For code that writes paths chosen by someone else: a received file
//! name, an archive entry, a config path from a request. Every component of
//! `rel` must be a real directory or (last) a regular file; a symlink
//! anywhere below `root` is refused rather than followed. `root` itself is
//! trusted and may be a symlink.
//!
//! On Linux each component is opened relative to its parent's descriptor
//! with `O_NOFOLLOW` (`openat`), so nothing can be swapped in between a
//! check and the open. On other platforms each component is checked with
//! `symlink_metadata` before use. There a local process racing to replace
//! a directory between that check and the open is not stopped, but a
//! symlink already in place (for example one planted by an earlier archive
//! entry) is.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path};

/// A file opened by [`open_for_write`].
#[derive(Debug)]
pub struct Opened {
    /// Open for writing, positioned at the start, never truncated.
    pub file: File,
    /// Whether this call created the file.
    pub created: bool,
}

/// Creates `rel` and any missing parents beneath `root`.
///
/// # Errors
/// `InvalidInput` if `rel` is absolute or contains `..`. An error if any
/// existing component is a symlink or not a directory, or from the
/// underlying I/O.
pub fn create_dir_all(root: &Path, rel: &Path) -> io::Result<()> {
    BACKEND.create_dir_all(root, rel)
}

/// Opens `rel` beneath `root` for writing, creating it if it does not
/// exist. Parent directories must already exist ([`create_dir_all`]). An
/// existing file is not truncated; the caller sizes it.
///
/// # Errors
/// `InvalidInput` if `rel` is empty, absolute or contains `..`. An error
/// if any component is a symlink, a parent is not a directory, or an
/// existing `rel` is not a regular file, or from the underlying I/O.
pub fn open_for_write(root: &Path, rel: &Path) -> io::Result<Opened> {
    BACKEND.open_for_write(root, rel)
}

/// Opens the regular file `rel` beneath `root` for reading.
///
/// # Errors
/// `InvalidInput` if `rel` is empty, absolute or contains `..`. An error
/// if any component is a symlink, a parent is not a directory, or `rel`
/// is not a regular file, or from the underlying I/O.
pub fn open_for_read(root: &Path, rel: &Path) -> io::Result<File> {
    BACKEND.open_for_read(root, rel)
}

/// One implementation of the operations, over validated names.
struct Backend {
    create_dir_all: fn(&Path, &[&OsStr]) -> io::Result<()>,
    open_for_write: fn(&Path, &[&OsStr], &OsStr) -> io::Result<Opened>,
    open_for_read: fn(&Path, &[&OsStr], &OsStr) -> io::Result<File>,
}

#[cfg(target_os = "linux")]
const BACKEND: Backend = fd_walk::BACKEND;
#[cfg(not(target_os = "linux"))]
const BACKEND: Backend = checked::BACKEND;

impl Backend {
    fn create_dir_all(&self, root: &Path, rel: &Path) -> io::Result<()> {
        (self.create_dir_all)(root, &normal_components(rel)?)
    }

    fn open_for_write(&self, root: &Path, rel: &Path) -> io::Result<Opened> {
        let names = normal_components(rel)?;
        let Some((last, parents)) = names.split_last() else {
            return Err(invalid("an empty path names no file"));
        };
        (self.open_for_write)(root, parents, last)
    }

    fn open_for_read(&self, root: &Path, rel: &Path) -> io::Result<File> {
        let names = normal_components(rel)?;
        let Some((last, parents)) = names.split_last() else {
            return Err(invalid("an empty path names no file"));
        };
        (self.open_for_read)(root, parents, last)
    }
}

/// `rel`'s names, refusing anything that could leave `root`.
fn normal_components(rel: &Path) -> io::Result<Vec<&OsStr>> {
    let mut names = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(name) => names.push(name),
            Component::CurDir => {}
            Component::ParentDir => return Err(invalid("`..` is not allowed")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid("the path must be relative to the root"));
            }
        }
    }
    Ok(names)
}

fn invalid(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why)
}

fn not_a_regular_file() -> io::Error {
    io::Error::other("refusing: the path is not a regular file")
}

#[cfg(target_os = "linux")]
mod fd_walk {
    //! Descriptor walk: every component is opened relative to its parent's
    //! fd with `O_NOFOLLOW`, so there is no window between check and use.

    use super::{Backend, Opened, not_a_regular_file};
    use rusty_libc::Errno;
    use rusty_libc::fd::{
        AT_FDCWD, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_NONBLOCK, O_RDONLY,
        O_WRONLY, openat,
    };
    use rusty_libc::fs::mkdirat;
    use std::ffi::{CString, OsStr};
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    const DIR: i32 = O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC;

    pub(super) const BACKEND: Backend = Backend {
        create_dir_all,
        open_for_write,
        open_for_read,
    };

    fn create_dir_all(root: &Path, names: &[&OsStr]) -> io::Result<()> {
        let mut dir = open_root(root)?;
        for name in names {
            dir = descend(&dir, &c_name(name)?, true)?;
        }
        Ok(())
    }

    fn open_for_write(root: &Path, parents: &[&OsStr], last: &OsStr) -> io::Result<Opened> {
        let dir = open_parent(root, parents)?;
        let name = c_name(last)?;
        let create = O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC;
        match openat(dir.as_raw_fd(), &name, create, 0o666) {
            Ok(fd) => {
                return Ok(Opened {
                    file: File::from(owned(fd)),
                    created: true,
                });
            }
            Err(e) if e == Errno::EEXIST => {}
            Err(e) => return Err(e.into()),
        }
        // `O_NONBLOCK` so a FIFO planted at `last` fails or is refused below
        // instead of blocking the open; it has no effect on a regular file.
        let existing = O_WRONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC;
        let file = File::from(owned(openat(dir.as_raw_fd(), &name, existing, 0)?));
        if !file.metadata()?.is_file() {
            return Err(not_a_regular_file());
        }
        Ok(Opened {
            file,
            created: false,
        })
    }

    fn open_for_read(root: &Path, parents: &[&OsStr], last: &OsStr) -> io::Result<File> {
        let dir = open_parent(root, parents)?;
        // `O_NONBLOCK` as in `open_for_write`: a FIFO is refused, not waited on.
        let flags = O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC;
        let file = File::from(owned(openat(dir.as_raw_fd(), &c_name(last)?, flags, 0)?));
        if !file.metadata()?.is_file() {
            return Err(not_a_regular_file());
        }
        Ok(file)
    }

    /// The directory holding the last component, reached without following
    /// a symlink.
    fn open_parent(root: &Path, parents: &[&OsStr]) -> io::Result<OwnedFd> {
        let mut dir = open_root(root)?;
        for name in parents {
            dir = descend(&dir, &c_name(name)?, false)?;
        }
        Ok(dir)
    }

    fn open_root(root: &Path) -> io::Result<OwnedFd> {
        let path = CString::new(root.as_os_str().as_bytes())
            .map_err(|_| super::invalid("the root contains a NUL byte"))?;
        let flags = O_RDONLY | O_DIRECTORY | O_CLOEXEC;
        Ok(owned(openat(AT_FDCWD, &path, flags, 0)?))
    }

    /// Opens directory `name` in `dir`, refusing a symlink (`ELOOP`) or a
    /// non-directory (`ENOTDIR`); with `create`, makes it first if absent.
    fn descend(dir: &OwnedFd, name: &CString, create: bool) -> io::Result<OwnedFd> {
        match openat(dir.as_raw_fd(), name, DIR, 0) {
            Ok(fd) => return Ok(owned(fd)),
            Err(e) if create && e == Errno::ENOENT => {}
            Err(e) => return Err(e.into()),
        }
        match mkdirat(dir.as_raw_fd(), name, 0o777) {
            Ok(()) => {}
            Err(e) if e == Errno::EEXIST => {} // a concurrent create; the open re-checks it
            Err(e) => return Err(e.into()),
        }
        Ok(owned(openat(dir.as_raw_fd(), name, DIR, 0)?))
    }

    fn c_name(name: &OsStr) -> io::Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| super::invalid("a name contains a NUL byte"))
    }

    #[allow(unsafe_code)] // the crate's one `unsafe`: adopting a fresh fd
    fn owned(fd: i32) -> OwnedFd {
        // SAFETY: `fd` was just returned by a successful `openat` and no
        // other owner holds it; the `OwnedFd` closes it exactly once.
        unsafe { OwnedFd::from_raw_fd(fd) }
    }
}

#[cfg(any(not(target_os = "linux"), test))]
mod checked {
    //! Checked walk: each component is `lstat`ed before use. A symlink
    //! already in place is refused; one swapped in between the check and
    //! the open by a local process is not. Also built for Linux tests, so
    //! this fallback's behavior is checked there.

    use super::{Backend, Opened, not_a_regular_file};
    use std::ffi::OsStr;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::path::{Path, PathBuf};

    pub(super) const BACKEND: Backend = Backend {
        create_dir_all,
        open_for_write,
        open_for_read,
    };

    fn create_dir_all(root: &Path, names: &[&OsStr]) -> io::Result<()> {
        walk(root, names, true).map(drop)
    }

    fn open_for_read(root: &Path, parents: &[&OsStr], last: &OsStr) -> io::Result<File> {
        let path = walk(root, parents, false)?.join(last);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(symlink_refused(&path));
        }
        let file = File::open(&path)?;
        verify_opened(&file, &path)?;
        Ok(file)
    }

    /// `file` must be the regular file `lstat` finds at `path`, not a
    /// symlink target swapped in before the open.
    fn verify_opened(file: &File, path: &Path) -> io::Result<()> {
        let opened = file.metadata()?;
        let at_path = fs::symlink_metadata(path)?;
        if !opened.is_file() || at_path.file_type().is_symlink() || !same_file(&opened, &at_path) {
            return Err(not_a_regular_file());
        }
        Ok(())
    }

    fn open_for_write(root: &Path, parents: &[&OsStr], last: &OsStr) -> io::Result<Opened> {
        let path = walk(root, parents, false)?.join(last);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(symlink_refused(&path));
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return Ok(Opened {
                    file,
                    created: true,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let file = OpenOptions::new().write(true).open(&path)?;
        verify_opened(&file, &path)?;
        Ok(Opened {
            file,
            created: false,
        })
    }

    /// `root` joined with `names`, each checked to be a real directory.
    fn walk(root: &Path, names: &[&OsStr], create: bool) -> io::Result<PathBuf> {
        let mut path = root.to_path_buf();
        for name in names {
            path.push(name);
            match fs::symlink_metadata(&path) {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(&path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(e),
                    }
                    check_dir(&path, &fs::symlink_metadata(&path)?)?;
                }
                Err(e) => return Err(e),
                Ok(meta) => check_dir(&path, &meta)?,
            }
        }
        Ok(path)
    }

    fn check_dir(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
        if meta.file_type().is_symlink() {
            return Err(symlink_refused(path));
        }
        if !meta.is_dir() {
            return Err(io::Error::other(format!(
                "{} is not a directory",
                path.display()
            )));
        }
        Ok(())
    }

    fn symlink_refused(path: &Path) -> io::Error {
        io::Error::other(format!(
            "refusing to follow the symlink at {}",
            path.display()
        ))
    }

    #[cfg(unix)]
    fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }

    /// Windows: std has no stable file identity; the symlink checks stand
    /// (creating a symlink there needs a privilege a remote sender lacks).
    #[cfg(not(unix))]
    fn same_file(_a: &fs::Metadata, _b: &fs::Metadata) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Every implementation built for this target: on Linux both the
    /// descriptor walk and the checked fallback other platforms use.
    fn backends() -> Vec<(&'static str, Backend)> {
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut all = vec![("checked", checked::BACKEND)];
        #[cfg(target_os = "linux")]
        all.push(("fd_walk", fd_walk::BACKEND));
        all
    }

    /// A fresh root (and a sibling "outside" directory) per test.
    struct Scratch {
        base: PathBuf,
        root: PathBuf,
        #[cfg_attr(not(unix), allow(dead_code))] // only the symlink tests use it
        outside: PathBuf,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let base = std::env::temp_dir().join(format!(
                "rusty_confined_fs-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let root = base.join("root");
            let outside = base.join("outside");
            std::fs::create_dir_all(&root).expect("root");
            std::fs::create_dir_all(&outside).expect("outside");
            Self {
                base,
                root,
                outside,
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn creates_directories_and_a_file_then_reopens_it_without_truncating() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            b.create_dir_all(&s.root, Path::new("a/b")).expect(name);
            let mut first = b
                .open_for_write(&s.root, Path::new("a/b/f.txt"))
                .expect(name);
            assert!(first.created, "{name}");
            first.file.write_all(b"hello").expect(name);
            drop(first);

            let again = b
                .open_for_write(&s.root, Path::new("./a/b/f.txt"))
                .expect(name);
            assert!(!again.created, "{name}");
            drop(again);
            assert_eq!(
                std::fs::read(s.root.join("a/b/f.txt")).expect(name),
                b"hello"
            );
        }
    }

    #[test]
    fn create_dir_all_is_idempotent_and_accepts_an_empty_path() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            b.create_dir_all(&s.root, Path::new("x/y")).expect(name);
            b.create_dir_all(&s.root, Path::new("x/y")).expect(name);
            b.create_dir_all(&s.root, Path::new("")).expect(name);
            assert!(s.root.join("x/y").is_dir(), "{name}");
        }
    }

    #[test]
    fn paths_that_could_leave_the_root_are_invalid_input() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            for rel in ["../escape", "a/../../escape", "/etc/passwd", ""] {
                let err = b.open_for_write(&s.root, Path::new(rel)).expect_err(rel);
                assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name} {rel}");
            }
            let err = b
                .create_dir_all(&s.root, Path::new("../escape"))
                .expect_err(name);
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name}");
        }
    }

    #[test]
    fn a_missing_parent_is_not_created_by_open_for_write() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            let err = b
                .open_for_write(&s.root, Path::new("absent/f"))
                .expect_err(name);
            assert_eq!(err.kind(), io::ErrorKind::NotFound, "{name}");
            assert!(!s.root.join("absent").exists(), "{name}");
        }
    }

    #[test]
    fn a_directory_at_the_destination_is_refused() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            std::fs::create_dir(s.root.join("d")).expect(name);
            assert!(b.open_for_write(&s.root, Path::new("d")).is_err(), "{name}");
        }
    }

    #[test]
    fn a_file_where_a_directory_is_needed_is_refused() {
        for (name, b) in backends() {
            let s = Scratch::new(name);
            std::fs::write(s.root.join("f"), b"x").expect(name);
            assert!(
                b.create_dir_all(&s.root, Path::new("f/sub")).is_err(),
                "{name}"
            );
            assert!(
                b.open_for_write(&s.root, Path::new("f/sub")).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn open_for_read_reads_a_regular_file_and_refuses_anything_else() {
        use std::io::Read;
        for (name, b) in backends() {
            let s = Scratch::new(name);
            std::fs::create_dir(s.root.join("d")).expect(name);
            std::fs::write(s.root.join("d/f"), b"data").expect(name);
            let mut out = String::new();
            b.open_for_read(&s.root, Path::new("d/f"))
                .expect(name)
                .read_to_string(&mut out)
                .expect(name);
            assert_eq!(out, "data", "{name}");

            let missing = b
                .open_for_read(&s.root, Path::new("d/absent"))
                .expect_err(name);
            assert_eq!(missing.kind(), io::ErrorKind::NotFound, "{name}");
            assert!(b.open_for_read(&s.root, Path::new("d")).is_err(), "{name}");
            assert!(
                b.open_for_read(&s.root, Path::new("../x")).is_err(),
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    mod symlinks {
        use super::*;
        use std::os::unix::fs::symlink;

        #[test]
        fn a_symlinked_destination_is_refused_and_its_target_untouched() {
            for (name, b) in backends() {
                let s = Scratch::new(name);
                let target = s.outside.join("victim");
                std::fs::write(&target, b"untouched").expect(name);
                symlink(&target, s.root.join("f")).expect(name);

                assert!(b.open_for_write(&s.root, Path::new("f")).is_err(), "{name}");
                assert_eq!(std::fs::read(&target).expect(name), b"untouched", "{name}");
            }
        }

        #[test]
        fn a_dangling_symlinked_destination_is_not_created_through() {
            for (name, b) in backends() {
                let s = Scratch::new(name);
                let target = s.outside.join("would-be-created");
                symlink(&target, s.root.join("f")).expect(name);

                assert!(b.open_for_write(&s.root, Path::new("f")).is_err(), "{name}");
                assert!(!target.exists(), "{name}");
            }
        }

        #[test]
        fn a_symlinked_intermediate_directory_is_refused() {
            for (name, b) in backends() {
                let s = Scratch::new(name);
                symlink(&s.outside, s.root.join("sub")).expect(name);

                assert!(
                    b.open_for_write(&s.root, Path::new("sub/f")).is_err(),
                    "{name}"
                );
                assert!(
                    b.create_dir_all(&s.root, Path::new("sub/new")).is_err(),
                    "{name}"
                );
                assert_eq!(
                    std::fs::read_dir(&s.outside).expect(name).count(),
                    0,
                    "{name}"
                );
            }
        }

        #[test]
        fn open_for_read_refuses_a_symlink_at_any_component() {
            for (name, b) in backends() {
                let s = Scratch::new(name);
                std::fs::write(s.outside.join("secret"), b"no").expect(name);
                symlink(s.outside.join("secret"), s.root.join("f")).expect(name);
                symlink(&s.outside, s.root.join("sub")).expect(name);

                assert!(b.open_for_read(&s.root, Path::new("f")).is_err(), "{name}");
                assert!(
                    b.open_for_read(&s.root, Path::new("sub/secret")).is_err(),
                    "{name}"
                );
            }
        }

        #[test]
        fn a_symlinked_root_is_trusted() {
            for (name, b) in backends() {
                let s = Scratch::new(name);
                let alias = s.base.join("alias");
                symlink(&s.root, &alias).expect(name);
                let opened = b.open_for_write(&alias, Path::new("f")).expect(name);
                assert!(opened.created, "{name}");
                assert!(s.root.join("f").is_file(), "{name}");
            }
        }

        /// The public functions use the descriptor walk on Linux, where the
        /// kernel itself refuses: `ENOTDIR` for a symlinked directory
        /// (`O_DIRECTORY | O_NOFOLLOW`), `ELOOP` for a symlinked file.
        #[cfg(target_os = "linux")]
        #[test]
        fn on_linux_the_kernel_refuses_each_symlink() {
            let s = Scratch::new("kernel");
            symlink(&s.outside, s.root.join("sub")).expect("symlink");
            symlink(s.outside.join("x"), s.root.join("f")).expect("symlink");
            let dir = open_for_write(&s.root, Path::new("sub/f")).expect_err("dir");
            assert_eq!(dir.raw_os_error(), Some(20), "ENOTDIR");
            let file = open_for_write(&s.root, Path::new("f")).expect_err("file");
            assert_eq!(file.raw_os_error(), Some(40), "ELOOP");
            let read = open_for_read(&s.root, Path::new("f")).expect_err("read");
            assert_eq!(read.raw_os_error(), Some(40), "ELOOP");
        }
    }
}
