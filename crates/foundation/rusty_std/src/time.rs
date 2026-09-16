//! Sovereign Time abstractions for rusty_std.

/// A duration in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    secs: u64,
    nanos: u32,
}

impl Duration {
    /// Creates a Duration from seconds.
    pub const fn from_secs(secs: u64) -> Self {
        Self { secs, nanos: 0 }
    }

    /// Creates a Duration from milliseconds.
    pub const fn from_millis(millis: u64) -> Self {
        Self {
            secs: millis / 1000,
            nanos: ((millis % 1000) * 1_000_000) as u32,
        }
    }

    /// Returns the total number of seconds.
    pub const fn as_secs(&self) -> u64 {
        self.secs
    }

    /// Returns the sub-second remainder in nanoseconds (`0..1_000_000_000`).
    pub const fn subsec_nanos(&self) -> u32 {
        self.nanos
    }
}

/// A measurement of a monotonically nondecreasing clock.
///
/// Backed by a real OS clock: `rusty_libc::time::clock_gettime(CLOCK_MONOTONIC)`
/// on Linux, `rusty_win32::now_monotonic` (`QueryPerformanceCounter`) on
/// Windows. On any other target (e.g. `wasm32`) there is no wired clock yet,
/// and `now`/`elapsed` fall back to a stub that always reports zero elapsed
/// time -- callers on those targets must not rely on `elapsed()` for real
/// timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instant {
    secs: u64,
    nanos: u32,
}

impl Instant {
    /// Returns an Instant corresponding to "now", read from a real
    /// monotonic OS clock on Linux/Windows (see the type-level docs for the
    /// unwired-target fallback).
    pub fn now() -> Self {
        #[cfg(target_os = "linux")]
        {
            let ts = rusty_libc::time::clock_gettime(rusty_libc::time::CLOCK_MONOTONIC)
                .unwrap_or_default();
            Self {
                secs: ts.tv_sec as u64,
                nanos: ts.tv_nsec as u32,
            }
        }
        #[cfg(windows)]
        {
            let ts = rusty_win32::now_monotonic().unwrap_or_default();
            Self {
                secs: ts.secs as u64,
                nanos: ts.nanos,
            }
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Self { secs: 0, nanos: 0 }
        }
    }

    /// Returns the amount of time elapsed since this instant was created,
    /// measured against the same real monotonic clock `now()` reads (see
    /// the type-level docs for the unwired-target fallback).
    pub fn elapsed(&self) -> Duration {
        let now = Self::now();
        let secs = now.secs.saturating_sub(self.secs);
        let (secs, nanos) = if now.nanos < self.nanos {
            (
                secs.saturating_sub(1),
                now.nanos + 1_000_000_000 - self.nanos,
            )
        } else {
            (secs, now.nanos - self.nanos)
        };
        Duration { secs, nanos }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real clock-backed: proves `Instant::now`/`elapsed` actually read a
    // wall-clock-adjacent monotonic clock (rusty_libc on Linux, rusty_win32
    // on Windows) rather than being hardcoded to zero, as the stub this
    // module started as always reported.
    #[test]
    fn elapsed_reflects_real_wall_clock_time() {
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let elapsed = start.elapsed();

        let elapsed_nanos =
            elapsed.as_secs() as u128 * 1_000_000_000 + elapsed.subsec_nanos() as u128;
        assert!(
            elapsed_nanos >= 10_000_000,
            "elapsed() reported {elapsed_nanos}ns after a real 20ms sleep -- \
             Instant is not wired to a real clock"
        );
    }
}
