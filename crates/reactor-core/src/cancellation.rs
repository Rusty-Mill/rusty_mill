//! Explicit, disclosed cancellation (`RM-ASYNC-CANCEL-0001`: cancellation
//! races with ordinary completion and yields one terminal result). This
//! type is the caller-supplied cancellation input ADR-0160 requires
//! engines to accept explicitly, rather than each engine inventing its
//! own ad hoc mechanism.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A cheaply cloneable, shared cancellation flag. Cancelling any clone
/// cancels all of them — this is the point: one token is handed to both
/// the caller (who may cancel) and the engine (who observes it).
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent — cancelling an already-
    /// cancelled token is a no-op, not an error.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_uncancelled_and_latches() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn clones_share_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }
}
