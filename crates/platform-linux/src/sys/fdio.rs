//! Owned-fd primitives: openat-family operations, read/write, directory
//! enumeration. The building blocks the `Dir`/`File` impls compose.

#![allow(unsafe_code)] // the one place in this crate it is permitted

#[cfg(not(feature = "track-p"))]
use std::ffi::CStr;
use std::ffi::{CString, OsStr, OsString};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::time::{Duration, SystemTime};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::fs::{FileType, UnixMode};

use crate::ffi::libc_surface as c;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Track P error path: a raw syscall's failure comes back as the `Errno`
/// value in the return register — the thread-local `errno` that `os_err`
/// reads is never touched. The code must flow from the returned value.
#[cfg(feature = "track-p")]
fn trackp_err(op: &'static str, e: rusty_libc::Errno) -> PlatformError {
    PlatformError::new(kind_of(e.0), OsCode::Errno(e.0), op)
}

fn kind_of(errno: i32) -> ErrorKind {
    match errno {
        libc::ENOENT => ErrorKind::NotFound,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::EEXIST => ErrorKind::AlreadyExists,
        libc::ENOTDIR => ErrorKind::NotADirectory,
        libc::EISDIR => ErrorKind::IsADirectory,
        libc::ENOTEMPTY => ErrorKind::DirectoryNotEmpty,
        libc::EINVAL => ErrorKind::InvalidInput,
        libc::EAGAIN => ErrorKind::WouldBlock,
        libc::EINTR => ErrorKind::Interrupted,
        libc::EPIPE => ErrorKind::BrokenPipe,
        // R2/R3 containment (openat2 RESOLVE_NO_SYMLINKS/RESOLVE_NO_XDEV,
        // sys::openat2) and the pre-existing "rename across filesystems"
        // case (renameat2, unrelated to R2 but the identical errno) —
        // see `ErrorKind::FilesystemLoop`/`CrossesDevices`'s own doc
        // comments for why both share one variant apiece.
        libc::ELOOP => ErrorKind::FilesystemLoop,
        libc::EXDEV => ErrorKind::CrossesDevices,
        _ => ErrorKind::Other,
    }
}

fn os_err(op: &'static str, path: &OsStr) -> PlatformError {
    let e = errno();
    PlatformError::new(kind_of(e), OsCode::Errno(e), op).with_path(path)
}

fn to_cstring(path: &OsStr, op: &'static str) -> Result<CString> {
    CString::new(path.as_bytes())
        .map_err(|_| PlatformError::new(ErrorKind::InvalidInput, OsCode::None, op).with_path(path))
}

/// `openat(dirfd, rel, flags, mode)` returning an owned fd.
///
/// `flags` are `O_*` values; on Linux the libc crate's and the kernel's
/// values are the same numbers, so both backends accept them unchanged.
#[cfg(not(feature = "track-p"))]
pub fn openat(dirfd: RawFd, rel: &OsStr, flags: i32, mode: u32) -> Result<OwnedFd> {
    let c_rel = to_cstring(rel, "openat")?;
    // SAFETY: `c_rel` is a valid NUL-terminated string that outlives the
    // call; `dirfd` is a valid open descriptor owned by our caller; openat
    // has no other pointer parameters.
    let fd = unsafe { c::openat(dirfd, c_rel.as_ptr(), flags | c::O_CLOEXEC, mode) };
    if fd < 0 {
        return Err(os_err("openat", rel));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapping it exactly once transfers ownership.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Kernel ABI layout for `openat2(2)`'s `struct open_how`
/// (`linux/openat2.h`): three `u64`s, no padding, in this exact order.
/// Defined locally rather than using libc's own `open_how` because that
/// type is `#[non_exhaustive]` (see `libc_surface`'s doc comment) and so
/// cannot be literal-constructed outside the `libc` crate at all, even
/// naming every field.
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

/// R2 containment (Rusty-Mill fs slice): `RESOLVE_NO_SYMLINKS` (reject
/// *any* symlink encountered while resolving the path — every
/// intermediate component, atomically, the R1→R2 "link-confined" gap
/// this slice closes) `|` `RESOLVE_NO_XDEV` (reject crossing a
/// filesystem/mount boundary during that same resolution —
/// "mount-confined", R2's own addition on top of R1). Deliberately not
/// `RESOLVE_BENEATH`/`RESOLVE_IN_ROOT`: this crate's `Dir` contract does
/// not promise beneath-confinement today (nothing in `docs/behavior/
/// fs.md` stops a relative `rel` containing `..`, or a symlink whose
/// *target* is absolute, from resolving outside the capability's own
/// subtree — that would be a real, separate, and currently undocumented
/// behavior narrowing, not a side effect of closing this gap) — an R3
/// slice would need to *design* that contract first, not adopt it as an
/// accident of which flags happened to be convenient here.
const R2_RESOLVE: u64 = c::RESOLVE_NO_SYMLINKS | c::RESOLVE_NO_XDEV;

/// `openat2(dirfd, rel, &how)` — raw syscall: no libc wrapper exists at
/// this repo's MSRV baseline (glibc grew its own `openat2()` well after
/// this crate's floor), the same situation `renameat2`/`pidfd_open` were
/// in before their own slices landed (`libc_surface`'s doc comment).
/// `ENOSYS` (pre-5.6 kernel, or a `resolve` bit the running kernel
/// doesn't implement) comes back as `ErrorKind::Unsupported` rather than
/// a hard failure — callers fall back to the plain `openat` path, the
/// exact "try the modern mechanism, fall back to the portable one on an
/// old kernel" shape `sys::spawn::poll_pids` already established for
/// `pidfd_open` (RFC v2 §5.6).
fn openat2_raw(dirfd: RawFd, rel: &OsStr, flags: i32, mode: u32, resolve: u64) -> Result<OwnedFd> {
    let c_rel = to_cstring(rel, "openat2")?;
    let how = OpenHow {
        flags: (flags | c::O_CLOEXEC) as u64,
        mode: u64::from(mode),
        resolve,
    };
    // SAFETY: `c_rel` is a valid NUL-terminated string outliving the
    // call; `how` is a fully initialized, correctly laid-out `open_how`
    // (repr(C), field-for-field matching the kernel UAPI struct) whose
    // address and exact `size_of` are passed together, satisfying
    // openat2's own extensible-struct-by-size ABI contract; `dirfd` is a
    // valid open descriptor owned by our caller.
    let fd = unsafe {
        c::syscall(
            c::SYS_openat2,
            dirfd,
            c_rel.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        let code = errno();
        if code == libc::ENOSYS {
            return Err(PlatformError::new(
                ErrorKind::Unsupported,
                OsCode::Errno(code),
                "openat2",
            ));
        }
        return Err(
            PlatformError::new(kind_of(code), OsCode::Errno(code), "openat2").with_path(rel),
        );
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// `openat2` with [`R2_RESOLVE`], transparently falling back to plain
/// `openat` — today's unchanged R1 behavior, symlinks and mount
/// crossings both silently followed — when the kernel has no `openat2`
/// at all. Used by [`crate::fs::LinuxDir::open_dir`] and the
/// parent-resolution half of [`crate::fs::LinuxDir::create_dir`] only:
/// see each call site's own doc comment for why `Dir::open`'s plain-file
/// case is deliberately excluded (it would silently stop following a
/// *terminal* symlink, breaking `docs/behavior/fs.md`'s existing "open
/// follows symlinks transparently" promise, not just add a guarantee).
pub fn openat_r2(dirfd: RawFd, rel: &OsStr, flags: i32, mode: u32) -> Result<OwnedFd> {
    match openat2_raw(dirfd, rel, flags, mode, R2_RESOLVE) {
        Ok(fd) => Ok(fd),
        Err(e) if e.kind == ErrorKind::Unsupported => openat(dirfd, rel, flags, mode),
        Err(e) => Err(e),
    }
}

/// Whether the running kernel implements `openat2` at all (5.6+),
/// probed once (against `AT_FDCWD`/`"."`, with `resolve = 0` — this
/// probe is "does the syscall exist," not itself an R2 containment
/// check) and cached. Exists so a symlink/mount-containment test can
/// honestly skip on an old kernel instead of silently exercising the R1
/// fallback path inside [`openat_r2`] and asserting the wrong thing —
/// the same `tun_or_skip!`-style honesty `tests/tun_parity.rs` already
/// practices for its own environment gap.
pub fn openat2_supported() -> bool {
    use std::sync::OnceLock;
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        !matches!(
            openat2_raw(c::AT_FDCWD, OsStr::new("."), c::O_RDONLY | c::O_DIRECTORY, 0, 0),
            Err(e) if e.kind == ErrorKind::Unsupported
        )
    })
}

/// `memfd_create(2)` — an anonymous, memory-backed file with no
/// filesystem namespace entry at all, unlinked from the moment it's
/// created. `name` is a debugging label only (visible in
/// `/proc/self/fd/<n>`), not a path — no directory fd, no `AT_FDCWD`,
/// nothing to resolve. Not track-p-gated: `memfd_create` has had a
/// real libc wrapper since long before this crate existed, so there is
/// exactly one implementation, the same treatment `fsync` gets.
pub fn memfd_create(name: &str) -> Result<OwnedFd> {
    let c_name = CString::new(name).map_err(|_| {
        PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "memfd_create")
            .with_path(OsStr::new(name))
    })?;
    // SAFETY: `c_name` is a valid NUL-terminated string that outlives
    // the call; `MFD_CLOEXEC` matches every fd this crate creates
    // (RFC v2's CLOEXEC-by-default discipline — `openat` above ORs it
    // into every open unconditionally).
    let fd = unsafe { c::memfd_create(c_name.as_ptr(), c::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(os_err("memfd_create", OsStr::new(name)));
    }
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapping it exactly once transfers ownership.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `openat(dirfd, rel, flags, mode)` — Track P: raw `SYS_openat`.
#[cfg(feature = "track-p")]
pub fn openat(dirfd: RawFd, rel: &OsStr, flags: i32, mode: u32) -> Result<OwnedFd> {
    let c_rel = to_cstring(rel, "openat")?;
    let fd = rusty_libc::fd::openat(dirfd, &c_rel, flags | rusty_libc::fd::O_CLOEXEC, mode)
        .map_err(|e| trackp_err("openat", e).with_path(rel))?;
    // SAFETY: `fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapping it exactly once transfers ownership.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `read(2)` into `buf`.
#[cfg(not(feature = "track-p"))]
pub fn read(fd: &OwnedFd, buf: &mut [u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid, writable region of exactly `buf.len()`
    // bytes for the duration of the call; `fd` is a valid open descriptor.
    let n = unsafe { c::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(os_err("read", OsStr::new("")));
    }
    Ok(n as usize)
}

/// `read(2)` into `buf` — Track P: raw `SYS_read`, no libc in the path.
/// rusty_libc's safe wrapper derives the pointer/length pair from the slice
/// itself, so no unsafe block appears at this call site.
#[cfg(feature = "track-p")]
pub fn read(fd: &OwnedFd, buf: &mut [u8]) -> Result<usize> {
    rusty_libc::fd::read(fd.as_raw_fd(), buf).map_err(|e| trackp_err("read", e))
}

/// `write(2)` from `buf`.
#[cfg(not(feature = "track-p"))]
pub fn write(fd: &OwnedFd, buf: &[u8]) -> Result<usize> {
    // SAFETY: `buf` is a valid readable region of exactly `buf.len()`
    // bytes for the duration of the call; `fd` is a valid open descriptor.
    let n = unsafe { c::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(os_err("write", OsStr::new("")));
    }
    Ok(n as usize)
}

/// `write(2)` from `buf` — Track P: raw `SYS_write`, no libc in the path.
#[cfg(feature = "track-p")]
pub fn write(fd: &OwnedFd, buf: &[u8]) -> Result<usize> {
    rusty_libc::fd::write(fd.as_raw_fd(), buf).map_err(|e| trackp_err("write", e))
}

/// `fcntl(fd, F_DUPFD_CLOEXEC, 0)` — `File::try_clone` (D5, rustils#51):
/// a fresh fd sharing the same open-file description (position
/// included) as `fd`, with `CLOEXEC` set atomically on the duplicate so
/// it never leaks into an unrelated future child by accident — spawn-
/// time inheritance for a `Stdio::File` slot is a separate, explicit
/// step (`sys::spawn`'s own `Stdio::File` handling), not this function's
/// concern. Not track-p-gated: like `fsync`/`pidfd_open`, rusty_libc has
/// no `fcntl` surface of its own yet (outside its ~25-syscall
/// inventory) — one implementation for both configurations.
pub fn dup_cloexec(fd: &OwnedFd) -> Result<OwnedFd> {
    // SAFETY: `fd` is a valid open descriptor; `F_DUPFD_CLOEXEC` takes a
    // plain integer (the minimum fd number to return, `0` = any) as its
    // sole variadic argument.
    let new_fd = unsafe { c::fcntl(fd.as_raw_fd(), c::F_DUPFD_CLOEXEC, 0) };
    if new_fd < 0 {
        return Err(os_err("fcntl(F_DUPFD_CLOEXEC)", OsStr::new("")));
    }
    // SAFETY: `new_fd` is a freshly returned, valid, otherwise-unowned
    // descriptor; wrapped exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(new_fd) })
}

/// `mkdirat(dirfd, rel, 0o777)` (mode filtered by umask, as usual).
#[cfg(not(feature = "track-p"))]
pub fn mkdirat(dirfd: RawFd, rel: &OsStr) -> Result<()> {
    let c_rel = to_cstring(rel, "mkdirat")?;
    // SAFETY: valid NUL-terminated path outliving the call; valid dirfd.
    let r = unsafe { c::mkdirat(dirfd, c_rel.as_ptr(), 0o777) };
    if r < 0 {
        return Err(os_err("mkdirat", rel));
    }
    Ok(())
}

/// `mkdirat(dirfd, rel, 0o777)` — Track P: raw `SYS_mkdirat`.
#[cfg(feature = "track-p")]
pub fn mkdirat(dirfd: RawFd, rel: &OsStr) -> Result<()> {
    let c_rel = to_cstring(rel, "mkdirat")?;
    rusty_libc::fs::mkdirat(dirfd, &c_rel, 0o777)
        .map_err(|e| trackp_err("mkdirat", e).with_path(rel))
}

/// `unlinkat(dirfd, rel, flags)`.
#[cfg(not(feature = "track-p"))]
pub fn unlinkat(dirfd: RawFd, rel: &OsStr, remove_dir: bool) -> Result<()> {
    let c_rel = to_cstring(rel, "unlinkat")?;
    let flags = if remove_dir { c::AT_REMOVEDIR } else { 0 };
    // SAFETY: valid NUL-terminated path outliving the call; valid dirfd.
    let r = unsafe { c::unlinkat(dirfd, c_rel.as_ptr(), flags) };
    if r < 0 {
        return Err(os_err("unlinkat", rel));
    }
    Ok(())
}

/// `unlinkat(dirfd, rel, flags)` — Track P: raw `SYS_unlinkat`.
#[cfg(feature = "track-p")]
pub fn unlinkat(dirfd: RawFd, rel: &OsStr, remove_dir: bool) -> Result<()> {
    let c_rel = to_cstring(rel, "unlinkat")?;
    let flags = if remove_dir {
        rusty_libc::fs::AT_REMOVEDIR
    } else {
        0
    };
    rusty_libc::fs::unlinkat(dirfd, &c_rel, flags)
        .map_err(|e| trackp_err("unlinkat", e).with_path(rel))
}

/// `symlinkat(target, dirfd, link_name)` — creates `link_name` (relative
/// to `dirfd`) as a symlink storing `target` verbatim. Note the argument
/// order: `target` is not itself resolved relative to `dirfd`, only
/// `link_name` is.
#[cfg(not(feature = "track-p"))]
pub fn symlink(dirfd: RawFd, target: &OsStr, link_name: &OsStr) -> Result<()> {
    let c_target = to_cstring(target, "symlinkat")?;
    let c_link = to_cstring(link_name, "symlinkat")?;
    // SAFETY: both paths are valid NUL-terminated strings outliving the
    // call; `dirfd` is a valid open descriptor for the link-name end.
    let r = unsafe { c::symlinkat(c_target.as_ptr(), dirfd, c_link.as_ptr()) };
    if r < 0 {
        return Err(os_err("symlinkat", link_name));
    }
    Ok(())
}

/// `symlinkat(target, dirfd, link_name)` — Track P: `rusty_libc::fs::symlinkat`.
#[cfg(feature = "track-p")]
pub fn symlink(dirfd: RawFd, target: &OsStr, link_name: &OsStr) -> Result<()> {
    let c_target = to_cstring(target, "symlinkat")?;
    let c_link = to_cstring(link_name, "symlinkat")?;
    rusty_libc::fs::symlinkat(&c_target, dirfd, &c_link)
        .map_err(|e| trackp_err("symlinkat", e).with_path(link_name))
}

/// `readlinkat(dirfd, rel, buf)` — the stored target, not resolved. Grows
/// the buffer and retries when the target may have been truncated (the
/// syscall gives no explicit truncation signal beyond "filled the buffer
/// exactly"), capped well past any real-world target length.
#[cfg(not(feature = "track-p"))]
pub fn read_link(dirfd: RawFd, rel: &OsStr) -> Result<OsString> {
    let c_rel = to_cstring(rel, "readlinkat")?;
    let mut cap = 256usize;
    loop {
        let mut buf = vec![0u8; cap];
        // SAFETY: `buf` is a valid writable region of `cap` bytes
        // outliving the call; `c_rel` is a valid NUL-terminated path;
        // `dirfd` is a valid open descriptor.
        let n = unsafe { c::readlinkat(dirfd, c_rel.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(os_err("readlinkat", rel));
        }
        let n = n as usize;
        if n < cap {
            buf.truncate(n);
            return Ok(OsString::from_vec(buf));
        }
        cap *= 4;
        if cap > 1 << 20 {
            return Err(
                PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "readlinkat")
                    .with_path(rel),
            );
        }
    }
}

/// `readlinkat(dirfd, rel, buf)` — Track P: `rusty_libc::fs::readlinkat`.
#[cfg(feature = "track-p")]
pub fn read_link(dirfd: RawFd, rel: &OsStr) -> Result<OsString> {
    let c_rel = to_cstring(rel, "readlinkat")?;
    let mut cap = 256usize;
    loop {
        let mut buf = vec![0u8; cap];
        match rusty_libc::fs::readlinkat(dirfd, &c_rel, &mut buf) {
            Ok(bytes) => {
                let n = bytes.len();
                if n < cap {
                    buf.truncate(n);
                    return Ok(OsString::from_vec(buf));
                }
            }
            Err(e) => return Err(trackp_err("readlinkat", e).with_path(rel)),
        }
        cap *= 4;
        if cap > 1 << 20 {
            return Err(
                PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "readlinkat")
                    .with_path(rel),
            );
        }
    }
}

/// Reconstructs a [`SystemTime`] from a POSIX `(seconds, nanoseconds)`
/// pair, where `nanoseconds` is always in `0..1_000_000_000` regardless
/// of the sign of `seconds` (the `timespec` convention every stat-family
/// call here follows) — a plain "seconds since epoch as `i64`" cast
/// would get pre-1970 timestamps wrong by simply flipping the sign of
/// the nanosecond remainder too.
fn system_time_from_secs_nsec(secs: i64, nsec: u32) -> SystemTime {
    if secs >= 0 {
        SystemTime::UNIX_EPOCH + Duration::new(secs as u64, nsec)
    } else {
        SystemTime::UNIX_EPOCH - Duration::new((-secs) as u64, 0) + Duration::new(0, nsec)
    }
}

/// `fstatat` returning (file type, size, link count, mtime).
#[cfg(not(feature = "track-p"))]
pub fn statat(dirfd: RawFd, rel: &OsStr) -> Result<(FileType, u64, u64, SystemTime)> {
    let c_rel = to_cstring(rel, "fstatat")?;
    // SAFETY: `stat` is a plain-old-data struct for which the all-zeroes
    // bit pattern is a valid (if meaningless) value; the kernel overwrites
    // it on success.
    let mut st: c::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid path pointer and out-pointer to a properly sized
    // `stat` struct, both outliving the call; valid dirfd.
    let r = unsafe { c::fstatat(dirfd, c_rel.as_ptr(), &mut st, c::AT_SYMLINK_NOFOLLOW) };
    if r < 0 {
        return Err(os_err("fstatat", rel));
    }
    let ft = match st.st_mode & c::S_IFMT {
        m if m == c::S_IFREG => FileType::File,
        m if m == c::S_IFDIR => FileType::Dir,
        m if m == c::S_IFLNK => FileType::Symlink,
        _ => FileType::Other,
    };
    let modified = system_time_from_secs_nsec(st.st_mtime, st.st_mtime_nsec as u32);
    Ok((ft, st.st_size as u64, st.st_nlink, modified))
}

/// Metadata via raw `SYS_statx` — Track P. There is no raw `fstatat` worth
/// wanting: `statx` is the kernel's own extended-stat interface, the buffer
/// layout is kernel-defined (no glibc `struct stat` translation shim), and
/// the mode/size fields live at const-asserted offsets in rusty_libc.
#[cfg(feature = "track-p")]
pub fn statat(dirfd: RawFd, rel: &OsStr) -> Result<(FileType, u64, u64, SystemTime)> {
    use rusty_libc::fs as rfs;
    let c_rel = to_cstring(rel, "statx")?;
    let st = rfs::statx(
        dirfd,
        &c_rel,
        rfs::AT_SYMLINK_NOFOLLOW,
        rfs::STATX_BASIC_STATS,
    )
    .map_err(|e| trackp_err("statx", e).with_path(rel))?;
    let ft = match st.file_type() {
        rfs::S_IFREG => FileType::File,
        rfs::S_IFDIR => FileType::Dir,
        rfs::S_IFLNK => FileType::Symlink,
        _ => FileType::Other,
    };
    let modified = system_time_from_secs_nsec(st.stx_mtime.tv_sec, st.stx_mtime.tv_nsec);
    Ok((ft, st.stx_size, u64::from(st.stx_nlink), modified))
}

/// `S_ISUID`/`S_ISGID`/`S_ISVTX` decoded from `fstatat`'s `st_mode`, plus
/// owning `st_uid`/`st_gid` — `test`'s `-u/-g/-k/-O/-G` donor material
/// (D11). `AT_SYMLINK_NOFOLLOW`, matching `statat`'s lstat-style
/// contract (the object itself, not a followed target).
#[cfg(not(feature = "track-p"))]
pub fn unix_mode(dirfd: RawFd, rel: &OsStr) -> Result<UnixMode> {
    let c_rel = to_cstring(rel, "fstatat")?;
    // SAFETY: `stat` is a plain-old-data struct for which the all-zeroes
    // bit pattern is a valid (if meaningless) value; the kernel overwrites
    // it on success.
    let mut st: c::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid path pointer and out-pointer to a properly sized
    // `stat` struct, both outliving the call; valid dirfd.
    let r = unsafe { c::fstatat(dirfd, c_rel.as_ptr(), &mut st, c::AT_SYMLINK_NOFOLLOW) };
    if r < 0 {
        return Err(os_err("fstatat", rel));
    }
    Ok(UnixMode {
        setuid: st.st_mode & c::S_ISUID != 0,
        setgid: st.st_mode & c::S_ISGID != 0,
        sticky: st.st_mode & c::S_ISVTX != 0,
        uid: st.st_uid,
        gid: st.st_gid,
        permissions: (st.st_mode & 0o777) as u16,
    })
}

/// `S_ISUID`/`S_ISGID`/`S_ISVTX` — Track P: raw `SYS_statx`, the same
/// call `statat`'s track-p arm makes.
#[cfg(feature = "track-p")]
pub fn unix_mode(dirfd: RawFd, rel: &OsStr) -> Result<UnixMode> {
    use rusty_libc::fs as rfs;
    let c_rel = to_cstring(rel, "statx")?;
    let st = rfs::statx(
        dirfd,
        &c_rel,
        rfs::AT_SYMLINK_NOFOLLOW,
        rfs::STATX_BASIC_STATS,
    )
    .map_err(|e| trackp_err("statx", e).with_path(rel))?;
    let mode = u32::from(st.stx_mode);
    Ok(UnixMode {
        setuid: mode & c::S_ISUID != 0,
        setgid: mode & c::S_ISGID != 0,
        sticky: mode & c::S_ISVTX != 0,
        uid: st.stx_uid,
        gid: st.stx_gid,
        permissions: (mode & 0o777) as u16,
    })
}

/// `fchmodat(dirfd, rel, mode, 0)` — `chmod`'s write-side counterpart to
/// `unix_mode` (coreutils gap backlog #64). No `AT_SYMLINK_NOFOLLOW`:
/// Linux's kernel does not implement changing a symlink's own
/// permissions (fails `ENOTSUP` on every real filesystem — symlink mode
/// bits are unused and ignored), so this follows the terminal symlink
/// like `chmod(1)` does.
#[cfg(not(feature = "track-p"))]
pub fn set_unix_mode(dirfd: RawFd, rel: &OsStr, mode: platform::fs::Mode) -> Result<()> {
    let c_rel = to_cstring(rel, "fchmodat")?;
    let raw_mode: u32 = (if mode.setuid { c::S_ISUID } else { 0 }
        | if mode.setgid { c::S_ISGID } else { 0 }
        | if mode.sticky { c::S_ISVTX } else { 0 })
        | (u32::from(mode.permissions) & 0o777);
    // SAFETY: `c_rel` is a valid NUL-terminated string outliving the
    // call; valid dirfd; fchmodat has no other pointer parameters.
    let r = unsafe { c::fchmodat(dirfd, c_rel.as_ptr(), raw_mode, 0) };
    if r < 0 {
        return Err(os_err("fchmodat", rel));
    }
    Ok(())
}

/// Track P: `rusty_libc`'s `fs` module has no `chmod`/`fchmodat` binding
/// yet at the pinned rev (unlike `statx`/`faccessat`/`renameat2`, which
/// it does have) — rather than silently falling back to libc's own
/// `fchmodat` (defeating the entire point of `track-p`: zero libc
/// *function* calls on the hot path), this arm is `Unsupported` until
/// rusty_libc grows the primitive and this gets its own call-by-call
/// replacement, per RFC v2 §2's Track P plan.
#[cfg(feature = "track-p")]
pub fn set_unix_mode(_dirfd: RawFd, rel: &OsStr, _mode: platform::fs::Mode) -> Result<()> {
    Err(PlatformError::new(
        ErrorKind::Unsupported,
        OsCode::None,
        "fchmodat (rusty_libc has no chmod primitive yet)",
    )
    .with_path(rel))
}

/// `(dev, ino)` from `fstatat` — `test -ef`'s donor material (D11),
/// wrapped into the opaque `platform::fs::FileId` by the `Dir` impl.
/// `AT_SYMLINK_NOFOLLOW`, matching `statat`.
#[cfg(not(feature = "track-p"))]
pub fn file_id(dirfd: RawFd, rel: &OsStr) -> Result<(u64, u64)> {
    let c_rel = to_cstring(rel, "fstatat")?;
    // SAFETY: see `unix_mode`.
    let mut st: c::stat = unsafe { std::mem::zeroed() };
    // SAFETY: see `unix_mode`.
    let r = unsafe { c::fstatat(dirfd, c_rel.as_ptr(), &mut st, c::AT_SYMLINK_NOFOLLOW) };
    if r < 0 {
        return Err(os_err("fstatat", rel));
    }
    Ok((st.st_dev, st.st_ino))
}

/// `(dev, ino)` — Track P: raw `SYS_statx`. The major/minor device
/// numbers `statx` reports separately are bit-packed into one `u64` —
/// only equality within a single running process matters for `test
/// -ef`, not a stable cross-process or cross-config encoding.
#[cfg(feature = "track-p")]
pub fn file_id(dirfd: RawFd, rel: &OsStr) -> Result<(u64, u64)> {
    use rusty_libc::fs as rfs;
    let c_rel = to_cstring(rel, "statx")?;
    let st = rfs::statx(
        dirfd,
        &c_rel,
        rfs::AT_SYMLINK_NOFOLLOW,
        rfs::STATX_BASIC_STATS,
    )
    .map_err(|e| trackp_err("statx", e).with_path(rel))?;
    let dev = (u64::from(st.stx_dev_major) << 32) | u64::from(st.stx_dev_minor);
    Ok((dev, st.stx_ino))
}

/// `faccessat(dirfd, rel, mode, 0)` — real, not effective, uid/gid (the
/// bare syscall's own semantics; `libc_surface`'s doc comment has the
/// track-p-consistency rationale for not requesting `AT_EACCESS`).
#[cfg(not(feature = "track-p"))]
pub fn access(dirfd: RawFd, rel: &OsStr, mode: i32) -> Result<()> {
    let c_rel = to_cstring(rel, "faccessat")?;
    // SAFETY: valid NUL-terminated path outliving the call; valid dirfd.
    let r = unsafe { c::faccessat(dirfd, c_rel.as_ptr(), mode, 0) };
    if r < 0 {
        return Err(os_err("faccessat", rel));
    }
    Ok(())
}

/// `faccessat(dirfd, rel, mode)` — Track P: `rusty_libc::fs::faccessat`,
/// which has no flags parameter at all (real ids only), matching the
/// non-track-p arm's explicit `0`.
#[cfg(feature = "track-p")]
pub fn access(dirfd: RawFd, rel: &OsStr, mode: i32) -> Result<()> {
    let c_rel = to_cstring(rel, "faccessat")?;
    rusty_libc::fs::faccessat(dirfd, &c_rel, mode)
        .map_err(|e| trackp_err("faccessat", e).with_path(rel))
}

/// `renameat2(olddirfd, old, newdirfd, new, flags)` — same directory on
/// both ends (D11's Fs second wave). `flags` is `0` (replace) or
/// `RENAME_NOREPLACE`.
///
/// No libc *wrapper function* exists at this repo's MSRV baseline on
/// the glibc x86_64 target (the same situation `pidfd_open` was in) —
/// the raw syscall via `SYS_renameat2`.
#[cfg(not(feature = "track-p"))]
fn renameat2_raw(dirfd: RawFd, old: &OsStr, new: &OsStr, flags: u32) -> Result<()> {
    let c_old = to_cstring(old, "renameat2")?;
    let c_new = to_cstring(new, "renameat2")?;
    // SAFETY: both paths are valid NUL-terminated strings outliving the
    // call; `dirfd` is a valid open descriptor used for both ends
    // (rename within one directory); renameat2 has no other pointer
    // parameters.
    let r = unsafe {
        c::syscall(
            c::SYS_renameat2,
            dirfd,
            c_old.as_ptr(),
            dirfd,
            c_new.as_ptr(),
            flags,
        )
    };
    if r < 0 {
        return Err(os_err("renameat2", old));
    }
    Ok(())
}

/// Track P: rusty_libc has `renameat2` directly (unlike `pidfd_open`),
/// so this — unlike the escape-hatch functions — follows the ordinary
/// split shape every other fdio call in this module uses.
#[cfg(feature = "track-p")]
fn renameat2_raw(dirfd: RawFd, old: &OsStr, new: &OsStr, flags: u32) -> Result<()> {
    let c_old = to_cstring(old, "renameat2")?;
    let c_new = to_cstring(new, "renameat2")?;
    rusty_libc::fs::renameat2(dirfd, &c_old, dirfd, &c_new, flags)
        .map_err(|e| trackp_err("renameat2", e).with_path(old))
}

pub fn rename(dirfd: RawFd, from: &OsStr, to: &OsStr) -> Result<()> {
    renameat2_raw(dirfd, from, to, 0)
}

pub fn rename_no_replace(dirfd: RawFd, from: &OsStr, to: &OsStr) -> Result<()> {
    #[cfg(not(feature = "track-p"))]
    let flag = c::RENAME_NOREPLACE;
    #[cfg(feature = "track-p")]
    let flag = rusty_libc::fs::RENAME_NOREPLACE;
    renameat2_raw(dirfd, from, to, flag)
}

/// `fsync(fd)` — durability (`File::sync_all`). Not track-p-gated: like
/// `pidfd_open`, rusty_libc has no `fsync` yet (outside its current
/// ~25-syscall surface), and `fsync` has no interesting arguments to
/// route through a raw-syscall story either way — one implementation
/// for both configurations, the same treatment `pidfd_open` gets in
/// `sys/spawn.rs`.
pub fn fsync(fd: &OwnedFd) -> Result<()> {
    // SAFETY: `fd` is a valid open descriptor; fsync has no pointer
    // parameters.
    let r = unsafe { c::fsync(fd.as_raw_fd()) };
    if r < 0 {
        return Err(os_err("fsync", OsStr::new("")));
    }
    Ok(())
}

/// Enumerate a directory via `fdopendir`/`readdir`, consuming an fd opened
/// with `O_DIRECTORY`. Returns (name, file type) pairs, excluding `.`/`..`.
///
/// glibc-only: `readdir`'s `DIR*` stream has no raw-syscall equivalent of
/// its own (it's userspace buffering over `getdents64`) — the track-p arm
/// below bypasses it entirely instead of reimplementing it.
#[cfg(not(feature = "track-p"))]
pub fn read_dir(dirfd: OwnedFd) -> Result<Vec<(OsString, FileType)>> {
    use std::os::fd::IntoRawFd;
    // SAFETY: `into_raw_fd` transfers ownership of a valid directory fd to
    // fdopendir, which takes ownership on success; on failure we must (and
    // do) reconstruct and drop the fd to avoid a leak.
    let dir: *mut c::DIR = unsafe { c::fdopendir(dirfd.as_raw_fd()) };
    if dir.is_null() {
        return Err(os_err("fdopendir", OsStr::new("")));
    }
    // fdopendir now owns the fd; into_raw_fd relinquishes our ownership so
    // Drop will not close it.
    let _ = dirfd.into_raw_fd();

    let mut out = Vec::new();
    loop {
        // SAFETY: `dir` is the valid stream returned above and is only
        // used on this thread; readdir's returned pointer is valid until
        // the next readdir on the same stream, and we copy out of it
        // before looping.
        let ent: *const c::dirent = unsafe { c::readdir(dir) };
        if ent.is_null() {
            break;
        }
        // SAFETY: non-null `ent` points at a valid dirent whose d_name is
        // NUL-terminated per POSIX.
        let (name_bytes, d_type) = unsafe {
            let name = CStr::from_ptr((*ent).d_name.as_ptr()).to_bytes().to_vec();
            (name, (*ent).d_type)
        };
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let ft = match d_type {
            t if t == c::DT_REG => FileType::File,
            t if t == c::DT_DIR => FileType::Dir,
            t if t == c::DT_LNK => FileType::Symlink,
            _ => FileType::Other,
        };
        out.push((OsString::from_vec(name_bytes), ft));
    }
    // SAFETY: `dir` is a valid open stream not used after this call.
    unsafe { libc::closedir(dir) };
    Ok(out)
}

/// Enumerate a directory via raw `getdents64`, consuming an fd opened
/// with `O_DIRECTORY`. Returns (name, file type) pairs, excluding
/// `.`/`..`. Track P: this — not the `fdopendir`/`readdir` `DIR*` stream
/// above, which is glibc userspace buffering with no raw-syscall
/// equivalent of its own — closes the last gap the convergence roadmap's
/// Phase 4 named (`rusty_libc::fs::getdents64`/`dirents`).
///
/// Unlike the non-track-p arm, `dirfd` needs no ownership hand-off:
/// `getdents64` operates directly on the fd we already own, so it just
/// closes normally when `dirfd` drops at the end of this function.
#[cfg(feature = "track-p")]
pub fn read_dir(dirfd: OwnedFd) -> Result<Vec<(OsString, FileType)>> {
    use rusty_libc::fs as rfs;
    let fd = dirfd.as_raw_fd();
    let mut out = Vec::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = rfs::getdents64(fd, &mut buf).map_err(|e| trackp_err("getdents64", e))?;
        if n == 0 {
            break;
        }
        for entry in rfs::dirents(&buf[..n]) {
            if entry.d_name == b"." || entry.d_name == b".." {
                continue;
            }
            let ft = match entry.d_type {
                rfs::DT_REG => FileType::File,
                rfs::DT_DIR => FileType::Dir,
                rfs::DT_LNK => FileType::Symlink,
                _ => FileType::Other,
            };
            out.push((OsString::from_vec(entry.d_name.to_vec()), ft));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod memfd_tests {
    use super::*;

    #[test]
    fn memfd_create_gives_a_real_readable_writable_fd() {
        let fd = memfd_create("rustils-test").expect("memfd_create should succeed");
        // SAFETY: `fd` was just returned by memfd_create above, open for
        // read+write (memfd_create's own default), and not used elsewhere
        // in this test.
        let n = unsafe { c::write(fd.as_raw_fd(), b"hello".as_ptr().cast(), 5) };
        assert_eq!(n, 5, "write into a fresh memfd should succeed in full");

        // SAFETY: `fd` is still open; lseek is a plain libc call with no
        // pointer arguments.
        unsafe { c::lseek(fd.as_raw_fd(), 0, c::SEEK_SET) };
        let mut buf = [0u8; 5];
        // SAFETY: `buf` is a valid 5-byte writable region for the
        // duration of the call.
        let n = unsafe { c::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello", "read back exactly what was written");
    }

    #[test]
    fn memfd_create_is_genuinely_unlinked() {
        let fd = memfd_create("rustils-unlinked-check").expect("memfd_create should succeed");
        // Real Linux ground truth, not just a successful syscall return:
        // /proc/self/fd/<n> for a memfd resolves to "/memfd:<name>
        // (deleted)" — the kernel's own way of saying this fd has never
        // had, and can never regain, a directory entry. The same
        // discipline the PTY surface's own tests use against
        // /proc/<pid>/stat rather than trusting a bare Ok(()) (see
        // docs/behavior/pty.md).
        let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
            .expect("the memfd should have a /proc/self/fd entry to read");
        let link = link.to_string_lossy();
        assert!(
            link.contains("(deleted)"),
            "a memfd's /proc/self/fd link should always show (deleted): got {link:?}"
        );
        assert!(
            link.contains("rustils-unlinked-check"),
            "the debug label should appear in the /proc link: got {link:?}"
        );
    }

    #[test]
    fn two_memfds_are_independent() {
        let fd_a = memfd_create("a").expect("memfd_create a");
        let fd_b = memfd_create("b").expect("memfd_create b");
        // SAFETY: `fd_a` is freshly created and open for writing.
        unsafe { c::write(fd_a.as_raw_fd(), b"AAAAA".as_ptr().cast(), 5) };
        // SAFETY: `fd_b` is freshly created and open for writing.
        unsafe { c::write(fd_b.as_raw_fd(), b"BBBBB".as_ptr().cast(), 5) };

        for fd in [&fd_a, &fd_b] {
            // SAFETY: each `fd` in this loop is still open; lseek takes
            // no pointers.
            unsafe { c::lseek(fd.as_raw_fd(), 0, c::SEEK_SET) };
        }
        let mut buf_a = [0u8; 5];
        let mut buf_b = [0u8; 5];
        // SAFETY: `buf_a` is a valid 5-byte writable region for the
        // duration of this call.
        unsafe { c::read(fd_a.as_raw_fd(), buf_a.as_mut_ptr().cast(), 5) };
        // SAFETY: `buf_b` is a valid 5-byte writable region for the
        // duration of this call.
        unsafe { c::read(fd_b.as_raw_fd(), buf_b.as_mut_ptr().cast(), 5) };
        assert_eq!(&buf_a, b"AAAAA");
        assert_eq!(&buf_b, b"BBBBB");
    }
}
