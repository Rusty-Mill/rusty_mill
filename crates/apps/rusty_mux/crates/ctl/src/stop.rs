//! When a command gives up waiting on the server.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a bounded connection blocks in one read or write before it
/// checks its [`Stop`] again (in one read only, on Windows).
pub(crate) const POLL: Duration = Duration::from_millis(50);

/// A deadline and a cancellation check for a command's socket I/O. The
/// default never stops, and its I/O blocks without polling.
#[derive(Clone, Default)]
pub struct Stop {
    deadline: Option<Instant>,
    cancelled: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl Stop {
    /// Stops `timeout` from now; one too large to represent never stops.
    pub fn after(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now().checked_add(timeout),
            cancelled: None,
        }
    }

    /// Also stops once `cancelled` returns true.
    pub fn or_when(self, cancelled: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            cancelled: Some(Arc::new(cancelled)),
            ..self
        }
    }

    /// Whether I/O must poll, so a stop is noticed while the server is silent.
    pub(crate) fn is_bounded(&self) -> bool {
        self.deadline.is_some() || self.cancelled.is_some()
    }

    /// `Interrupted` once cancelled, `TimedOut` past the deadline.
    pub(crate) fn check(&self) -> io::Result<()> {
        if self.cancelled.as_ref().is_some_and(|cancelled| cancelled()) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "no answer before the deadline",
            ));
        }
        Ok(())
    }
}

/// Whether an I/O error only means a bounded wait ran out, or a signal
/// arrived, so the operation is retried after a [`Stop::check`].
pub(crate) fn is_wait(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn the_default_never_stops() {
        let stop = Stop::default();
        assert!(!stop.is_bounded());
        assert!(stop.check().is_ok());
    }

    #[test]
    fn a_passed_deadline_times_out() {
        let stop = Stop::after(Duration::ZERO);
        assert!(stop.is_bounded());
        let err = stop.check().expect_err("past the deadline");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(Stop::after(Duration::from_secs(60)).check().is_ok());
        assert!(Stop::after(Duration::MAX).check().is_ok());
    }

    #[test]
    fn cancelling_interrupts_before_the_deadline() {
        let flag = Arc::new(AtomicBool::new(false));
        let raised = Arc::clone(&flag);
        let stop =
            Stop::after(Duration::from_secs(60)).or_when(move || raised.load(Ordering::SeqCst));
        assert!(stop.check().is_ok());
        flag.store(true, Ordering::SeqCst);
        let err = stop.check().expect_err("cancelled");
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }
}
