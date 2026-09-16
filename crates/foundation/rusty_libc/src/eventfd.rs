//! `eventfd(2)`: a kernel-maintained 64-bit counter exposed as a pollable fd.
//!
//! The third of this crate's `*fd` event-notification primitives, alongside
//! [`crate::signal::signalfd`] and [`crate::time::timerfd_create`] -- a
//! simple counter an event loop can `poll`/write/read for cross-thread or
//! self-pipe-style wakeups, without needing an actual pipe pair.
//!
//! There is no dedicated `eventfd_read`/`eventfd_write` syscall: the
//! resulting fd is driven with the ordinary [`crate::fd::read`]/
//! [`crate::fd::write`] primitives already in this crate, each moving
//! exactly 8 bytes (a little-endian `u64`) at a time. Writing adds the
//! given value to the kernel's counter (blocking, unless [`EFD_NONBLOCK`],
//! if the addition would overflow `u64::MAX`); reading either returns the
//! whole accumulated value and resets the counter to `0` (the default), or
//! -- with [`EFD_SEMAPHORE`] -- returns `1` and decrements the counter by
//! `1`, so long as it isn't already `0`.

use crate::arch::nr;
use crate::arch::{from_ret_i32, syscall2, Errno};

/// `eventfd(2)` flag: treat the fd as a semaphore -- each read returns `1`
/// and decrements the counter by exactly `1`, instead of returning (and
/// resetting) the whole accumulated value.
pub const EFD_SEMAPHORE: i32 = 1 << 0;
/// `eventfd(2)` flag: set close-on-exec on the returned descriptor.
pub const EFD_CLOEXEC: i32 = 0o2000000;
/// `eventfd(2)` flag: non-blocking reads/writes on the returned descriptor.
pub const EFD_NONBLOCK: i32 = 0o0004000;

/// Create an eventfd with its counter initialized to `initval`, returning
/// the new fd. `flags` is an OR of [`EFD_SEMAPHORE`]/[`EFD_CLOEXEC`]/
/// [`EFD_NONBLOCK`], or `0`.
pub fn eventfd(initval: u32, flags: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer arguments.
    let ret = unsafe { syscall2(nr::EVENTFD2, initval as usize, flags as usize) };
    from_ret_i32(ret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fd;

    #[test]
    fn write_then_read_roundtrips_and_accumulates() {
        let efd = eventfd(0, 0).expect("eventfd");

        fd::write(efd, &5u64.to_ne_bytes()).expect("write 5");
        fd::write(efd, &3u64.to_ne_bytes()).expect("write 3");

        let mut buf = [0u8; 8];
        let n = fd::read(efd, &mut buf).expect("read");
        assert_eq!(n, 8);
        // Writes to an eventfd counter add; a single read returns the
        // accumulated total and resets the counter to 0.
        assert_eq!(u64::from_ne_bytes(buf), 8);

        fd::close(efd).ok();
    }

    #[test]
    fn initval_seeds_the_counter() {
        let efd = eventfd(42, 0).expect("eventfd");

        let mut buf = [0u8; 8];
        fd::read(efd, &mut buf).expect("read");
        assert_eq!(u64::from_ne_bytes(buf), 42);

        fd::close(efd).ok();
    }

    #[test]
    fn efd_semaphore_reads_return_one_and_decrement() {
        let efd = eventfd(0, EFD_SEMAPHORE).expect("eventfd");
        fd::write(efd, &3u64.to_ne_bytes()).expect("write 3");

        let mut buf = [0u8; 8];
        for _ in 0..3 {
            let n = fd::read(efd, &mut buf).expect("read");
            assert_eq!(n, 8);
            assert_eq!(u64::from_ne_bytes(buf), 1);
        }

        // Counter is back to 0: a non-blocking read now returns EAGAIN
        // instead of blocking forever.
        fd::close(efd).ok();
        let efd = eventfd(0, EFD_SEMAPHORE | EFD_NONBLOCK).expect("eventfd nonblock");
        assert_eq!(fd::read(efd, &mut buf), Err(Errno::EAGAIN));

        fd::close(efd).ok();
    }

    #[test]
    fn nonblocking_read_on_a_zero_counter_is_eagain() {
        let efd = eventfd(0, EFD_NONBLOCK).expect("eventfd");
        let mut buf = [0u8; 8];
        assert_eq!(fd::read(efd, &mut buf), Err(Errno::EAGAIN));
        fd::close(efd).ok();
    }

    #[test]
    fn short_write_buffer_is_einval() {
        let efd = eventfd(0, 0).expect("eventfd");
        // eventfd requires exactly 8 bytes per write.
        assert_eq!(fd::write(efd, &[1, 2, 3]), Err(Errno::EINVAL));
        fd::close(efd).ok();
    }
}
