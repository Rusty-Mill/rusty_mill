//! Output repaint pacing: during an output flood, repaint at most once per
//! display refresh.
//!
//! The GPU renderer's Fifo present already blocks to the refresh rate. The
//! CPU renderer presents as fast as wakeups arrive, about 1150 fps on a
//! flood. Each of those frames holds the grid lock that the parser needs, so
//! unpaced repaint cost 6–8% of `type` throughput on Windows (Beast,
//! 2026-11-03). Frames the display never shows are only lost work.
//!
//! Pacing applies only to output-driven repaints. Input, resize and timers
//! still repaint at once, and any frame they paint also covers held output,
//! because panes share the grid.

use std::time::{Duration, Instant};

/// Refresh rate assumed when the monitor does not report one, in mHz.
const FALLBACK_MILLIHERTZ: u32 = 60_000;

/// Paces output-driven repaints to one per `interval`.
#[derive(Debug)]
pub(crate) struct Pacer {
    /// Minimum gap between output-driven frames; zero turns pacing off.
    interval: Duration,
    /// When the last frame was painted, for any reason.
    last: Option<Instant>,
    /// A held output repaint, due at this time.
    due: Option<Instant>,
}

impl Pacer {
    /// A pacer that repaints output immediately, as before pacing existed.
    pub(crate) fn unpaced() -> Self {
        Pacer {
            interval: Duration::ZERO,
            last: None,
            due: None,
        }
    }

    /// Pace to a display refreshing at `millihertz`. `None` (unknown) or zero
    /// falls back to 60 Hz.
    pub(crate) fn set_refresh(&mut self, millihertz: Option<u32>) {
        let mhz = millihertz.filter(|&m| m > 0).unwrap_or(FALLBACK_MILLIHERTZ);
        self.interval = Duration::from_nanos(1_000_000_000_000 / u64::from(mhz));
    }

    /// Stop pacing, for a renderer whose present already waits for vsync.
    pub(crate) fn disable(&mut self) {
        self.interval = Duration::ZERO;
        self.due = None;
    }

    /// Output arrived at `now`. Returns `true` to repaint now; otherwise the
    /// repaint is held until [`poll`](Self::poll) says it is due.
    pub(crate) fn output(&mut self, now: Instant) -> bool {
        if self.due.is_some() {
            return false;
        }
        let next = self.last.map_or(now, |last| last + self.interval);
        if next <= now {
            return true;
        }
        self.due = Some(next);
        false
    }

    /// A frame was painted at `now`. It also covers any held output repaint.
    pub(crate) fn painted(&mut self, now: Instant) {
        self.last = Some(now);
        self.due = None;
    }

    /// Timer check at `now`. Returns whether the held repaint is due now, and
    /// otherwise the deadline still pending, if any.
    pub(crate) fn poll(&mut self, now: Instant) -> (bool, Option<Instant>) {
        match self.due {
            Some(at) if at <= now => {
                self.due = None;
                (true, None)
            }
            pending => (false, pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn paced(hz: u32) -> Pacer {
        let mut p = Pacer::unpaced();
        p.set_refresh(Some(hz * 1000));
        p
    }

    #[test]
    fn first_output_repaints_at_once() {
        let mut p = paced(240);
        assert!(p.output(Instant::now()));
    }

    #[test]
    fn output_within_a_frame_is_held_until_the_next_refresh() {
        let t0 = Instant::now();
        let mut p = paced(100); // 10 ms frames
        p.painted(t0);
        assert!(!p.output(t0 + 2 * MS));
        assert!(!p.output(t0 + 3 * MS), "a held repaint absorbs more output");
        assert_eq!(p.poll(t0 + 5 * MS), (false, Some(t0 + 10 * MS)));
        assert_eq!(p.poll(t0 + 10 * MS), (true, None));
        assert_eq!(p.poll(t0 + 11 * MS), (false, None), "fires once");
    }

    #[test]
    fn output_after_a_full_frame_repaints_at_once() {
        let t0 = Instant::now();
        let mut p = paced(100);
        p.painted(t0);
        assert!(p.output(t0 + 10 * MS));
        assert!(p.output(t0 + 50 * MS));
    }

    #[test]
    fn any_frame_covers_a_held_repaint() {
        let t0 = Instant::now();
        let mut p = paced(100);
        p.painted(t0);
        assert!(!p.output(t0 + MS));
        p.painted(t0 + 2 * MS); // a keypress repainted
        assert_eq!(p.poll(t0 + 20 * MS), (false, None));
    }

    #[test]
    fn unpaced_and_disabled_never_hold() {
        let t0 = Instant::now();
        let mut p = Pacer::unpaced();
        p.painted(t0);
        assert!(p.output(t0));
        let mut p = paced(60);
        p.painted(t0);
        assert!(!p.output(t0 + MS));
        p.disable();
        assert_eq!(
            p.poll(t0 + MS),
            (false, None),
            "disable drops the held repaint"
        );
        assert!(p.output(t0 + MS));
    }

    #[test]
    fn unknown_or_zero_refresh_falls_back_to_60_hz() {
        for rate in [None, Some(0)] {
            let mut p = Pacer::unpaced();
            p.set_refresh(rate);
            assert_eq!(p.interval, Duration::from_nanos(16_666_666));
        }
    }
}
