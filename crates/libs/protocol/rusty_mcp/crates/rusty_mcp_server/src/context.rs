//! What a running handler can see about its request.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Per-request state handed to handlers.
#[derive(Clone, Debug, Default)]
pub struct Context {
    cancelled: Arc<AtomicBool>,
}

impl Context {
    /// A context that is not cancelled.
    pub fn new() -> Self {
        Context::default()
    }

    /// The caller sent `notifications/cancelled` for this request. Long
    /// handlers should poll this and return early.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Mark the request cancelled. The transport calls this; tests may too.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}
