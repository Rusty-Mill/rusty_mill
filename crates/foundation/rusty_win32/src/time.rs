//! `QueryPerformanceCounter`/`GetSystemTimePreciseAsFileTime` — the Windows
//! analog of `rusty_libc::vdso`'s "skip the syscall, read kernel-shared
//! memory" fast path. `QueryPerformanceCounter` is documented to be backed
//! by the same `KUSER_SHARED_DATA` page Windows maps read-only into every
//! process for exactly this reason, a genuine parallel to the vDSO trick
//! rather than a differently-shaped primitive.
//!
//! Lowest-priority module in this crate (see rush's
//! `docs/WINDOWS_BACKEND_ANALYSIS.md` §3): no `cfg(not(unix))` site in rush
//! calls for it today — rush uses `std::time` exclusively, and std's own
//! Windows backend already uses `QueryPerformanceCounter` internally. This
//! exists for [rusty_lines](https://github.com/baileyrd/rusty_lines) and
//! for completeness, not to close an open rush gap.

use crate::error::Win32Error;

/// A `timespec`-equivalent: seconds plus a sub-second remainder in
/// nanoseconds (`0..1_000_000_000`), matching `rusty_libc`'s own
/// `Timespec` shape for symmetry across the two crates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timespec {
    pub secs: i64,
    pub nanos: u32,
}

// FILETIME: `size_of` 8, `align_of` 4 on x86_64 — two `DWORD`s, no padding.
// Verified against mingw-w64's `minwinbase.h`/`lmaccess.h` the same way as
// `process.rs`/`job.rs`'s structs (a `_Static_assert` probe compiled with
// `x86_64-w64-mingw32-gcc` against the real header).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}
const _: () = assert!(core::mem::size_of::<FileTime>() == 8);
const _: () = assert!(core::mem::align_of::<FileTime>() == 4);

/// 100ns ticks between the FILETIME epoch (1601-01-01) and the Unix epoch
/// (1970-01-01) — the standard, widely-documented conversion constant.
const FILETIME_UNIX_EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
const HUNDRED_NS_PER_SEC: i64 = 10_000_000;
const NANOS_PER_HUNDRED_NS: i64 = 100;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(performance_count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    fn GetSystemTimePreciseAsFileTime(system_time_as_file_time: *mut FileTime);
}

/// A monotonic timestamp — the Windows analog of `CLOCK_MONOTONIC`, via
/// `QueryPerformanceCounter`/`QueryPerformanceFrequency`. Not comparable
/// across processes or reboots; only meaningful as a difference between two
/// calls in the same running system.
///
/// Documented to succeed on every Windows version since XP (this crate's
/// own floor is far newer — Windows 10 1809+, per ConPTY's requirement —
/// so failure here would indicate a fundamentally broken host, not a
/// reachable error path in practice); still returns `Result` rather than
/// panicking, matching every other fallible call in this crate.
pub fn now_monotonic() -> Result<Timespec, Win32Error> {
    let mut ticks: i64 = 0;
    let mut freq: i64 = 0;
    // SAFETY: both out-pointers are valid, non-null locals of the exact
    // `i64` (`LARGE_INTEGER`-as-`LONGLONG`) size `QueryPerformanceCounter`/
    // `QueryPerformanceFrequency` write.
    let ok = unsafe { QueryPerformanceCounter(&mut ticks) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    // SAFETY: see above.
    let ok = unsafe { QueryPerformanceFrequency(&mut freq) };
    if ok == 0 {
        return Err(Win32Error::last());
    }
    let secs = ticks / freq;
    let remainder_ticks = ticks % freq;
    let nanos = (remainder_ticks as i128 * 1_000_000_000 / freq as i128) as u32;
    Ok(Timespec { secs, nanos })
}

/// A wall-clock timestamp (seconds/nanoseconds since the Unix epoch) — the
/// Windows analog of `CLOCK_REALTIME`, via
/// `GetSystemTimePreciseAsFileTime` (the sub-millisecond-precision sibling
/// of `GetSystemTimeAsFileTime`, available since Windows 8 — well within
/// this crate's Windows 10 1809+ floor).
pub fn now_realtime() -> Timespec {
    let mut ft = FileTime::default();
    // SAFETY: `ft` is a valid, correctly-sized out-pointer;
    // `GetSystemTimePreciseAsFileTime` has no other precondition and is
    // documented to never fail.
    unsafe { GetSystemTimePreciseAsFileTime(&mut ft) };
    let ticks_100ns =
        (i64::from(ft.high) << 32 | i64::from(ft.low)) - FILETIME_UNIX_EPOCH_DIFF_100NS;
    let secs = ticks_100ns.div_euclid(HUNDRED_NS_PER_SEC);
    let remainder_100ns = ticks_100ns.rem_euclid(HUNDRED_NS_PER_SEC);
    Timespec {
        secs,
        nanos: (remainder_100ns * NANOS_PER_HUNDRED_NS) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_monotonic_is_nondecreasing_and_advances() {
        let first = now_monotonic().expect("QueryPerformanceCounter should succeed");
        // Busy-loop a little real work rather than sleeping, so this stays
        // fast and doesn't depend on a scheduler-granularity sleep firing.
        let mut acc: u64 = 0;
        for i in 0..5_000_000u64 {
            acc = acc.wrapping_add(i);
        }
        core::hint::black_box(acc);
        let second = now_monotonic().expect("QueryPerformanceCounter should succeed");

        assert!(
            (second.secs, second.nanos) >= (first.secs, first.nanos),
            "monotonic clock must not go backwards"
        );
        assert!(
            second.secs > first.secs || second.nanos > first.nanos,
            "clock must advance"
        );
    }

    #[test]
    fn now_realtime_is_a_plausible_unix_timestamp() {
        let t = now_realtime();
        // Sanity bounds rather than an exact value: comfortably after this
        // crate's own creation date and comfortably before any plausible
        // clock-misconfiguration false positive.
        assert!(t.secs > 1_700_000_000, "timestamp should be after ~2023");
        assert!(t.secs < 4_000_000_000, "timestamp should be before ~2096");
        assert!(
            t.nanos < 1_000_000_000,
            "nanos must be a sub-second remainder"
        );
    }
}
