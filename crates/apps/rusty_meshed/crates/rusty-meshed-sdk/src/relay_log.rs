//! Rate-limited stderr logging for the outbox relay thread.
//!
//! The relay retries every cycle, so a persistent failure (a dead broker,
//! an unreadable clock) would otherwise log every two seconds. This keeps
//! the first error of each kind, then at most one line per [`WINDOW`] with
//! a count of what was suppressed, and one line when work succeeds again.
//!
//! Time is monotonic (`Instant`) and passed in, so a wall-clock jump can
//! neither silence nor flood the log and tests never sleep. Categories are
//! a fixed enum -- error text never becomes a key, so memory is constant.

use std::io::Write;
use std::time::{Duration, Instant};

/// Minimum spacing between lines for one error kind.
pub(crate) const WINDOW: Duration = Duration::from_secs(60);

/// Bounded error categories, derived from the error variant, never its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    Clock,
    Kafka,
    Database,
}

impl ErrorKind {
    const COUNT: usize = 3;

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            ErrorKind::Clock => "clock",
            ErrorKind::Kafka => "kafka",
            ErrorKind::Database => "database",
        }
    }
}

/// What one polling cycle did.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// Nothing was pending; the broker was not contacted.
    Idle,
    /// At least one entry was produced and committed. Never used for an
    /// empty batch -- that is [`Outcome::Idle`].
    Relayed,
    /// The cycle failed.
    Failed { kind: ErrorKind, detail: String },
}

#[derive(Debug, Clone, Copy, Default)]
struct KindState {
    last_logged: Option<Instant>,
    suppressed: u32,
    failed: bool,
}

/// Rate-limited writer of relay outcomes.
pub(crate) struct RelayLog<W: Write> {
    out: W,
    window: Duration,
    states: [KindState; ErrorKind::COUNT],
}

impl<W: Write> RelayLog<W> {
    pub(crate) fn new(out: W, window: Duration) -> Self {
        RelayLog {
            out,
            window,
            states: [KindState::default(); ErrorKind::COUNT],
        }
    }

    /// Accounts for one cycle's `outcome` at monotonic time `now`.
    pub(crate) fn record(&mut self, now: Instant, outcome: Outcome) {
        match outcome {
            // An empty batch is not evidence of recovery: no state change, no line.
            Outcome::Idle => {}
            Outcome::Relayed => self.recover(),
            Outcome::Failed { kind, detail } => self.fail(now, kind, &detail),
        }
    }

    fn fail(&mut self, now: Instant, kind: ErrorKind, detail: &str) {
        let window = self.window;
        let state = &mut self.states[kind.index()];
        state.failed = true;
        let due = state
            .last_logged
            .is_none_or(|last| now.saturating_duration_since(last) >= window);
        if !due {
            state.suppressed = state.suppressed.saturating_add(1);
            return;
        }
        let suppressed = std::mem::take(&mut state.suppressed);
        state.last_logged = Some(now);
        if suppressed == 0 {
            self.line(format_args!(
                "outbox relay {} error: {detail}",
                kind.label()
            ));
        } else {
            self.line(format_args!(
                "outbox relay {} error: {detail} ({suppressed} similar errors suppressed)",
                kind.label()
            ));
        }
    }

    fn recover(&mut self) {
        if !self.states.iter().any(|s| s.failed) {
            return;
        }
        self.states = [KindState::default(); ErrorKind::COUNT];
        self.line(format_args!("outbox relay recovered"));
    }

    fn line(&mut self, message: std::fmt::Arguments<'_>) {
        // Deliberately discarded: a closed or full stderr must not kill the
        // relay thread, and there is nowhere left to report it.
        let _ = writeln!(self.out, "{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared buffer so the test can read what the log wrote.
    #[derive(Clone, Default)]
    struct Sink(Rc<RefCell<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Sink {
        fn lines(&self) -> Vec<String> {
            String::from_utf8(self.0.borrow().clone())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn failed(kind: ErrorKind) -> Outcome {
        Outcome::Failed {
            kind,
            detail: "boom".to_string(),
        }
    }

    fn relayed() -> Outcome {
        Outcome::Relayed
    }

    fn log() -> (RelayLog<Sink>, Sink, Instant) {
        let sink = Sink::default();
        (RelayLog::new(sink.clone(), WINDOW), sink, Instant::now())
    }

    #[test]
    fn the_first_error_logs_and_repeats_inside_the_window_are_suppressed() {
        let (mut log, sink, t0) = log();
        log.record(t0, failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(2), failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(59), failed(ErrorKind::Kafka));
        assert_eq!(sink.lines(), ["outbox relay kafka error: boom"]);
    }

    #[test]
    fn the_window_expiring_logs_the_suppressed_count_and_resets_it() {
        let (mut log, sink, t0) = log();
        log.record(t0, failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(2), failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(4), failed(ErrorKind::Kafka));
        log.record(t0 + WINDOW, failed(ErrorKind::Kafka));
        log.record(
            t0 + WINDOW + Duration::from_secs(2),
            failed(ErrorKind::Kafka),
        );
        log.record(t0 + WINDOW * 2, failed(ErrorKind::Kafka));
        assert_eq!(
            sink.lines(),
            [
                "outbox relay kafka error: boom",
                "outbox relay kafka error: boom (2 similar errors suppressed)",
                "outbox relay kafka error: boom (1 similar errors suppressed)",
            ]
        );
    }

    #[test]
    fn kinds_are_limited_independently() {
        let (mut log, sink, t0) = log();
        log.record(t0, failed(ErrorKind::Kafka));
        log.record(t0, failed(ErrorKind::Clock));
        log.record(t0, failed(ErrorKind::Database));
        log.record(t0, failed(ErrorKind::Clock));
        assert_eq!(sink.lines().len(), 3);
    }

    #[test]
    fn the_suppressed_counter_saturates() {
        let (mut log, _sink, t0) = log();
        log.states[ErrorKind::Kafka.index()] = KindState {
            last_logged: Some(t0),
            suppressed: u32::MAX,
            failed: true,
        };
        log.record(t0 + Duration::from_secs(1), failed(ErrorKind::Kafka));
        assert_eq!(log.states[ErrorKind::Kafka.index()].suppressed, u32::MAX);
    }

    #[test]
    fn idle_after_a_failure_neither_logs_nor_clears() {
        let (mut log, sink, t0) = log();
        log.record(t0, failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(2), Outcome::Idle);
        assert_eq!(sink.lines().len(), 1);
        assert!(log.states[ErrorKind::Kafka.index()].failed);
        // still failing, still inside the window: suppressed, not re-logged
        log.record(t0 + Duration::from_secs(4), failed(ErrorKind::Kafka));
        assert_eq!(sink.lines().len(), 1);
    }

    #[test]
    fn relayed_after_a_failure_logs_one_recovery_line_without_a_count_and_clears() {
        let (mut log, sink, t0) = log();
        log.record(t0, failed(ErrorKind::Kafka));
        log.record(t0 + Duration::from_secs(2), failed(ErrorKind::Clock));
        log.record(t0 + Duration::from_secs(4), relayed());
        log.record(t0 + Duration::from_secs(6), relayed());
        assert_eq!(
            sink.lines(),
            [
                "outbox relay kafka error: boom",
                "outbox relay clock error: boom",
                "outbox relay recovered",
            ]
        );
        // cleared: the next failure logs at once, inside the old window
        log.record(t0 + Duration::from_secs(8), failed(ErrorKind::Kafka));
        assert_eq!(sink.lines().len(), 4);
    }

    #[test]
    fn relayed_without_a_prior_failure_is_silent() {
        let (mut log, sink, t0) = log();
        log.record(t0, relayed());
        assert!(sink.lines().is_empty());
    }

    #[test]
    fn a_writer_that_always_errors_does_not_panic_and_state_still_advances() {
        let mut log = RelayLog::new(BrokenWriter, WINDOW);
        let t0 = Instant::now();
        log.record(t0, failed(ErrorKind::Database));
        assert_eq!(
            log.states[ErrorKind::Database.index()].last_logged,
            Some(t0)
        );
        log.record(t0 + Duration::from_secs(1), failed(ErrorKind::Database));
        assert_eq!(log.states[ErrorKind::Database.index()].suppressed, 1);
        log.record(t0 + Duration::from_secs(2), relayed());
        assert!(!log.states[ErrorKind::Database.index()].failed);
    }
}
