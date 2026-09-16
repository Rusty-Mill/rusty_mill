//! Filesystem operations: file metadata via `statx`, access checks via
//! `faccessat`, and the path-mutating `*at` syscalls (`unlinkat`, `mkdirat`,
//! `renameat2`, `symlinkat`, `readlinkat`).
//!
//! Every entry point is an `*at`-form syscall taking an explicit `dirfd`;
//! [`AT_FDCWD`] resolves relative paths against the current working directory,
//! and thin `unlink`/`mkdir`/`rename`/`stat`/… wrappers pass it for you.
//! aarch64 has no legacy `stat`/`unlink`/`rename`, so building on the `*at`
//! forms keeps both arches identical.

use crate::arch::nr;
use crate::arch::{from_ret, syscall2, syscall3, syscall4, syscall5, Errno};
use core::ffi::CStr;

pub use crate::fd::AT_FDCWD;

/// `*at` flag: do not follow a terminal symlink (operate on the link itself).
pub const AT_SYMLINK_NOFOLLOW: i32 = 0x100;
/// `*at` flag: operate on `dirfd` itself when the path is empty (e.g. `fstat`
/// via [`statx`] on an open fd).
pub const AT_EMPTY_PATH: i32 = 0x1000;
/// [`unlinkat`] flag: remove a directory instead of a file (like `rmdir`).
pub const AT_REMOVEDIR: i32 = 0x200;

// --- access(2) mode bits ------------------------------------------------------

/// [`faccessat`] mode: test for existence only.
pub const F_OK: i32 = 0;
/// [`faccessat`] mode: test for execute (or directory search) permission.
pub const X_OK: i32 = 1;
/// [`faccessat`] mode: test for write permission.
pub const W_OK: i32 = 2;
/// [`faccessat`] mode: test for read permission.
pub const R_OK: i32 = 4;

/// Check the calling process's permissions for the file at `path` relative to
/// `dirfd`. `mode` is [`F_OK`] or an OR of [`R_OK`]/[`W_OK`]/[`X_OK`]; returns
/// `Ok(())` if all requested accesses are allowed, else the `Errno` (e.g.
/// `EACCES`, `ENOENT`).
///
/// The check uses the **real** uid/gid (matching the bare `faccessat` syscall),
/// which is exactly what a shell wants when testing PATH candidates for
/// executability.
pub fn faccessat(dirfd: i32, path: &CStr, mode: i32) -> Result<(), Errno> {
    // SAFETY: `path` is a valid nul-terminated C string the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::FACCESSAT,
            dirfd as usize,
            path.as_ptr() as usize,
            mode as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// [`faccessat`] against the current working directory ([`AT_FDCWD`]).
#[inline]
pub fn access(path: &CStr, mode: i32) -> Result<(), Errno> {
    faccessat(AT_FDCWD, path, mode)
}

// --- statx(2) -----------------------------------------------------------------

/// `mask` bit set: request the basic `stat`-equivalent fields (mode, size,
/// timestamps, ids, links). The common choice for [`statx`].
pub const STATX_BASIC_STATS: u32 = 0x0000_07ff;

/// `stx_mode` type mask; AND with it and compare to an `S_IF*` value.
pub const S_IFMT: u16 = 0o170000;
/// `stx_mode` type: named pipe (FIFO).
pub const S_IFIFO: u16 = 0o010000;
/// `stx_mode` type: character device.
pub const S_IFCHR: u16 = 0o020000;
/// `stx_mode` type: directory.
pub const S_IFDIR: u16 = 0o040000;
/// `stx_mode` type: block device.
pub const S_IFBLK: u16 = 0o060000;
/// `stx_mode` type: regular file.
pub const S_IFREG: u16 = 0o100000;
/// `stx_mode` type: symbolic link.
pub const S_IFLNK: u16 = 0o120000;
/// `stx_mode` type: socket.
pub const S_IFSOCK: u16 = 0o140000;

/// A `statx` timestamp (kernel `struct statx_timestamp`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatxTimestamp {
    /// Seconds since the Unix epoch.
    pub tv_sec: i64,
    /// Nanoseconds within the second.
    pub tv_nsec: u32,
    __reserved: i32,
}

/// File metadata (kernel `struct statx`, 256 bytes). Only the fields requested
/// in the `statx` `mask` and reported back in [`Statx::stx_mask`] are valid.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statx {
    /// Bitmask of which fields the kernel actually filled in.
    pub stx_mask: u32,
    /// Preferred I/O block size.
    pub stx_blksize: u32,
    /// Extra file attribute flags (`STATX_ATTR_*`).
    pub stx_attributes: u64,
    /// Number of hard links.
    pub stx_nlink: u32,
    /// Owner user ID.
    pub stx_uid: u32,
    /// Owner group ID.
    pub stx_gid: u32,
    /// File type and mode bits (test with [`S_IFMT`]/`S_IF*`).
    pub stx_mode: u16,
    __spare0: u16,
    /// Inode number.
    pub stx_ino: u64,
    /// Size in bytes (for a regular file, its length).
    pub stx_size: u64,
    /// Number of 512-byte blocks allocated.
    pub stx_blocks: u64,
    /// Which `stx_attributes` bits are supported/meaningful.
    pub stx_attributes_mask: u64,
    /// Last access time.
    pub stx_atime: StatxTimestamp,
    /// Creation (birth) time, if supported.
    pub stx_btime: StatxTimestamp,
    /// Last status-change time.
    pub stx_ctime: StatxTimestamp,
    /// Last modification time.
    pub stx_mtime: StatxTimestamp,
    /// Device major of the file this refers to (for device files).
    pub stx_rdev_major: u32,
    /// Device minor of the file this refers to (for device files).
    pub stx_rdev_minor: u32,
    /// Major of the device that holds the file.
    pub stx_dev_major: u32,
    /// Minor of the device that holds the file.
    pub stx_dev_minor: u32,
    /// Mount ID of the mount holding the file.
    pub stx_mnt_id: u64,
    /// Memory-alignment requirement for direct I/O, if any.
    pub stx_dio_mem_align: u32,
    /// Offset-alignment requirement for direct I/O, if any.
    pub stx_dio_offset_align: u32,
    __spare3: [u64; 12],
}

const _: () = assert!(core::mem::size_of::<StatxTimestamp>() == 16);
const _: () = assert!(core::mem::size_of::<Statx>() == 256);
const _: () = assert!(core::mem::offset_of!(Statx, stx_mode) == 28);
const _: () = assert!(core::mem::offset_of!(Statx, stx_ino) == 32);
const _: () = assert!(core::mem::offset_of!(Statx, stx_size) == 40);
const _: () = assert!(core::mem::offset_of!(Statx, stx_mtime) == 112);

impl Statx {
    /// The file-type portion of [`stx_mode`](Self::stx_mode).
    #[inline]
    pub const fn file_type(self) -> u16 {
        self.stx_mode & S_IFMT
    }
    /// True if this is a regular file.
    #[inline]
    pub const fn is_file(self) -> bool {
        self.file_type() == S_IFREG
    }
    /// True if this is a directory.
    #[inline]
    pub const fn is_dir(self) -> bool {
        self.file_type() == S_IFDIR
    }
    /// True if this is a symbolic link (only meaningful under
    /// [`AT_SYMLINK_NOFOLLOW`]/[`lstat`]).
    #[inline]
    pub const fn is_symlink(self) -> bool {
        self.file_type() == S_IFLNK
    }
}

/// Fetch metadata for `path` relative to `dirfd`. `flags` accepts
/// [`AT_SYMLINK_NOFOLLOW`]/[`AT_EMPTY_PATH`]; `mask` selects the fields to
/// retrieve (usually [`STATX_BASIC_STATS`]). See [`stat`]/[`lstat`]/[`fstat`]
/// for the common shorthands.
pub fn statx(dirfd: i32, path: &CStr, flags: i32, mask: u32) -> Result<Statx, Errno> {
    let mut buf = Statx::default();
    // statx(dirfd, path, flags, mask, &mut buf).
    // SAFETY: `path` is a valid C string; `buf` is a valid, exclusively
    // borrowed `struct statx` the kernel writes.
    let ret = unsafe {
        syscall5(
            nr::STATX,
            dirfd as usize,
            path.as_ptr() as usize,
            flags as usize,
            mask as usize,
            &mut buf as *mut Statx as usize,
        )
    };
    from_ret(ret)?;
    Ok(buf)
}

/// Metadata for `path`, following symlinks (like `stat(2)`).
#[inline]
pub fn stat(path: &CStr) -> Result<Statx, Errno> {
    statx(AT_FDCWD, path, 0, STATX_BASIC_STATS)
}

/// Metadata for `path` **without** following a terminal symlink (like
/// `lstat(2)`); [`Statx::is_symlink`] is meaningful here.
#[inline]
pub fn lstat(path: &CStr) -> Result<Statx, Errno> {
    statx(AT_FDCWD, path, AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS)
}

/// Metadata for an already-open descriptor (like `fstat(2)`), via
/// [`AT_EMPTY_PATH`].
#[inline]
pub fn fstat(fd: i32) -> Result<Statx, Errno> {
    statx(fd, c"", AT_EMPTY_PATH, STATX_BASIC_STATS)
}

// --- path mutations -----------------------------------------------------------

/// Remove the link at `path` relative to `dirfd`. `flags` may be `0` (unlink a
/// file) or [`AT_REMOVEDIR`] (remove an empty directory).
pub fn unlinkat(dirfd: i32, path: &CStr, flags: i32) -> Result<(), Errno> {
    // SAFETY: `path` is a valid C string the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::UNLINKAT,
            dirfd as usize,
            path.as_ptr() as usize,
            flags as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Delete the file `path` (relative to the cwd). Shorthand for [`unlinkat`].
#[inline]
pub fn unlink(path: &CStr) -> Result<(), Errno> {
    unlinkat(AT_FDCWD, path, 0)
}

/// Remove the empty directory `path`. Shorthand for [`unlinkat`] with
/// [`AT_REMOVEDIR`].
#[inline]
pub fn rmdir(path: &CStr) -> Result<(), Errno> {
    unlinkat(AT_FDCWD, path, AT_REMOVEDIR)
}

/// Create a directory `path` relative to `dirfd` with permission bits `mode`
/// (masked by the process umask).
pub fn mkdirat(dirfd: i32, path: &CStr, mode: u32) -> Result<(), Errno> {
    // SAFETY: `path` is a valid C string the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::MKDIRAT,
            dirfd as usize,
            path.as_ptr() as usize,
            mode as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Create the directory `path` (relative to the cwd). Shorthand for
/// [`mkdirat`].
#[inline]
pub fn mkdir(path: &CStr, mode: u32) -> Result<(), Errno> {
    mkdirat(AT_FDCWD, path, mode)
}

/// [`renameat2`] flag: fail with `EEXIST` if `new` already exists.
pub const RENAME_NOREPLACE: u32 = 1;
/// [`renameat2`] flag: atomically exchange `old` and `new` (both must exist).
pub const RENAME_EXCHANGE: u32 = 2;

/// Rename `old` (relative to `olddirfd`) to `new` (relative to `newdirfd`),
/// with `RENAME_*` `flags` (`0` for plain rename).
pub fn renameat2(
    olddirfd: i32,
    old: &CStr,
    newdirfd: i32,
    new: &CStr,
    flags: u32,
) -> Result<(), Errno> {
    // SAFETY: both paths are valid C strings the kernel only reads.
    let ret = unsafe {
        syscall5(
            nr::RENAMEAT2,
            olddirfd as usize,
            old.as_ptr() as usize,
            newdirfd as usize,
            new.as_ptr() as usize,
            flags as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Rename `old` to `new` (both relative to the cwd). Shorthand for
/// [`renameat2`] with no flags.
#[inline]
pub fn rename(old: &CStr, new: &CStr) -> Result<(), Errno> {
    renameat2(AT_FDCWD, old, AT_FDCWD, new, 0)
}

/// Create a symbolic link at `linkpath` (relative to `newdirfd`) pointing at
/// `target` (stored verbatim, not resolved).
pub fn symlinkat(target: &CStr, newdirfd: i32, linkpath: &CStr) -> Result<(), Errno> {
    // SAFETY: both paths are valid C strings the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::SYMLINKAT,
            target.as_ptr() as usize,
            newdirfd as usize,
            linkpath.as_ptr() as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Create the symlink `linkpath` -> `target` (relative to the cwd).
#[inline]
pub fn symlink(target: &CStr, linkpath: &CStr) -> Result<(), Errno> {
    symlinkat(target, AT_FDCWD, linkpath)
}

/// [`linkat`] flag: if `oldpath` is a symlink, hard-link the file it points
/// to rather than the symlink itself (the raw syscall's default, without this
/// flag, is to hard-link the symlink itself).
pub const AT_SYMLINK_FOLLOW: i32 = 0x400;

/// Create a hard link at `newpath` (relative to `newdirfd`) for the existing
/// file `oldpath` (relative to `olddirfd`). `flags` accepts
/// [`AT_SYMLINK_FOLLOW`] (dereference `oldpath` if it is a symlink) or `0`.
pub fn linkat(
    olddirfd: i32,
    oldpath: &CStr,
    newdirfd: i32,
    newpath: &CStr,
    flags: i32,
) -> Result<(), Errno> {
    // SAFETY: both paths are valid C strings the kernel only reads.
    let ret = unsafe {
        syscall5(
            nr::LINKAT,
            olddirfd as usize,
            oldpath.as_ptr() as usize,
            newdirfd as usize,
            newpath.as_ptr() as usize,
            flags as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Create the hard link `newpath` -> `oldpath` (both relative to the cwd),
/// without following a symlinked `oldpath`. Shorthand for [`linkat`].
#[inline]
pub fn link(oldpath: &CStr, newpath: &CStr) -> Result<(), Errno> {
    linkat(AT_FDCWD, oldpath, AT_FDCWD, newpath, 0)
}

/// Read the target of the symlink at `path` (relative to `dirfd`) into `buf`,
/// returning the target bytes actually read.
///
/// `readlink` does **not** nul-terminate, and it truncates silently if `buf`
/// is too small — a returned length equal to `buf.len()` means the target may
/// have been longer, so retry with a bigger buffer.
pub fn readlinkat<'a>(dirfd: i32, path: &CStr, buf: &'a mut [u8]) -> Result<&'a [u8], Errno> {
    // SAFETY: `path` is a valid C string; `buf` is a valid, exclusively
    // borrowed slice the kernel writes at most `buf.len()` bytes into.
    let ret = unsafe {
        syscall4(
            nr::READLINKAT,
            dirfd as usize,
            path.as_ptr() as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
        )
    };
    let n = from_ret(ret)?;
    Ok(&buf[..n])
}

/// Read the target of the symlink `path` (relative to the cwd). Shorthand for
/// [`readlinkat`].
#[inline]
pub fn readlink<'a>(path: &CStr, buf: &'a mut [u8]) -> Result<&'a [u8], Errno> {
    readlinkat(AT_FDCWD, path, buf)
}

// --- fchmodat(2) / fchownat(2) -------------------------------------------

/// Sentinel for [`fchownat`]'s `uid`/`gid`: leave that id unchanged. The
/// kernel treats `-1` (reinterpreted as `u32::MAX`) this way for both fields
/// independently, so `chown(path, new_uid, DONT_CHANGE)` changes only the
/// owner.
pub const DONT_CHANGE: u32 = u32::MAX;

/// Change the permission bits of the file at `path` relative to `dirfd` to
/// `mode` (masked by neither the process umask nor anything else — this sets
/// the mode bits exactly, unlike [`crate::fd::open`]'s `mode` argument).
///
/// Unlike [`fchownat`], the raw kernel `fchmodat` syscall takes **no**
/// `flags` argument — there is no way to change a symlink's own permission
/// bits through it (symlink permissions are meaningless on Linux and the
/// kernel ignores them regardless), so this always follows a terminal
/// symlink.
pub fn fchmodat(dirfd: i32, path: &CStr, mode: u32) -> Result<(), Errno> {
    // SAFETY: `path` is a valid nul-terminated C string the kernel only reads.
    let ret = unsafe {
        syscall3(
            nr::FCHMODAT,
            dirfd as usize,
            path.as_ptr() as usize,
            mode as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Change the permission bits of `path` (relative to the cwd) to `mode`.
/// Shorthand for [`fchmodat`].
#[inline]
pub fn chmod(path: &CStr, mode: u32) -> Result<(), Errno> {
    fchmodat(AT_FDCWD, path, mode)
}

/// Resize the file at `path` to exactly `length` bytes: extends it with
/// zero bytes if it was shorter, discards data past `length` if it was
/// longer. Path-based sibling of [`crate::fd::ftruncate`] (fd-based); there
/// is no `truncateat` -- open the file and use `ftruncate` if a `dirfd`-
/// relative path is needed.
pub fn truncate(path: &CStr, length: i64) -> Result<(), Errno> {
    // On 64-bit Linux `truncate` takes a 64-bit `off_t` directly.
    // SAFETY: `path` is a valid nul-terminated C string the kernel only reads.
    let ret = unsafe { syscall2(nr::TRUNCATE, path.as_ptr() as usize, length as usize) };
    from_ret(ret).map(|_| ())
}

/// Change the owner/group of the file at `path` relative to `dirfd`. `flags`
/// accepts [`AT_SYMLINK_NOFOLLOW`] (operate on a symlink itself rather than
/// its target) or `0`. Pass [`DONT_CHANGE`] for `uid` or `gid` to leave that
/// id as-is.
pub fn fchownat(dirfd: i32, path: &CStr, uid: u32, gid: u32, flags: i32) -> Result<(), Errno> {
    // SAFETY: `path` is a valid nul-terminated C string the kernel only reads.
    let ret = unsafe {
        syscall5(
            nr::FCHOWNAT,
            dirfd as usize,
            path.as_ptr() as usize,
            uid as usize,
            gid as usize,
            flags as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Change the owner/group of `path` (relative to the cwd), following a
/// terminal symlink. Shorthand for [`fchownat`] with `flags = 0`.
#[inline]
pub fn chown(path: &CStr, uid: u32, gid: u32) -> Result<(), Errno> {
    fchownat(AT_FDCWD, path, uid, gid, 0)
}

/// Change the owner/group of the symlink `path` itself (relative to the
/// cwd), **not** the file it points to. Shorthand for [`fchownat`] with
/// [`AT_SYMLINK_NOFOLLOW`].
#[inline]
pub fn lchown(path: &CStr, uid: u32, gid: u32) -> Result<(), Errno> {
    fchownat(AT_FDCWD, path, uid, gid, AT_SYMLINK_NOFOLLOW)
}

// --- utimensat(2) ---------------------------------------------------------

/// Sentinel for a [`crate::time::Timespec`] passed to [`utimensat`]/[`utimens`]: set this
/// timestamp to the current time (`CLOCK_REALTIME` at the moment the syscall
/// runs), ignoring [`crate::time::Timespec::tv_sec`].
pub const UTIME_NOW: i64 = (1 << 30) - 1;
/// Sentinel for a [`crate::time::Timespec`] passed to [`utimensat`]/[`utimens`]: leave
/// this timestamp unchanged.
pub const UTIME_OMIT: i64 = (1 << 30) - 2;

/// Set the access and modification times of the file at `path` relative to
/// `dirfd`. `times` is `[atime, mtime]`; pass `None` to set both to the
/// current time (equivalent to `[UTIME_NOW, UTIME_NOW]`), or build the array
/// yourself using [`UTIME_NOW`]/[`UTIME_OMIT`] as either timestamp's
/// `tv_nsec` to set-to-now or leave-unchanged that one independently.
/// `flags` accepts [`AT_SYMLINK_NOFOLLOW`] to touch a symlink itself.
pub fn utimensat(
    dirfd: i32,
    path: &CStr,
    times: Option<[crate::time::Timespec; 2]>,
    flags: i32,
) -> Result<(), Errno> {
    let ptr = match &times {
        Some(t) => t.as_ptr() as usize,
        None => 0,
    };
    // SAFETY: `path` is a valid nul-terminated C string the kernel only
    // reads; `times` is either null (meaning "set both to now") or a valid
    // 2-element `timespec` array the kernel only reads.
    let ret = unsafe {
        syscall4(
            nr::UTIMENSAT,
            dirfd as usize,
            path.as_ptr() as usize,
            ptr,
            flags as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Set the access/modification times of `path` (relative to the cwd),
/// following a terminal symlink. Shorthand for [`utimensat`] with
/// `flags = 0`.
#[inline]
pub fn utimens(path: &CStr, times: Option<[crate::time::Timespec; 2]>) -> Result<(), Errno> {
    utimensat(AT_FDCWD, path, times, 0)
}

// --- getdents64(2) --------------------------------------------------------

/// [`RawDirent::d_type`] tag: the filesystem didn't return a type cheaply —
/// a caller needing to know it unconditionally must `stat`/`lstat`.
pub const DT_UNKNOWN: u8 = 0;
/// [`RawDirent::d_type`] tag: a regular file.
pub const DT_REG: u8 = 8;
/// [`RawDirent::d_type`] tag: a directory.
pub const DT_DIR: u8 = 4;
/// [`RawDirent::d_type`] tag: a symbolic link.
pub const DT_LNK: u8 = 10;

/// `getdents64(2)`, Track P: fill `buf` with as many packed
/// `linux_dirent64` records as fit, returning the byte count written (`0`
/// at end of directory). Parse the filled region with [`dirents`].
///
/// Unlike glibc's `readdir`/`DIR*`, there is no per-entry allocation or
/// hidden internal buffering here — `buf` *is* the buffer, reused across
/// calls by the caller, matching this crate's other bring-your-own-buffer
/// calls ([`crate::fd::read`]).
pub fn getdents64(fd: i32, buf: &mut [u8]) -> Result<usize, Errno> {
    // SAFETY: `buf` is a valid, exclusively borrowed region of `buf.len()`
    // bytes the kernel packs `linux_dirent64` records into, never writing
    // past `buf.len()`; `fd` is a caller-owned directory descriptor.
    let ret = unsafe {
        syscall3(
            nr::GETDENTS64,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
        )
    };
    from_ret(ret)
}

/// One directory entry parsed out of a [`getdents64`]-filled buffer by
/// [`dirents`] — borrows directly from it, no allocation.
#[derive(Debug, Clone, Copy)]
pub struct RawDirent<'a> {
    /// The entry's inode number.
    pub d_ino: u64,
    /// A [`DT_REG`]/[`DT_DIR`]/[`DT_LNK`]/… tag, or [`DT_UNKNOWN`].
    pub d_type: u8,
    /// The entry's bare name — not NUL-terminated, and `.`/`..` are not
    /// filtered out (the kernel includes them like any other entry; a
    /// `readdir`-emulating caller filters them the way it already must).
    pub d_name: &'a [u8],
}

/// Iterator over a [`getdents64`]-filled buffer, yielding [`RawDirent`].
/// Constructed by [`dirents`].
pub struct Dirents<'a> {
    buf: &'a [u8],
    pos: usize,
}

/// Parse the `n` bytes [`getdents64`] wrote into `buf[..n]` as a sequence
/// of [`RawDirent`]s. Pass exactly the filled prefix (the byte count
/// `getdents64` returned) — the kernel's own `d_reclen` chain governs
/// iteration, so a self-consistent buffer parses correctly regardless of
/// how many records it holds.
///
/// # Panics
///
/// Panics on a buffer that isn't a genuine `getdents64` fill (truncated
/// mid-record, or a corrupt `d_reclen`) — a programmer error, not a
/// runtime condition: `buf` is kernel-produced, never external/untrusted
/// data, so there is nothing to recover from gracefully here.
pub fn dirents(buf: &[u8]) -> Dirents<'_> {
    Dirents { buf, pos: 0 }
}

impl<'a> Iterator for Dirents<'a> {
    type Item = RawDirent<'a>;

    fn next(&mut self) -> Option<RawDirent<'a>> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let rec = &self.buf[self.pos..];
        // Kernel `struct linux_dirent64` layout: d_ino (u64) @0, d_off
        // (i64, unused here) @8, d_reclen (u16) @16, d_type (u8) @18,
        // then the NUL-terminated d_name, padded to d_reclen.
        let d_ino = u64::from_ne_bytes(rec[0..8].try_into().unwrap());
        let d_reclen = u16::from_ne_bytes(rec[16..18].try_into().unwrap()) as usize;
        let d_type = rec[18];
        let name_region = &rec[19..d_reclen];
        let nul = name_region
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_region.len());
        let d_name = &name_region[..nul];
        self.pos += d_reclen;
        Some(RawDirent {
            d_ino,
            d_type,
            d_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fd;

    fn temp_path(tag: &str) -> std::ffi::CString {
        let p = format!(
            "{}/rusty_libc_fs_{}_{}",
            std::env::temp_dir().display(),
            tag,
            std::process::id()
        );
        std::ffi::CString::new(p).unwrap()
    }

    #[test]
    fn access_checks_existence_and_exec() {
        // /bin/sh exists and is executable; a bogus path does not exist.
        assert!(access(c"/bin/sh", F_OK).is_ok());
        assert!(access(c"/bin/sh", X_OK).is_ok());
        assert_eq!(
            access(c"/no/such/rusty_libc/path", F_OK),
            Err(Errno::ENOENT)
        );
    }

    #[test]
    fn statx_reports_type_and_size() {
        // A directory.
        let d = stat(c"/").expect("stat /");
        assert!(d.is_dir());
        assert!(!d.is_file());

        // A regular file we create with known contents.
        let path = temp_path("statx");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::write_all(f, b"12345").expect("write");

        // fstat via AT_EMPTY_PATH sees the size before we close.
        let via_fd = fstat(f).expect("fstat");
        assert!(via_fd.is_file());
        assert_eq!(via_fd.stx_size, 5);
        fd::close(f).expect("close");

        // stat by path agrees.
        let via_path = stat(&path).expect("stat file");
        assert!(via_path.is_file());
        assert_eq!(via_path.stx_size, 5);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn symlink_lstat_readlink_unlink() {
        let link = temp_path("link");
        // Point it at a target that need not exist; symlink stores it verbatim.
        symlink(c"/target/of/link", &link).expect("symlink");

        // lstat sees the link itself.
        let st = lstat(&link).expect("lstat");
        assert!(st.is_symlink());

        // readlink returns the stored target (no NUL).
        let mut buf = [0u8; 256];
        let target = readlink(&link, &mut buf).expect("readlink");
        assert_eq!(target, b"/target/of/link");

        unlink(&link).expect("unlink link");
        assert_eq!(lstat(&link), Err(Errno::ENOENT));
    }

    #[test]
    fn link_creates_a_second_name_for_the_same_inode() {
        let original = temp_path("link_src");
        let hardlink = temp_path("link_dst");
        let _ = unlink(&hardlink);

        let f = fd::open(&original, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::write_all(f, b"shared content").expect("write_all");
        fd::close(f).expect("close");

        link(&original, &hardlink).expect("link");

        // Same inode, and the kernel now reports 2 links to it.
        let a = stat(&original).expect("stat original");
        let b = stat(&hardlink).expect("stat hardlink");
        assert_eq!(a.stx_ino, b.stx_ino);
        assert_eq!(b.stx_nlink, 2);

        // Content is visible through either name (it's the same file, not a copy).
        let f = fd::open(&hardlink, fd::O_RDONLY, 0).expect("open hardlink");
        let mut buf = [0u8; 32];
        let n = fd::read(f, &mut buf).expect("read");
        assert_eq!(&buf[..n], b"shared content");
        fd::close(f).expect("close");

        // Removing one name leaves the other (and the data) intact.
        unlink(&original).expect("unlink original");
        assert!(stat(&hardlink)
            .expect("stat hardlink after unlink")
            .is_file());

        unlink(&hardlink).expect("unlink hardlink");
    }

    #[test]
    fn linkat_symlink_follow_flag_controls_dereferencing() {
        let target = temp_path("linkat_target");
        let symlink_path = temp_path("linkat_symlink");
        let via_symlink_itself = temp_path("linkat_of_symlink");
        let via_dereferenced = temp_path("linkat_of_target");
        for p in [
            &target,
            &symlink_path,
            &via_symlink_itself,
            &via_dereferenced,
        ] {
            let _ = unlink(p);
        }

        let f = fd::open(&target, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600)
            .expect("open target");
        fd::close(f).expect("close");
        symlink(&target, &symlink_path).expect("symlink");

        // Without AT_SYMLINK_FOLLOW: hard-links the symlink itself.
        linkat(AT_FDCWD, &symlink_path, AT_FDCWD, &via_symlink_itself, 0)
            .expect("linkat no-follow");
        assert!(lstat(&via_symlink_itself).expect("lstat").is_symlink());

        // With AT_SYMLINK_FOLLOW: hard-links the regular file it points to.
        linkat(
            AT_FDCWD,
            &symlink_path,
            AT_FDCWD,
            &via_dereferenced,
            AT_SYMLINK_FOLLOW,
        )
        .expect("linkat follow");
        assert!(lstat(&via_dereferenced).expect("lstat").is_file());
        assert_eq!(
            stat(&via_dereferenced).expect("stat").stx_ino,
            stat(&target).expect("stat target").stx_ino
        );

        for p in [
            &target,
            &symlink_path,
            &via_symlink_itself,
            &via_dereferenced,
        ] {
            unlink(p).expect("unlink");
        }
    }

    #[test]
    fn link_missing_source_is_enoent() {
        assert_eq!(
            link(
                c"/no/such/rusty_libc/path",
                c"/tmp/rusty_libc_unused_target"
            ),
            Err(Errno::ENOENT)
        );
    }

    #[test]
    fn mkdir_rename_rmdir() {
        let a = temp_path("dir_a");
        let b = temp_path("dir_b");
        // Clean any leftovers from a previous aborted run.
        let _ = rmdir(&a);
        let _ = rmdir(&b);

        mkdir(&a, 0o700).expect("mkdir a");
        assert!(stat(&a).expect("stat a").is_dir());

        rename(&a, &b).expect("rename a -> b");
        assert_eq!(stat(&a), Err(Errno::ENOENT));
        assert!(stat(&b).expect("stat b").is_dir());

        rmdir(&b).expect("rmdir b");
        assert_eq!(stat(&b), Err(Errno::ENOENT));
    }

    #[test]
    fn chmod_changes_permission_bits() {
        let path = temp_path("chmod");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o644).expect("open");
        fd::close(f).expect("close");

        chmod(&path, 0o600).expect("chmod 600");
        assert_eq!(stat(&path).expect("stat").stx_mode as u32 & 0o777, 0o600);

        chmod(&path, 0o755).expect("chmod 755");
        assert_eq!(stat(&path).expect("stat").stx_mode as u32 & 0o777, 0o755);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn chown_noop_leaves_ownership_unchanged() {
        let path = temp_path("chown_noop");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::close(f).expect("close");

        let before = stat(&path).expect("stat before");
        // DONT_CHANGE for both fields is always permitted, even unprivileged
        // -- nothing is actually being altered, so this exercises the
        // sentinel without needing CAP_CHOWN.
        chown(&path, DONT_CHANGE, DONT_CHANGE).expect("chown noop");
        let after = stat(&path).expect("stat after");
        assert_eq!(after.stx_uid, before.stx_uid);
        assert_eq!(after.stx_gid, before.stx_gid);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn chown_and_lchown_change_ownership_when_privileged() {
        // Changing the owner id requires CAP_CHOWN. Always issue the
        // syscalls (so a regression in argument order/flags still shows up
        // as an unexpected error), but only assert the resulting metadata
        // when running privileged -- this crate's own test suite runs as
        // root in this environment, but a consumer's CI might not.
        let path = temp_path("chown_priv");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::close(f).expect("close");

        let target_uid = crate::process::getuid().wrapping_add(1);
        match chown(&path, target_uid, DONT_CHANGE) {
            Ok(()) => assert_eq!(stat(&path).expect("stat").stx_uid, target_uid),
            Err(Errno::EPERM) => {} // unprivileged: expected, not a crate bug
            Err(e) => panic!("unexpected chown error: {e:?}"),
        }

        let link = temp_path("lchown_priv");
        let _ = unlink(&link);
        // The target need not exist: lchown (AT_SYMLINK_NOFOLLOW) touches the
        // link itself, never resolving it.
        symlink(c"/does/not/need/to/exist", &link).expect("symlink");
        match lchown(&link, target_uid, DONT_CHANGE) {
            Ok(()) => assert_eq!(lstat(&link).expect("lstat").stx_uid, target_uid),
            Err(Errno::EPERM) => {}
            Err(e) => panic!("unexpected lchown error: {e:?}"),
        }

        unlink(&path).expect("unlink");
        unlink(&link).expect("unlink link");
    }

    #[test]
    fn chmod_missing_path_is_enoent() {
        assert_eq!(
            chmod(c"/no/such/rusty_libc/path", 0o600),
            Err(Errno::ENOENT)
        );
    }

    #[test]
    fn truncate_shrinks_and_grows_a_path() {
        let path = temp_path("truncate");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::write_all(f, b"0123456789").expect("write");
        fd::close(f).expect("close");
        assert_eq!(stat(&path).expect("stat").stx_size, 10);

        truncate(&path, 4).expect("truncate shrink");
        assert_eq!(stat(&path).expect("stat").stx_size, 4);

        truncate(&path, 20).expect("truncate grow");
        assert_eq!(stat(&path).expect("stat").stx_size, 20);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn truncate_missing_path_is_enoent() {
        assert_eq!(truncate(c"/no/such/rusty_libc/path", 0), Err(Errno::ENOENT));
    }

    #[test]
    fn utimens_sets_explicit_atime_and_mtime() {
        use crate::time::Timespec;

        let path = temp_path("utimens_explicit");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::close(f).expect("close");

        // An arbitrary, exact past timestamp -- easy to tell apart from "now".
        let atime = Timespec {
            tv_sec: 1_000_000,
            tv_nsec: 123_000,
        };
        let mtime = Timespec {
            tv_sec: 2_000_000,
            tv_nsec: 456_000,
        };
        utimens(&path, Some([atime, mtime])).expect("utimens explicit");

        let st = stat(&path).expect("stat");
        assert_eq!(st.stx_atime.tv_sec, atime.tv_sec);
        assert_eq!(st.stx_atime.tv_nsec as i64, atime.tv_nsec);
        assert_eq!(st.stx_mtime.tv_sec, mtime.tv_sec);
        assert_eq!(st.stx_mtime.tv_nsec as i64, mtime.tv_nsec);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn utimens_none_sets_both_to_now() {
        let path = temp_path("utimens_now");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::close(f).expect("close");

        // Back-date it first so "now" is unambiguously later.
        let old = crate::time::Timespec {
            tv_sec: 1_000_000,
            tv_nsec: 0,
        };
        utimens(&path, Some([old, old])).expect("utimens backdate");

        let before = crate::time::clock_gettime(crate::time::CLOCK_REALTIME).expect("clock");
        utimens(&path, None).expect("utimens now");
        let after = crate::time::clock_gettime(crate::time::CLOCK_REALTIME).expect("clock");

        let st = stat(&path).expect("stat");
        assert!(st.stx_mtime.tv_sec >= before.tv_sec && st.stx_mtime.tv_sec <= after.tv_sec + 1);
        assert!(st.stx_atime.tv_sec >= before.tv_sec && st.stx_atime.tv_sec <= after.tv_sec + 1);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn utimens_omit_leaves_one_timestamp_unchanged() {
        use crate::time::Timespec;

        let path = temp_path("utimens_omit");
        let f = fd::open(&path, fd::O_WRONLY | fd::O_CREAT | fd::O_TRUNC, 0o600).expect("open");
        fd::close(f).expect("close");

        let baseline = Timespec {
            tv_sec: 1_500_000,
            tv_nsec: 0,
        };
        utimens(&path, Some([baseline, baseline])).expect("utimens baseline");

        // Change only mtime; OMIT on atime must leave it exactly as set above.
        let new_mtime = Timespec {
            tv_sec: 3_000_000,
            tv_nsec: 0,
        };
        let omit = Timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        };
        utimens(&path, Some([omit, new_mtime])).expect("utimens omit atime");

        let st = stat(&path).expect("stat");
        assert_eq!(
            st.stx_atime.tv_sec, baseline.tv_sec,
            "atime must be omitted"
        );
        assert_eq!(st.stx_mtime.tv_sec, new_mtime.tv_sec);

        unlink(&path).expect("unlink");
    }

    #[test]
    fn utimensat_nofollow_touches_the_symlink_not_its_target() {
        use crate::time::Timespec;

        let link = temp_path("utimens_symlink");
        let _ = unlink(&link);
        // Target need not exist: AT_SYMLINK_NOFOLLOW never resolves it.
        symlink(c"/does/not/need/to/exist", &link).expect("symlink");

        let mtime = Timespec {
            tv_sec: 1_234_567,
            tv_nsec: 0,
        };
        utimensat(AT_FDCWD, &link, Some([mtime, mtime]), AT_SYMLINK_NOFOLLOW)
            .expect("utimensat nofollow");

        let st = lstat(&link).expect("lstat");
        assert_eq!(st.stx_mtime.tv_sec, mtime.tv_sec);

        unlink(&link).expect("unlink link");
    }

    #[test]
    fn utimens_missing_path_is_enoent() {
        assert_eq!(
            utimens(c"/no/such/rusty_libc/path", None),
            Err(Errno::ENOENT)
        );
    }

    #[test]
    fn getdents64_lists_created_entries_with_types() {
        let dir = temp_path("getdents");
        let _ = rmdir(&dir);
        mkdir(&dir, 0o700).expect("mkdir");

        let dirfd = fd::open(&dir, fd::O_RDONLY | fd::O_DIRECTORY, 0).expect("open dir");
        let file =
            fd::openat(dirfd, c"a_file", fd::O_WRONLY | fd::O_CREAT, 0o600).expect("create a_file");
        fd::close(file).expect("close a_file");
        mkdirat(dirfd, c"a_subdir", 0o700).expect("create a_subdir");

        let mut names_and_types = std::collections::BTreeMap::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = getdents64(dirfd, &mut buf).expect("getdents64");
            if n == 0 {
                break;
            }
            for entry in dirents(&buf[..n]) {
                let name = String::from_utf8_lossy(entry.d_name).into_owned();
                names_and_types.insert(name, entry.d_type);
            }
        }
        fd::close(dirfd).expect("close dirfd");

        // `.`/`..` are present (unfiltered, by design) alongside the two
        // created entries with their real kernel-reported types.
        assert!(names_and_types.contains_key("."));
        assert!(names_and_types.contains_key(".."));
        assert_eq!(names_and_types.get("a_file"), Some(&DT_REG));
        assert_eq!(names_and_types.get("a_subdir"), Some(&DT_DIR));

        let _ = std::fs::remove_dir_all(dir.to_str().expect("utf8 path"));
    }
}
