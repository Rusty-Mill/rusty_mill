//! Memory mapping: `mmap`(2)/`munmap`(2)/`mprotect`(2), plus the rest of the
//! family -- `madvise`(2)/`mlock`(2)/`munlock`(2)/`msync`(2).
//!
//! No caller in `rush` uses this today; it exists as a primitive for future
//! callers (a memory-mapped history file, or a large-buffer alternative to a
//! read loop) rather than a confirmed gap. See REVIEW.md Round 4, item 48.

use crate::arch::nr;
use crate::arch::{from_ret, syscall3, syscall6, Errno};

/// `mmap`/`mprotect` protection: pages may be read.
pub const PROT_READ: i32 = 0x1;
/// `mmap`/`mprotect` protection: pages may be written.
pub const PROT_WRITE: i32 = 0x2;
/// `mmap`/`mprotect` protection: pages may be executed.
pub const PROT_EXEC: i32 = 0x4;
/// `mmap`/`mprotect` protection: pages may not be accessed at all.
pub const PROT_NONE: i32 = 0x0;

/// `mmap` flag: changes are shared with other mappings of the same
/// file/region (and written back to the backing file, if any).
pub const MAP_SHARED: i32 = 0x01;
/// `mmap` flag: changes are private to this mapping (copy-on-write); never
/// written back to the backing file.
pub const MAP_PRIVATE: i32 = 0x02;
/// `mmap` flag: `addr` is not a hint -- map at exactly that address (or fail).
pub const MAP_FIXED: i32 = 0x10;
/// `mmap` flag: the mapping has no backing file; `fd` must be `-1` and
/// `offset` `0`. Combine with [`MAP_PRIVATE`] for a zero-initialized
/// scratch buffer.
pub const MAP_ANONYMOUS: i32 = 0x20;

/// [`madvise`] advice: no special treatment (the default).
pub const MADV_NORMAL: i32 = 0;
/// [`madvise`] advice: expect page references in random order.
pub const MADV_RANDOM: i32 = 1;
/// [`madvise`] advice: expect page references in sequential order.
pub const MADV_SEQUENTIAL: i32 = 2;
/// [`madvise`] advice: expect to access these pages soon -- a hint to start
/// readahead/prefetch.
pub const MADV_WILLNEED: i32 = 3;
/// [`madvise`] advice: these pages are not needed -- the kernel may discard
/// them (private pages) or write them back and drop them (shared/file
/// pages) at its discretion, freeing the underlying physical memory.
pub const MADV_DONTNEED: i32 = 4;
/// [`madvise`] advice: these pages may be freed if the system is under
/// memory pressure; unlike [`MADV_DONTNEED`], the kernel decides lazily
/// rather than reclaiming immediately, and the content survives until it
/// actually does.
pub const MADV_FREE: i32 = 8;

/// [`msync`] flag: schedule the sync but don't wait for it to complete.
pub const MS_ASYNC: i32 = 1;
/// [`msync`] flag: invalidate other mappings of the same file, so they see
/// the just-synced data on their next access.
pub const MS_INVALIDATE: i32 = 2;
/// [`msync`] flag: block until the sync completes.
pub const MS_SYNC: i32 = 4;

/// Map `length` bytes starting at `offset` in `fd` (or an anonymous, unbacked
/// region when `flags` includes [`MAP_ANONYMOUS`]) into the calling
/// process's address space, returning the mapping's base address.
///
/// `addr` is a placement hint; pass `0` to let the kernel choose unless
/// `flags` includes [`MAP_FIXED`], in which case it is mandatory. `length`
/// and `offset` are rounded/must already be page-aligned per the kernel's
/// usual rules.
///
/// # Safety
///
/// The returned region is raw, uninitialized (or file-backed) memory outside
/// Rust's ownership model: the caller must not let it outlive an [`munmap`]
/// call, must not construct overlapping mappings that alias a `&mut`
/// elsewhere, and must respect `prot` (e.g. never write through a
/// `PROT_READ`-only mapping).
pub unsafe fn mmap(
    addr: usize,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    offset: i64,
) -> Result<*mut u8, Errno> {
    // mmap(addr, length, prot, flags, fd, offset).
    // SAFETY: forwarded to the caller's own safety contract; this wrapper
    // performs no dereference itself.
    let ret = unsafe {
        syscall6(
            nr::MMAP,
            addr,
            length,
            prot as usize,
            flags as usize,
            fd as usize,
            offset as usize,
        )
    };
    from_ret(ret).map(|p| p as *mut u8)
}

/// Unmap the `length`-byte region starting at `addr`, previously returned by
/// [`mmap`].
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns; every pointer into the unmapped range becomes dangling the instant
/// this returns `Ok`.
pub unsafe fn munmap(addr: *mut u8, length: usize) -> Result<(), Errno> {
    // munmap(addr, length).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MUNMAP, addr as usize, length, 0) };
    from_ret(ret).map(|_| ())
}

/// Change the protection of the `length`-byte region starting at `addr`
/// (a live mapping from [`mmap`]) to `prot`.
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns; narrowing `prot` (e.g. dropping [`PROT_WRITE`]) invalidates any
/// `&mut` borrow a caller may be holding into that range.
pub unsafe fn mprotect(addr: *mut u8, length: usize, prot: i32) -> Result<(), Errno> {
    // mprotect(addr, length, prot).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MPROTECT, addr as usize, length, prot as usize) };
    from_ret(ret).map(|_| ())
}

/// Advise the kernel on expected access patterns for the `length`-byte
/// region starting at `addr` (`advice` is an `MADV_*` constant) -- a hint,
/// not a guarantee; the kernel may ignore it.
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns. [`MADV_DONTNEED`]/[`MADV_FREE`] in particular can make the
/// region's contents (or the whole region, for private/anonymous mappings)
/// unreliable to read afterward -- only use them on memory the caller is
/// truly done with.
pub unsafe fn madvise(addr: *mut u8, length: usize, advice: i32) -> Result<(), Errno> {
    // madvise(addr, length, advice).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MADVISE, addr as usize, length, advice as usize) };
    from_ret(ret).map(|_| ())
}

/// Lock the `length`-byte region starting at `addr` into physical memory,
/// preventing it from being swapped out -- e.g. to keep a buffer holding a
/// password or key from ever being written to disk.
///
/// Bounded by `RLIMIT_MEMLOCK` for an unprivileged caller (see
/// [`crate::rlimit`]); a privileged caller (`CAP_IPC_LOCK`) is unbounded.
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns.
pub unsafe fn mlock(addr: *mut u8, length: usize) -> Result<(), Errno> {
    // mlock(addr, length).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MLOCK, addr as usize, length, 0) };
    from_ret(ret).map(|_| ())
}

/// Undo a previous [`mlock`] on the `length`-byte region starting at `addr`,
/// allowing it to be swapped out again.
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns.
pub unsafe fn munlock(addr: *mut u8, length: usize) -> Result<(), Errno> {
    // munlock(addr, length).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MUNLOCK, addr as usize, length, 0) };
    from_ret(ret).map(|_| ())
}

/// Flush changes made to a file-backed mapping's `length`-byte region
/// starting at `addr` back to the underlying file (`flags` is
/// [`MS_ASYNC`]/[`MS_SYNC`], optionally OR'd with [`MS_INVALIDATE`]). A
/// no-op on a purely anonymous mapping -- there is no backing file to
/// flush to.
///
/// # Safety
///
/// `addr`/`length` must describe (a subset of) a live mapping this process
/// owns.
pub unsafe fn msync(addr: *mut u8, length: usize, flags: i32) -> Result<(), Errno> {
    // msync(addr, length, flags).
    // SAFETY: forwarded to the caller's own safety contract.
    let ret = unsafe { syscall3(nr::MSYNC, addr as usize, length, flags as usize) };
    from_ret(ret).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: usize = 4096;

    #[test]
    fn anonymous_mapping_reads_zeroed_and_is_writable() {
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            assert!(!p.is_null());

            let slice = core::slice::from_raw_parts(p, PAGE);
            assert!(slice.iter().all(|&b| b == 0));

            *p = 0xab;
            assert_eq!(*p, 0xab);

            munmap(p, PAGE).expect("munmap");
        }
    }

    #[test]
    fn mprotect_read_only_then_restore() {
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            *p = 1;

            mprotect(p, PAGE, PROT_READ).expect("mprotect read-only");
            // Reading through the now-read-only mapping is still fine.
            assert_eq!(*p, 1);

            mprotect(p, PAGE, PROT_READ | PROT_WRITE).expect("mprotect restore");
            *p = 2;
            assert_eq!(*p, 2);

            munmap(p, PAGE).expect("munmap");
        }
    }

    #[test]
    fn mmap_bad_fd_without_anonymous_is_ebadf() {
        unsafe {
            assert_eq!(
                mmap(0, PAGE, PROT_READ, MAP_PRIVATE, -1, 0),
                Err(Errno::EBADF)
            );
        }
    }

    #[test]
    fn munmap_bad_length_is_einval() {
        unsafe {
            assert_eq!(
                munmap(core::ptr::dangling_mut::<u8>(), 0),
                Err(Errno::EINVAL)
            );
        }
    }

    #[test]
    fn mprotect_bad_addr_is_enomem() {
        unsafe {
            // A page-aligned address with no mapping behind it: create one,
            // free it, and reuse the (still page-aligned) address -- an
            // unaligned address would fail with EINVAL before the kernel
            // ever checks whether anything is mapped there.
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            munmap(p, PAGE).expect("munmap");

            assert_eq!(mprotect(p, PAGE, PROT_READ), Err(Errno::ENOMEM));
        }
    }

    #[test]
    fn madvise_dontneed_and_willneed_succeed_on_anonymous_mapping() {
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            *p = 0xab;

            madvise(p, PAGE, MADV_WILLNEED).expect("madvise willneed");
            madvise(p, PAGE, MADV_DONTNEED).expect("madvise dontneed");
            // After MADV_DONTNEED on a private anonymous mapping, the page
            // is conceptually reset: touching it again must not fault, and
            // reads zeroed content (same contract as a fresh mapping).
            assert_eq!(*p, 0);

            munmap(p, PAGE).expect("munmap");
        }
    }

    #[test]
    fn madvise_on_an_unmapped_range_does_not_crash() {
        // Unlike mmap/mprotect/msync (which reliably ENOMEM on a hole,
        // exercised in their own bad-addr tests), madvise's handling of an
        // unmapped range is not consistently ENOMEM across kernels/advice
        // values -- verified empirically to differ between this x86_64 host
        // (ENOMEM) and the aarch64 qemu-user environment (Ok) for the same
        // MADV_NORMAL call over the same kind of hole. Only assert the call
        // doesn't panic/UB; don't assume which outcome a given target gives.
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            munmap(p, PAGE).expect("munmap");

            match madvise(p, PAGE, MADV_NORMAL) {
                Ok(()) | Err(Errno::ENOMEM) => {}
                Err(e) => panic!("unexpected madvise error: {e:?}"),
            }
        }
    }

    #[test]
    fn mlock_and_munlock_round_trip_when_permitted() {
        // mlock needs CAP_IPC_LOCK, or to fit within RLIMIT_MEMLOCK when
        // unprivileged. Always issue the syscall (so a regression in
        // argument order/flags still shows up as an unexpected error), but
        // only require success when it actually happens -- this crate's own
        // test suite runs as root in this environment, but a consumer's CI
        // (or a default RLIMIT_MEMLOCK too small for even one page) might
        // not permit it.
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");

            match mlock(p, PAGE) {
                Ok(()) => munlock(p, PAGE).expect("munlock"),
                Err(Errno::EPERM) | Err(Errno::ENOMEM) => {} // unprivileged: expected, not a crate bug
                Err(e) => panic!("unexpected mlock error: {e:?}"),
            }

            munmap(p, PAGE).expect("munmap");
        }
    }

    #[test]
    fn msync_flushes_a_shared_file_backed_mapping() {
        use crate::fd;
        unsafe {
            let f = fd::memfd_create(c"rusty_libc_msync", 0).expect("memfd_create");
            fd::ftruncate(f, PAGE as i64).expect("ftruncate");

            let p = mmap(0, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, f, 0).expect("mmap");
            *p = 0x42;

            msync(p, PAGE, MS_SYNC).expect("msync");

            // A MAP_SHARED mapping's writes are visible to the backing fd
            // directly (msync's guarantee is about durability/cross-mapping
            // coherency, not same-process visibility) -- confirm the write
            // actually landed in the file.
            let mut buf = [0u8; 1];
            fd::pread(f, &mut buf, 0).expect("pread");
            assert_eq!(buf[0], 0x42);

            munmap(p, PAGE).expect("munmap");
            fd::close(f).expect("close");
        }
    }

    #[test]
    fn msync_bad_addr_is_enomem() {
        unsafe {
            let p = mmap(
                0,
                PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .expect("mmap");
            munmap(p, PAGE).expect("munmap");

            assert_eq!(msync(p, PAGE, MS_SYNC), Err(Errno::ENOMEM));
        }
    }
}
