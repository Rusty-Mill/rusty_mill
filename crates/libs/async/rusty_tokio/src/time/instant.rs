//! [`Instant`]: tokio's `time::Instant`, a measurement of the current
//! runtime's (pausable) clock rather than of real time directly.
//!
//! Why a newtype rather than re-exporting `std::time::Instant`: the
//! whole point of [`super::pause`]/[`super::advance`] is that "now" is
//! whatever the runtime's clock says, and `std::time::Instant::now()`
//! can't be taught that -- code that measures elapsed time or computes a
//! deadline with std's `now()` keeps reading real time while every timer
//! it feeds runs on the frozen virtual clock, so a paused test either
//! hangs or silently compares against the wrong reference. Reading
//! [`Instant::now`] instead keeps both sides of every comparison on the
//! same clock, which is also what tokio's own `Instant` exists for.
//!
//! The wrapped value is still a plain `std::time::Instant` -- the paused
//! clock itself is represented as one (see `clock.rs`'s docs) -- so
//! [`Instant::from_std`]/[`Instant::into_std`] are lossless and a
//! deadline computed from either kind of "now" compares correctly with
//! the other.

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::time::Duration;

/// A measurement of the current runtime's clock -- see this module's
/// docs for why it isn't simply `std::time::Instant`. Outside any
/// runtime, [`now`](Self::now) falls back to real time, the same as
/// tokio's.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    std: std::time::Instant,
}

impl Instant {
    /// The current runtime's "now": frozen while [`super::pause`] is in
    /// effect (and moved only by [`super::advance`] or auto-advance),
    /// real time otherwise or when called with no runtime at all.
    pub fn now() -> Instant {
        Instant::from_std(super::clock_now())
    }

    /// Wraps an existing `std::time::Instant` -- no clock is read.
    pub fn from_std(std: std::time::Instant) -> Instant {
        Instant { std }
    }

    /// The wrapped `std::time::Instant`.
    pub fn into_std(self) -> std::time::Instant {
        self.std
    }

    /// Time from `earlier` to `self`, zero if `earlier` is later --
    /// saturating, like std's own since Rust 1.60 and tokio's.
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        self.std.saturating_duration_since(earlier.std)
    }

    /// Time from `earlier` to `self`, or `None` if `earlier` is later.
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.std.checked_duration_since(earlier.std)
    }

    /// Time from `earlier` to `self`, zero if `earlier` is later.
    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.std.saturating_duration_since(earlier.std)
    }

    /// Time since `self`, read against [`Instant::now`] -- so under a
    /// paused clock this is virtual elapsed time, not real.
    pub fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(*self)
    }

    /// `self + duration`, or `None` if that isn't representable.
    pub fn checked_add(&self, duration: Duration) -> Option<Instant> {
        self.std.checked_add(duration).map(Instant::from_std)
    }

    /// `self - duration`, or `None` if that isn't representable.
    pub fn checked_sub(&self, duration: Duration) -> Option<Instant> {
        self.std.checked_sub(duration).map(Instant::from_std)
    }
}

impl From<std::time::Instant> for Instant {
    fn from(std: std::time::Instant) -> Instant {
        Instant::from_std(std)
    }
}

impl From<Instant> for std::time::Instant {
    fn from(instant: Instant) -> std::time::Instant {
        instant.into_std()
    }
}

/// # Panics
/// Panics on overflow, like `std::time::Instant`'s own `Add`.
impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Instant {
        Instant::from_std(self.std + rhs)
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

/// # Panics
/// Panics on underflow, like `std::time::Instant`'s own `Sub`.
impl Sub<Duration> for Instant {
    type Output = Instant;

    fn sub(self, rhs: Duration) -> Instant {
        Instant::from_std(self.std - rhs)
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

/// Saturates at zero when `rhs` is later, the same as
/// [`Instant::duration_since`].
impl Sub<Instant> for Instant {
    type Output = Duration;

    fn sub(self, rhs: Instant) -> Duration {
        self.saturating_duration_since(rhs)
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.std.fmt(f)
    }
}
