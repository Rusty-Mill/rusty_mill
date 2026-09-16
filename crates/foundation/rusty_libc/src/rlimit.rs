//! Resource limits via `prlimit64` (used for both get and set).
//!
//! Note: there is **no** `RLIMIT_*` for pipe capacity — a pipe's buffer size is
//! a per-fd property set with `fcntl(fd, F_SETPIPE_SZ, size)` (see
//! [`crate::fd::F_SETPIPE_SZ`]), not a resource limit, so it lives in `fd`, not
//! here.

use crate::arch::nr;
use crate::arch::{from_ret, syscall4, Errno};

/// "No limit" sentinel for a [`Rlimit`] field.
pub const RLIM_INFINITY: u64 = u64::MAX;

// RLIMIT_* resource identifiers (asm-generic).
/// CPU time, in seconds.
pub const RLIMIT_CPU: i32 = 0;
/// Maximum file size.
pub const RLIMIT_FSIZE: i32 = 1;
/// Maximum data-segment size.
pub const RLIMIT_DATA: i32 = 2;
/// Maximum stack size.
pub const RLIMIT_STACK: i32 = 3;
/// Maximum core-dump size.
pub const RLIMIT_CORE: i32 = 4;
/// Maximum resident set size.
pub const RLIMIT_RSS: i32 = 5;
/// Maximum number of processes.
pub const RLIMIT_NPROC: i32 = 6;
/// Maximum number of open files.
pub const RLIMIT_NOFILE: i32 = 7;
/// Maximum lockable memory.
pub const RLIMIT_MEMLOCK: i32 = 8;
/// Maximum address-space size.
pub const RLIMIT_AS: i32 = 9;
/// Maximum number of file locks.
pub const RLIMIT_LOCKS: i32 = 10;
/// Maximum number of pending signals.
pub const RLIMIT_SIGPENDING: i32 = 11;
/// Maximum bytes in POSIX message queues.
pub const RLIMIT_MSGQUEUE: i32 = 12;
/// Ceiling on the process's nice value.
pub const RLIMIT_NICE: i32 = 13;
/// Ceiling on the real-time priority.
pub const RLIMIT_RTPRIO: i32 = 14;
/// Ceiling on real-time CPU time consumed without a blocking syscall, in
/// microseconds.
pub const RLIMIT_RTTIME: i32 = 15;

/// A soft/hard resource-limit pair (kernel `struct rlimit64`; both fields are
/// always `u64`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rlimit {
    /// Soft limit (the value the kernel enforces).
    pub cur: u64,
    /// Hard limit (ceiling on the soft limit; only privileged callers raise
    /// it).
    pub max: u64,
}

const _: () = assert!(core::mem::size_of::<Rlimit>() == 16);

/// Get and/or set the limit for `resource` on process `pid` in one call, the
/// full `prlimit64` primitive.
///
/// `pid == 0` targets the calling process. When `new` is `Some`, the limit is
/// set to it; when `old` is `Some`, the previous limit is written there. Both
/// may be supplied to atomically swap. Setting another process's limit needs
/// the appropriate privilege (`CAP_SYS_RESOURCE`); [`getrlimit`]/[`setrlimit`]
/// are the common `pid == 0` shorthands.
pub fn prlimit(
    pid: i32,
    resource: i32,
    new: Option<&Rlimit>,
    old: Option<&mut Rlimit>,
) -> Result<(), Errno> {
    let new_ptr = match new {
        Some(n) => n as *const Rlimit as usize,
        None => 0,
    };
    let old_ptr = match old {
        Some(o) => o as *mut Rlimit as usize,
        None => 0,
    };
    // prlimit64(pid, resource, new, old).
    // SAFETY: both pointers are either null or valid `rlimit64`s — `new` read
    // only, `old` written only.
    let ret = unsafe {
        syscall4(
            nr::PRLIMIT64,
            pid as usize,
            resource as usize,
            new_ptr,
            old_ptr,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Get the current soft/hard limit for `resource` (a `RLIMIT_*` constant) of
/// the calling process. Shorthand for [`prlimit`] with `pid = 0`.
#[inline]
pub fn getrlimit(resource: i32) -> Result<Rlimit, Errno> {
    let mut old = Rlimit { cur: 0, max: 0 };
    prlimit(0, resource, None, Some(&mut old))?;
    Ok(old)
}

/// Set the soft/hard limit for `resource` on the calling process to `limit`.
/// Shorthand for [`prlimit`] with `pid = 0`.
#[inline]
pub fn setrlimit(resource: i32, limit: &Rlimit) -> Result<(), Errno> {
    prlimit(0, resource, Some(limit), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_restore_nofile() {
        let original = getrlimit(RLIMIT_NOFILE).expect("getrlimit");

        // Lower the soft limit (always permitted up to the hard limit).
        let lowered = Rlimit {
            cur: original.cur.min(64),
            max: original.max,
        };
        setrlimit(RLIMIT_NOFILE, &lowered).expect("setrlimit lower");
        assert_eq!(getrlimit(RLIMIT_NOFILE).unwrap().cur, lowered.cur);

        // Restore.
        setrlimit(RLIMIT_NOFILE, &original).expect("setrlimit restore");
        assert_eq!(getrlimit(RLIMIT_NOFILE).unwrap(), original);
    }

    #[test]
    fn bad_resource_is_einval() {
        assert_eq!(getrlimit(9999), Err(Errno::EINVAL));
    }

    #[test]
    fn prlimit_reads_and_swaps() {
        // Read-only (new = None) matches getrlimit.
        let mut cur = Rlimit { cur: 0, max: 0 };
        prlimit(0, RLIMIT_NOFILE, None, Some(&mut cur)).expect("prlimit read");
        assert_eq!(cur, getrlimit(RLIMIT_NOFILE).unwrap());

        // Atomic swap: set a lowered soft limit while reading the old value.
        let lowered = Rlimit {
            cur: cur.cur.min(64),
            max: cur.max,
        };
        let mut prev = Rlimit { cur: 0, max: 0 };
        prlimit(0, RLIMIT_NOFILE, Some(&lowered), Some(&mut prev)).expect("prlimit swap");
        assert_eq!(prev, cur);
        assert_eq!(getrlimit(RLIMIT_NOFILE).unwrap().cur, lowered.cur);

        // Restore (set-only, old = None).
        prlimit(0, RLIMIT_NOFILE, Some(&cur), None).expect("prlimit restore");
        assert_eq!(getrlimit(RLIMIT_NOFILE).unwrap(), cur);
    }
}
