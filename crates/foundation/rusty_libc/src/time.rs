//! Clocks and sleeping: `clock_gettime`, `nanosleep`, and `timerfd_*`.

use crate::arch::nr;
use crate::arch::{from_ret, from_ret_i32, syscall2, syscall4, Errno};

/// A time value with nanosecond resolution (kernel `struct timespec`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    /// Whole seconds.
    pub tv_sec: i64,
    /// Nanoseconds within the second (`0..1_000_000_000`).
    pub tv_nsec: i64,
}

const _: () = assert!(core::mem::size_of::<Timespec>() == 16);

/// Wall-clock time since the Unix epoch (settable; can jump).
pub const CLOCK_REALTIME: i32 = 0;
/// Monotonic time since an unspecified start; never jumps backward. The right
/// clock for measuring elapsed intervals (timeouts, `time`-ing a command).
pub const CLOCK_MONOTONIC: i32 = 1;
/// Monotonic time that also advances while the system is suspended (unlike
/// [`CLOCK_MONOTONIC`]).
pub const CLOCK_BOOTTIME: i32 = 7;
/// A faster, lower-resolution [`CLOCK_REALTIME`]: on the vDSO fast path this
/// skips the fine-grained interpolation step, at the cost of coarser precision
/// (typically 1–4 ms, one kernel timer tick). Good for a prompt timestamp or
/// any read where sub-millisecond accuracy is not needed — it is roughly 2–3x
/// faster than the precise clock (see `bench/`).
pub const CLOCK_REALTIME_COARSE: i32 = 5;
/// A faster, lower-resolution [`CLOCK_MONOTONIC`]; see [`CLOCK_REALTIME_COARSE`]
/// for the precision/speed trade-off. The right choice for `$SECONDS`-style
/// counters, throttling, or any hot-path timestamp that only needs
/// millisecond-ish accuracy.
pub const CLOCK_MONOTONIC_COARSE: i32 = 6;

impl Timespec {
    /// Construct from a whole number of milliseconds.
    #[inline]
    pub const fn from_millis(ms: i64) -> Self {
        Timespec {
            tv_sec: ms / 1000,
            tv_nsec: (ms % 1000) * 1_000_000,
        }
    }
}

/// Read the current value of clock `clockid` (a `CLOCK_*` constant).
///
/// When the process vDSO exports a usable `clock_gettime`, this reads the clock
/// entirely in userspace (no syscall trap), matching glibc's speed; otherwise
/// it falls back to the raw syscall. The result is identical either way.
#[inline]
pub fn clock_gettime(clockid: i32) -> Result<Timespec, Errno> {
    // Fast path: the vDSO reads the clock without entering the kernel. Left
    // uninitialized until written: both the vDSO entry and the syscall below
    // fill it completely on success, and neither path reads it first.
    let mut ts = core::mem::MaybeUninit::<Timespec>::uninit();

    // A non-zero return means the vDSO declined (e.g. an unsupported clock),
    // so we fall through to the syscall, which reproduces success or the error.
    if let Some(f) = crate::vdso::clock_gettime_fn() {
        // SAFETY: `f` is the resolved vDSO entry with exactly this ABI; `ts` is
        // a valid, exclusively-borrowed, suitably-sized-and-aligned
        // `timespec` it writes on success.
        if unsafe { f(clockid, ts.as_mut_ptr()) } == 0 {
            // SAFETY: a zero return means the vDSO fully initialized `ts`.
            return Ok(unsafe { ts.assume_init() });
        }
    }

    // clock_gettime(clockid, &mut ts).
    // SAFETY: `ts` is a valid, exclusively-borrowed, suitably-sized-and-aligned
    // `struct timespec`; the kernel writes it completely on success and this
    // crate's `no_std` syscall path never reads it before checking `from_ret`.
    let ret = unsafe {
        syscall2(
            nr::CLOCK_GETTIME,
            clockid as usize,
            ts.as_mut_ptr() as usize,
        )
    };
    from_ret(ret)?;
    // SAFETY: `from_ret` returned `Ok`, so the kernel fully initialized `ts`.
    Ok(unsafe { ts.assume_init() })
}

/// Query clock `clockid`'s resolution (the smallest interval it can
/// represent) -- for most clocks on Linux this is a fixed `1` nanosecond
/// regardless of the hardware's actual precision, since the kernel doesn't
/// track a per-clock granularity beyond that; still real syscall data, not
/// a constant this crate could hardcode.
pub fn clock_getres(clockid: i32) -> Result<Timespec, Errno> {
    let mut ts = core::mem::MaybeUninit::<Timespec>::uninit();
    // clock_getres(clockid, &mut ts).
    // SAFETY: `ts` is a valid, exclusively-borrowed, suitably-sized-and-aligned
    // `struct timespec`; the kernel writes it completely on success.
    let ret = unsafe { syscall2(nr::CLOCK_GETRES, clockid as usize, ts.as_mut_ptr() as usize) };
    from_ret(ret)?;
    // SAFETY: `from_ret` returned `Ok`, so the kernel fully initialized `ts`.
    Ok(unsafe { ts.assume_init() })
}

/// Suspend execution for at least the interval `req`.
///
/// If a signal interrupts the sleep, this returns `Err(EINTR)`; when `rem` is
/// `Some`, the time left unslept is written there so the caller can resume.
pub fn nanosleep(req: &Timespec, rem: Option<&mut Timespec>) -> Result<(), Errno> {
    let rem_ptr = match rem {
        Some(r) => r as *mut Timespec as usize,
        None => 0,
    };
    // nanosleep(req, rem).
    // SAFETY: `req` is a valid `*const timespec` the kernel reads; `rem` is null
    // or a valid `*mut timespec` the kernel writes on EINTR.
    let ret = unsafe { syscall2(nr::NANOSLEEP, req as *const Timespec as usize, rem_ptr) };
    from_ret(ret).map(|_| ())
}

/// [`clock_nanosleep`] `flags`: interpret `req` as an absolute deadline on
/// `clockid` rather than an interval relative to now.
pub const TIMER_ABSTIME: i32 = 1;

/// Suspend execution until `req` elapses, on the clock named by `clockid`
/// (typically [`CLOCK_MONOTONIC`] or [`CLOCK_REALTIME`]) — unlike
/// [`nanosleep`], which always sleeps relative to an unspecified,
/// roughly-monotonic clock with no way to name it or target an absolute
/// deadline.
///
/// With [`TIMER_ABSTIME`], `req` is the absolute time to wake at rather
/// than a relative duration, giving a drift-free periodic sleep: compute
/// each deadline as `last_deadline + period` up front, rather than
/// `now + period` after each wake (which accumulates the scheduling
/// latency of every previous iteration). The same reasoning that makes
/// [`crate::time::timerfd_create`] preferable to `SIGALRM` for a repeating
/// timeout applies here to a thread that wants to block for it directly.
///
/// If a signal interrupts the sleep, this returns `Err(EINTR)`; when
/// `rem` is `Some` and `flags` does *not* include [`TIMER_ABSTIME`], the
/// time left unslept is written there so the caller can resume (as with
/// `nanosleep`) — the kernel never touches `rem` for an absolute sleep,
/// since "time remaining" isn't a meaningful concept for a fixed deadline.
pub fn clock_nanosleep(
    clockid: i32,
    flags: i32,
    req: &Timespec,
    rem: Option<&mut Timespec>,
) -> Result<(), Errno> {
    let rem_ptr = match rem {
        Some(r) => r as *mut Timespec as usize,
        None => 0,
    };
    // clock_nanosleep(clockid, flags, req, rem).
    // SAFETY: `req` is a valid `*const timespec` the kernel reads; `rem` is
    // null or a valid `*mut timespec` the kernel may write on EINTR.
    let ret = unsafe {
        syscall4(
            nr::CLOCK_NANOSLEEP,
            clockid as usize,
            flags as usize,
            req as *const Timespec as usize,
            rem_ptr,
        )
    };
    from_ret(ret).map(|_| ())
}

// --- timerfd_create(2) / timerfd_settime(2) / timerfd_gettime(2) --------

/// `timerfd_create(2)`/`timerfd_settime(2)` flag: return/set a non-blocking
/// descriptor.
pub const TFD_NONBLOCK: i32 = 0o0004000;
/// `timerfd_create(2)` flag: set close-on-exec on the returned descriptor.
pub const TFD_CLOEXEC: i32 = 0o2000000;
/// `timerfd_settime(2)` flag: interpret `new_value.it_value` as an absolute
/// time on `clockid`, rather than relative to now.
pub const TFD_TIMER_ABSTIME: i32 = 1;

/// A timerfd's expiration schedule (kernel `struct itimerspec`). `it_value`
/// is when the timer next expires; `it_interval` is the period for a
/// repeating timer (zero for one-shot). An all-zero value disarms the timer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Itimerspec {
    /// Period between expirations after the first (zero: one-shot).
    pub it_interval: Timespec,
    /// Time until the next expiration (or, with `TFD_TIMER_ABSTIME`, the
    /// absolute time of the next expiration).
    pub it_value: Timespec,
}

const _: () = assert!(core::mem::size_of::<Itimerspec>() == 32);
const _: () = assert!(core::mem::offset_of!(Itimerspec, it_value) == 16);

/// Create a timer that notifies through a file descriptor: readable (and
/// `POLLIN` under [`crate::fd::poll`]) once it expires, with an 8-byte
/// expiration counter available via [`crate::fd::read`]. `clockid` is
/// typically [`CLOCK_MONOTONIC`] or [`CLOCK_REALTIME`]; `flags` is an OR of
/// [`TFD_NONBLOCK`]/[`TFD_CLOEXEC`].
///
/// Unlike `alarm`/`SIGALRM`, this composes directly with `poll`: a timeout
/// becomes one more fd in the same event loop as I/O readiness, instead of
/// an asynchronously-delivered signal with its own signal-safety rules.
pub fn timerfd_create(clockid: i32, flags: i32) -> Result<i32, Errno> {
    // SAFETY: plain integer arguments, no memory referenced.
    let ret = unsafe { syscall2(nr::TIMERFD_CREATE, clockid as usize, flags as usize) };
    from_ret_i32(ret)
}

/// Arm, disarm, or rearm the timer `fd` to `new_value`; passing an all-zero
/// [`Itimerspec`] disarms it. `flags` accepts [`TFD_TIMER_ABSTIME`]. Returns
/// the schedule that was in effect before this call.
pub fn timerfd_settime(fd: i32, flags: i32, new_value: &Itimerspec) -> Result<Itimerspec, Errno> {
    let mut old = Itimerspec::default();
    // SAFETY: `new_value` is a valid `*const itimerspec` the kernel only
    // reads; `old` is a valid, exclusively-borrowed `*mut itimerspec` the
    // kernel writes.
    let ret = unsafe {
        syscall4(
            nr::TIMERFD_SETTIME,
            fd as usize,
            flags as usize,
            new_value as *const Itimerspec as usize,
            &mut old as *mut Itimerspec as usize,
        )
    };
    from_ret(ret)?;
    Ok(old)
}

/// Get the timer `fd`'s current schedule: time remaining until the next
/// expiration, and its repeat interval.
pub fn timerfd_gettime(fd: i32) -> Result<Itimerspec, Errno> {
    let mut curr = Itimerspec::default();
    // SAFETY: `curr` is a valid, exclusively-borrowed `*mut itimerspec` the
    // kernel writes.
    let ret = unsafe {
        syscall2(
            nr::TIMERFD_GETTIME,
            fd as usize,
            &mut curr as *mut Itimerspec as usize,
        )
    };
    from_ret(ret)?;
    Ok(curr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_clock_advances() {
        let a = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        // Sleep a short, bounded interval, then confirm the clock moved forward.
        nanosleep(&Timespec::from_millis(5), None).expect("nanosleep");
        let b = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        let delta_ns = (b.tv_sec - a.tv_sec) * 1_000_000_000 + (b.tv_nsec - a.tv_nsec);
        assert!(delta_ns >= 5_000_000, "monotonic clock did not advance 5ms");
    }

    #[test]
    fn realtime_is_after_2020() {
        // Sanity: the realtime clock is past 2020-01-01 (1_577_836_800).
        let now = clock_gettime(CLOCK_REALTIME).expect("clock_gettime");
        assert!(now.tv_sec > 1_577_836_800);
    }

    #[test]
    fn coarse_and_boottime_clocks_work() {
        // Coarse clocks trade resolution for speed but report the same epoch.
        let realtime_coarse = clock_gettime(CLOCK_REALTIME_COARSE).expect("clock_gettime");
        assert!(realtime_coarse.tv_sec > 1_577_836_800);

        // Coarse monotonic must also advance, like the precise clock.
        let a = clock_gettime(CLOCK_MONOTONIC_COARSE).expect("clock_gettime");
        nanosleep(&Timespec::from_millis(20), None).expect("nanosleep");
        let b = clock_gettime(CLOCK_MONOTONIC_COARSE).expect("clock_gettime");
        let delta_ns = (b.tv_sec - a.tv_sec) * 1_000_000_000 + (b.tv_nsec - a.tv_nsec);
        assert!(delta_ns > 0, "coarse monotonic clock did not advance");

        // BOOTTIME tracks MONOTONIC closely absent a suspend/resume in between.
        let boot = clock_gettime(CLOCK_BOOTTIME).expect("clock_gettime");
        assert!(boot.tv_sec > 0);
    }

    #[test]
    fn clock_getres_reports_a_sane_small_resolution() {
        for clockid in [CLOCK_REALTIME, CLOCK_MONOTONIC] {
            let res = clock_getres(clockid).expect("clock_getres");
            // Never negative, and well under a second -- every real Linux
            // clock's resolution is nanosecond- or microsecond-scale.
            assert!(res.tv_sec == 0, "unexpectedly coarse resolution: {res:?}");
            assert!(res.tv_nsec > 0 && res.tv_nsec < 1_000_000_000);
        }
    }

    #[test]
    fn clock_getres_bad_clockid_is_einval() {
        assert_eq!(clock_getres(9999), Err(Errno::EINVAL));
    }

    // Read CLOCK_MONOTONIC directly via the syscall, bypassing the vDSO.
    fn raw_monotonic() -> i128 {
        let mut ts = Timespec::default();
        let ret = unsafe {
            crate::arch::syscall2(
                nr::CLOCK_GETTIME,
                CLOCK_MONOTONIC as usize,
                &mut ts as *mut Timespec as usize,
            )
        };
        crate::arch::from_ret(ret).expect("raw clock_gettime");
        ts.tv_sec as i128 * 1_000_000_000 + ts.tv_nsec as i128
    }

    #[test]
    fn vdso_wrapper_agrees_with_raw_syscall() {
        // Interleave raw-syscall reads around the public (maybe-vDSO) read; on
        // the monotonic clock the three must be non-decreasing. A vDSO entry
        // that returned garbage would violate the ordering.
        let before = raw_monotonic();
        let mid = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        let mid_ns = mid.tv_sec as i128 * 1_000_000_000 + mid.tv_nsec as i128;
        let after = raw_monotonic();

        assert!(
            before <= mid_ns,
            "wrapper read is before the prior syscall read"
        );
        assert!(
            mid_ns <= after,
            "wrapper read is after the later syscall read"
        );
    }

    // The vDSO is always mapped on native x86_64 (and readable via
    // /proc/self/auxv), so resolution must succeed there — proving the fast
    // path is actually taken rather than silently always falling back. Not
    // asserted on aarch64, where the CI job runs under qemu-user, which may not
    // expose a guest vDSO.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn vdso_resolves_on_native_x86_64() {
        assert!(
            crate::vdso::clock_gettime_fn().is_some(),
            "vDSO clock_gettime should resolve on native x86_64"
        );
    }

    #[test]
    fn from_millis_splits_correctly() {
        assert_eq!(
            Timespec::from_millis(1500),
            Timespec {
                tv_sec: 1,
                tv_nsec: 500_000_000
            }
        );
        assert_eq!(
            Timespec::from_millis(0),
            Timespec {
                tv_sec: 0,
                tv_nsec: 0
            }
        );
    }

    #[test]
    fn timerfd_one_shot_fires_and_is_readable() {
        let tfd = timerfd_create(CLOCK_MONOTONIC, 0).expect("timerfd_create");

        let value = Itimerspec {
            it_interval: Timespec::default(),
            it_value: Timespec::from_millis(50),
        };
        timerfd_settime(tfd, 0, &value).expect("timerfd_settime");

        let mut fds = [crate::fd::PollFd::new(tfd, crate::fd::POLLIN)];
        let n = crate::fd::poll(&mut fds, 1000).expect("poll");
        assert_eq!(n, 1, "timer did not fire within 1s");
        assert!(fds[0].is_readable());

        // Reading yields an 8-byte little-endian expiration count.
        let mut buf = [0u8; 8];
        let read_n = crate::fd::read(tfd, &mut buf).expect("read expiration count");
        assert_eq!(read_n, 8);
        assert_eq!(u64::from_ne_bytes(buf), 1);

        crate::fd::close(tfd).expect("close");
    }

    #[test]
    fn timerfd_gettime_reports_remaining_and_interval() {
        let tfd = timerfd_create(CLOCK_MONOTONIC, 0).expect("timerfd_create");

        let value = Itimerspec {
            it_interval: Timespec::from_millis(200),
            it_value: Timespec::from_millis(10_000), // far in the future, won't fire during the test
        };
        let old = timerfd_settime(tfd, 0, &value).expect("timerfd_settime");
        // A freshly created timer had no prior schedule.
        assert_eq!(old, Itimerspec::default());

        let curr = timerfd_gettime(tfd).expect("timerfd_gettime");
        assert_eq!(curr.it_interval, value.it_interval);
        // Remaining time must be positive and no larger than what we set.
        assert!(curr.it_value.tv_sec >= 0 && curr.it_value.tv_sec <= 10);
        assert!(curr.it_value.tv_sec > 0 || curr.it_value.tv_nsec > 0);

        crate::fd::close(tfd).expect("close");
    }

    #[test]
    fn timerfd_disarm_with_zero_value_clears_schedule() {
        let tfd = timerfd_create(CLOCK_MONOTONIC, 0).expect("timerfd_create");

        let value = Itimerspec {
            it_interval: Timespec::default(),
            it_value: Timespec::from_millis(10_000),
        };
        timerfd_settime(tfd, 0, &value).expect("timerfd_settime arm");

        let disarmed =
            timerfd_settime(tfd, 0, &Itimerspec::default()).expect("timerfd_settime disarm");
        // The previous (armed) schedule is returned.
        assert!(disarmed.it_value.tv_sec > 0);

        let curr = timerfd_gettime(tfd).expect("timerfd_gettime");
        assert_eq!(curr, Itimerspec::default());

        crate::fd::close(tfd).expect("close");
    }

    #[test]
    fn timerfd_create_bad_clockid_is_einval() {
        assert_eq!(timerfd_create(9999, 0), Err(Errno::EINVAL));
    }

    #[test]
    fn clock_nanosleep_relative_sleeps_at_least_the_requested_duration() {
        let start = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        let req = Timespec::from_millis(20);
        clock_nanosleep(CLOCK_MONOTONIC, 0, &req, None).expect("clock_nanosleep");
        let end = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");

        let elapsed_ms =
            (end.tv_sec - start.tv_sec) * 1000 + (end.tv_nsec - start.tv_nsec) / 1_000_000;
        assert!(elapsed_ms >= 20, "slept only {elapsed_ms}ms, wanted >= 20");
    }

    #[test]
    fn clock_nanosleep_abstime_sleeps_until_the_deadline() {
        let now = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        // now + 20ms, on the same (arbitrary-epoch) monotonic timeline
        // `now` itself came from -- absolute deadlines only make sense
        // relative to a clock reading already on that timeline.
        let mut deadline_nsec = now.tv_nsec + 20_000_000;
        let deadline = Timespec {
            tv_sec: now.tv_sec + deadline_nsec / 1_000_000_000,
            tv_nsec: {
                deadline_nsec %= 1_000_000_000;
                deadline_nsec
            },
        };

        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &deadline, None)
            .expect("clock_nanosleep abstime");

        let end = clock_gettime(CLOCK_MONOTONIC).expect("clock_gettime");
        assert!(
            (end.tv_sec, end.tv_nsec) >= (deadline.tv_sec, deadline.tv_nsec),
            "woke at {end:?}, before the deadline {deadline:?}"
        );
    }

    #[test]
    fn clock_nanosleep_bad_clockid_is_einval() {
        let req = Timespec::default();
        assert_eq!(clock_nanosleep(9999, 0, &req, None), Err(Errno::EINVAL));
    }
}
