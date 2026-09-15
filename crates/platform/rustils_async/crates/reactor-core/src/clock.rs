//! Explicit time source (ADR-0160: engines "accept explicit... monotonic
//! time where needed"). Consumers depend on the [`Clock`] trait rather
//! than calling `Instant::now()` directly, so a deterministic clock can
//! be substituted in tests — Atlas `ATLAS-DET-0001`/`0010`:
//! reproducible behavior, explicit and documented nondeterminism.

use std::time::Instant;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The real monotonic clock (`Instant::now()`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_across_two_reads() {
        let clock = SystemClock;
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }
}
