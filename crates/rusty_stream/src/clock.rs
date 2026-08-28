//! An injectable clock, for the same reason storage I/O goes through
//! `rusty_tokio`'s `OpDriver`/`SimDriver` (ADR-0002 D3/D4) instead of calling
//! the real thing directly: time-based retention (`docs/phase1-scope.md` §2)
//! needs to be deterministically testable — "this segment is older than the
//! retention window" has to be provable without a real test actually
//! sleeping for that long.

use std::sync::atomic::{AtomicU64, Ordering};

/// Milliseconds since an arbitrary epoch. Only ever compared to another
/// reading from the *same* clock instance — never persisted, never compared
/// across a process restart (a fresh [`SystemClock`] reading after restart
/// has no defined relationship to readings from before it stopped; see
/// `retention::Log::open`'s docs for the concrete consequence this has for
/// recovered segments' ages).
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> u64;
}

/// The real clock — wraps [`std::time::SystemTime`].
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_millis() as u64
    }
}

/// A deterministic clock for tests: starts at 0, advances only when told to.
/// Pairs with `rusty_tokio::io::SimDriver` the same way `SimDriver` itself
/// pairs with the real `IoUringDriver` — a storage engine's own
/// crash-recovery/retention tests exercise the real code path against a
/// fully controlled environment, no real time or real disk involved.
pub struct SimClock(AtomicU64);

impl SimClock {
    pub fn new() -> SimClock {
        SimClock(AtomicU64::new(0))
    }

    /// Moves the clock forward by `millis`.
    pub fn advance(&self, millis: u64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Default for SimClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SimClock {
    fn now_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_clock_starts_at_zero_and_only_moves_when_told() {
        let clock = SimClock::new();
        assert_eq!(clock.now_millis(), 0);
        clock.advance(100);
        assert_eq!(clock.now_millis(), 100);
        clock.advance(50);
        assert_eq!(clock.now_millis(), 150);
    }

    #[test]
    fn system_clock_reads_something_plausible() {
        // Not deterministic by design (it's the real clock) -- just a smoke
        // check that it returns a sane, present-day value rather than
        // panicking or returning 0.
        let clock = SystemClock;
        let now = clock.now_millis();
        // 2020-01-01T00:00:00Z in millis -- any real reading is well past this.
        assert!(now > 1_577_836_800_000);
    }
}
