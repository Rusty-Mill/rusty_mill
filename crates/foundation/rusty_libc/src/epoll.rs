//! `epoll`: a scalable successor to [`crate::fd::poll`] for many-fd event
//! loops (a job-control shell tracking many background jobs' pipes/pidfds
//! at once), where `poll`'s O(n)-per-call fd scan starts to matter.
//!
//! `struct epoll_event`'s kernel layout genuinely differs by architecture:
//! x86_64 packs it (`__attribute__((packed))`, kept for 32-bit-compat
//! reasons per the kernel's own `EPOLL_PACKED` macro in
//! `include/uapi/linux/eventpoll.h`) for a 12-byte struct with `data` at
//! offset 4, while aarch64 uses natural alignment for a 16-byte struct with
//! `data` at offset 8. [`EpollEvent`]'s fields are private for exactly this
//! reason -- on x86_64 `data` sits at an unaligned offset, so touching it
//! through an ordinary `&`-reference (which `pub` fields would invite) is
//! rejected by the compiler; the accessor methods read the value out
//! instead, which is sound on both layouts.

use crate::arch::nr;
#[cfg(target_arch = "aarch64")]
use crate::arch::syscall6;
use crate::arch::{from_ret, from_ret_i32, syscall1, syscall4, Errno};

/// `epoll_ctl(2)` operation: register `fd` for the given events.
pub const EPOLL_CTL_ADD: i32 = 1;
/// `epoll_ctl(2)` operation: deregister `fd`.
pub const EPOLL_CTL_DEL: i32 = 2;
/// `epoll_ctl(2)` operation: change the registered events for `fd`.
pub const EPOLL_CTL_MOD: i32 = 3;

/// Event/interest flag: ready to read.
pub const EPOLLIN: u32 = 0x001;
/// Event/interest flag: ready to write.
pub const EPOLLOUT: u32 = 0x004;
/// Return-only flag: an error condition occurred.
pub const EPOLLERR: u32 = 0x008;
/// Return-only flag: the peer hung up (e.g. the pipe's writer closed).
pub const EPOLLHUP: u32 = 0x010;
/// Interest flag: edge-triggered mode -- report readiness only on a state
/// *change*, not on every call while still ready (the default, level-
/// triggered, matches `poll`'s own behavior and is almost always what a
/// caller migrating from `poll` wants).
pub const EPOLLET: u32 = 1 << 31;

/// A single epoll interest/event entry (kernel `struct epoll_event`). See
/// the module docs for why its fields are private rather than `pub`.
#[cfg(target_arch = "x86_64")]
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EpollEvent {
    events: u32,
    data: u64,
}

#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::size_of::<EpollEvent>() == 12);
#[cfg(target_arch = "x86_64")]
const _: () = assert!(core::mem::offset_of!(EpollEvent, data) == 4);

/// A single epoll interest/event entry (kernel `struct epoll_event`). See
/// the module docs for why its fields are private rather than `pub`.
#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EpollEvent {
    events: u32,
    data: u64,
}

#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::size_of::<EpollEvent>() == 16);
#[cfg(target_arch = "aarch64")]
const _: () = assert!(core::mem::offset_of!(EpollEvent, data) == 8);

impl EpollEvent {
    /// Build an event: `events` is an OR of `EPOLL*` constants, `data` is an
    /// opaque value handed back unchanged by [`epoll_wait`] (commonly the
    /// fd itself, so the caller can tell which fd a returned event is for
    /// without a separate lookup).
    pub fn new(events: u32, data: u64) -> Self {
        EpollEvent { events, data }
    }

    /// The `EPOLL*` flags this entry carries (the requested interest for
    /// [`epoll_ctl`], or the fired events after [`epoll_wait`]).
    pub fn events(&self) -> u32 {
        self.events
    }

    /// The opaque value passed to [`EpollEvent::new`].
    pub fn data(&self) -> u64 {
        self.data
    }
}

/// Create a new epoll instance, returning its fd. `flags` is `0` or
/// [`crate::fd::O_CLOEXEC`].
pub fn epoll_create1(flags: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer argument.
    let ret = unsafe { syscall1(nr::EPOLL_CREATE1, flags as usize) };
    from_ret_i32(ret)
}

/// Add/modify/remove `fd`'s registration on epoll instance `epfd` (`op` is
/// [`EPOLL_CTL_ADD`]/[`EPOLL_CTL_MOD`]/[`EPOLL_CTL_DEL`]). `event` gives the
/// interest flags and opaque data; required for `ADD`/`MOD`, ignored (may
/// be `None`) for `DEL`.
pub fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: Option<&EpollEvent>) -> Result<(), Errno> {
    let event_ptr = match event {
        Some(e) => e as *const EpollEvent as usize,
        None => 0,
    };
    // epoll_ctl(epfd, op, fd, event).
    // SAFETY: `event`, when present, is a valid `*const epoll_event` the
    // kernel only reads.
    let ret = unsafe {
        syscall4(
            nr::EPOLL_CTL,
            epfd as usize,
            op as usize,
            fd as usize,
            event_ptr,
        )
    };
    from_ret(ret).map(|_| ())
}

/// Wait up to `timeout_ms` (negative blocks indefinitely, `0` polls without
/// blocking) for a registered fd on `epfd` to become ready, filling `events`
/// with up to `events.len()` ready entries and returning how many fired.
///
/// x86_64 uses `epoll_wait` directly; aarch64 has no bare `epoll_wait`
/// syscall, so this issues `epoll_pwait` with a null signal mask (the same
/// `poll`-vs-`ppoll` substitution [`crate::fd::poll`] already makes).
#[cfg(target_arch = "x86_64")]
pub fn epoll_wait(epfd: i32, events: &mut [EpollEvent], timeout_ms: i32) -> Result<usize, Errno> {
    // SAFETY: `events` is a valid, exclusively-borrowed slice of
    // `events.len()` entries; the kernel writes at most that many.
    let ret = unsafe {
        syscall4(
            nr::EPOLL_WAIT,
            epfd as usize,
            events.as_mut_ptr() as usize,
            events.len(),
            timeout_ms as usize,
        )
    };
    from_ret(ret)
}

/// aarch64 `epoll_pwait`-backed [`epoll_wait`]; see the x86_64 variant for
/// docs.
#[cfg(target_arch = "aarch64")]
pub fn epoll_wait(epfd: i32, events: &mut [EpollEvent], timeout_ms: i32) -> Result<usize, Errno> {
    // epoll_pwait(epfd, events, maxevents, timeout, sigmask = NULL,
    // sigsetsize = 8). The kernel ignores sigsetsize when sigmask is null,
    // but pass the canonical 8, matching fd::poll's own ppoll call.
    // SAFETY: `events` is a valid, exclusively-borrowed slice of
    // `events.len()` entries; the kernel writes at most that many; the
    // signal mask is null.
    let ret = unsafe {
        syscall6(
            nr::EPOLL_PWAIT,
            epfd as usize,
            events.as_mut_ptr() as usize,
            events.len(),
            timeout_ms as usize,
            0,
            8,
        )
    };
    from_ret(ret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fd;

    #[test]
    fn epoll_wait_reports_a_readable_pipe() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");

        epoll_ctl(
            epfd,
            EPOLL_CTL_ADD,
            r,
            Some(&EpollEvent::new(EPOLLIN, r as u64)),
        )
        .expect("epoll_ctl add");

        fd::write(w, b"x").expect("write");

        let mut events = [EpollEvent::new(0, 0); 4];
        let n = epoll_wait(epfd, &mut events, 1000).expect("epoll_wait");
        assert_eq!(n, 1);
        assert_eq!(events[0].data(), r as u64);
        assert_ne!(events[0].events() & EPOLLIN, 0);

        fd::close(r).ok();
        fd::close(w).ok();
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_wait_times_out_with_no_ready_fds() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");
        epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0))).expect("epoll_ctl");

        let mut events = [EpollEvent::new(0, 0); 4];
        let n = epoll_wait(epfd, &mut events, 50).expect("epoll_wait");
        assert_eq!(n, 0);

        fd::close(r).ok();
        fd::close(w).ok();
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_wait_reports_hup_when_writer_closes() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");
        epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0))).expect("epoll_ctl");

        fd::close(w).expect("close w");

        let mut events = [EpollEvent::new(0, 0); 4];
        let n = epoll_wait(epfd, &mut events, 1000).expect("epoll_wait");
        assert_eq!(n, 1);
        assert_ne!(events[0].events() & EPOLLHUP, 0);

        fd::close(r).ok();
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_ctl_mod_changes_the_registered_interest() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");
        epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0)))
            .expect("epoll_ctl add");

        // Switch interest to EPOLLOUT (meaningless on a pipe's read end, but
        // exercises MOD): writing to the pipe should no longer wake it up
        // for the (now-uninterested) read side.
        epoll_ctl(epfd, EPOLL_CTL_MOD, r, Some(&EpollEvent::new(EPOLLOUT, 0)))
            .expect("epoll_ctl mod");
        fd::write(w, b"x").expect("write");

        let mut events = [EpollEvent::new(0, 0); 4];
        let n = epoll_wait(epfd, &mut events, 50).expect("epoll_wait");
        assert_eq!(n, 0, "read end should no longer be watched for EPOLLIN");

        fd::close(r).ok();
        fd::close(w).ok();
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_ctl_del_deregisters_the_fd() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");
        epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0)))
            .expect("epoll_ctl add");
        epoll_ctl(epfd, EPOLL_CTL_DEL, r, None).expect("epoll_ctl del");

        fd::write(w, b"x").expect("write");

        let mut events = [EpollEvent::new(0, 0); 4];
        let n = epoll_wait(epfd, &mut events, 50).expect("epoll_wait");
        assert_eq!(n, 0);

        fd::close(r).ok();
        fd::close(w).ok();
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_ctl_bad_fd_is_ebadf() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        assert_eq!(
            epoll_ctl(epfd, EPOLL_CTL_ADD, -1, Some(&EpollEvent::new(EPOLLIN, 0))),
            Err(Errno::EBADF)
        );
        fd::close(epfd).ok();
    }

    #[test]
    fn epoll_ctl_duplicate_add_is_eexist() {
        let epfd = epoll_create1(0).expect("epoll_create1");
        let (r, w) = fd::pipe2(0).expect("pipe2");
        epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0)))
            .expect("epoll_ctl add");

        assert_eq!(
            epoll_ctl(epfd, EPOLL_CTL_ADD, r, Some(&EpollEvent::new(EPOLLIN, 0))),
            Err(Errno::EEXIST)
        );

        fd::close(r).ok();
        fd::close(w).ok();
        fd::close(epfd).ok();
    }
}
