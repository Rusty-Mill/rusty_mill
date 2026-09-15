//! Explicit shutdown signaling (ADR-0160: engines "accept explicit...
//! shutdown policy"; `RM-ASYNC-LOAD-0002` distinguishes shutdown from
//! ordinary capacity pressure). A [`ShutdownSignal`] is a one-shot,
//! observable latch — once tripped it stays tripped — shared the same
//! way [`crate::CancellationToken`] is.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct ShutdownSignal {
    tripped: Arc<AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Trigger shutdown. Idempotent.
    pub fn trigger(&self) {
        self.tripped.store(true, Ordering::Release);
    }

    pub fn is_triggered(&self) -> bool {
        self.tripped.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_untriggered_and_latches() {
        let signal = ShutdownSignal::new();
        assert!(!signal.is_triggered());
        signal.trigger();
        assert!(signal.is_triggered());
    }
}
