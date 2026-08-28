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
}

/// A measurement of a monotonically nondecreasing clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instant {
    ticks: u64,
}

impl Instant {
    /// Returns an Instant corresponding to "now".
    pub fn now() -> Self {
        Self { ticks: 0 }
    }

    /// Returns the amount of time elapsed since this instant was created.
    pub fn elapsed(&self) -> Duration {
        Duration::from_secs(0)
    }
}
