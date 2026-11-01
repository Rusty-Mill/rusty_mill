//! Round-trip estimate and congestion window.

use std::time::Duration;

/// Bytes of stream data one datagram carries at most, for window sums.
pub(crate) const MSS: u64 = 1100;

const MIN_RTO: Duration = Duration::from_millis(100);
const MAX_RTO: Duration = Duration::from_secs(2);
const INITIAL_RTO: Duration = Duration::from_secs(1);
/// The smallest window: two datagrams.
const MIN_WINDOW: u64 = 2 * MSS;
/// The window a path starts with: ten datagrams.
const INITIAL_WINDOW: u64 = 10 * MSS;

/// RFC 6298's smoothed round trip and its variation.
#[derive(Debug, Default)]
pub(crate) struct Rtt {
    smoothed: Option<(Duration, Duration)>,
}

impl Rtt {
    pub(crate) fn sample(&mut self, rtt: Duration) {
        self.smoothed = Some(match self.smoothed {
            None => (rtt, rtt / 2),
            Some((srtt, var)) => {
                let diff = srtt.abs_diff(rtt);
                (srtt * 7 / 8 + rtt / 8, var * 3 / 4 + diff / 4)
            }
        });
    }

    /// The retransmission timeout after `backoff` doublings, between
    /// 100 ms and 2 s.
    pub(crate) fn rto(&self, backoff: u32) -> Duration {
        let base = match self.smoothed {
            None => INITIAL_RTO,
            Some((srtt, var)) => srtt + (var * 4).max(Duration::from_millis(1)),
        };
        let backed = base.saturating_mul(1 << backoff.min(5));
        backed.clamp(MIN_RTO, MAX_RTO)
    }

    pub(crate) fn smoothed(&self) -> Option<Duration> {
        self.smoothed.map(|(srtt, _)| srtt)
    }
}

/// A NewReno-style window in bytes.
#[derive(Debug)]
pub(crate) struct Window {
    bytes: u64,
    threshold: u64,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            bytes: INITIAL_WINDOW,
            threshold: u64::MAX,
        }
    }
}

impl Window {
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Grows by what was delivered: doubling per round trip below the
    /// threshold, one datagram per round trip above it.
    pub(crate) fn delivered(&mut self, bytes: u64) {
        let growth = if self.bytes < self.threshold {
            bytes
        } else {
            (MSS * bytes / self.bytes).max(1)
        };
        self.bytes = self.bytes.saturating_add(growth);
    }

    /// Halves on a loss the acks revealed.
    pub(crate) fn lost(&mut self) {
        self.threshold = (self.bytes / 2).max(MIN_WINDOW);
        self.bytes = self.threshold;
    }

    /// Falls to the minimum when the timer fired.
    pub(crate) fn timed_out(&mut self) {
        self.threshold = (self.bytes / 2).max(MIN_WINDOW);
        self.bytes = MIN_WINDOW;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_timeout_follows_the_round_trip_within_its_bounds() {
        let mut rtt = Rtt::default();
        assert_eq!(rtt.rto(0), INITIAL_RTO);
        rtt.sample(100 * MS);
        assert_eq!(rtt.rto(0), 300 * MS, "100 + 4 * 50");
        assert_eq!(rtt.rto(1), 600 * MS);
        assert_eq!(rtt.rto(9), MAX_RTO);
        let mut fast = Rtt::default();
        fast.sample(MS);
        assert_eq!(fast.rto(0), MIN_RTO);
        for _ in 0..50 {
            rtt.sample(40 * MS);
        }
        assert!(rtt.smoothed().unwrap() < 45 * MS);
    }

    #[test]
    fn the_window_grows_halves_and_collapses() {
        let mut window = Window::default();
        window.delivered(INITIAL_WINDOW);
        assert_eq!(window.bytes(), 2 * INITIAL_WINDOW, "slow start");
        window.lost();
        assert_eq!(window.bytes(), INITIAL_WINDOW);
        window.delivered(INITIAL_WINDOW);
        assert_eq!(
            window.bytes(),
            INITIAL_WINDOW + MSS,
            "one datagram a round trip"
        );
        window.timed_out();
        assert_eq!(window.bytes(), MIN_WINDOW);
        for _ in 0..10 {
            window.lost();
        }
        assert_eq!(window.bytes(), MIN_WINDOW);
    }
}
