//! [`WakerSlot`]: the latest waker of a future whose completion is
//! signalled from another thread (design review 4).
//!
//! A future that hands a *clone of its first waker* to a helper thread
//! wakes the wrong task once it is polled again from a different context
//! (moved between executors or tasks, or re-wrapped by a combinator): the
//! current poller then never learns it is ready. The slot is updated on
//! every poll and woken from the helper thread instead.
//!
//! Ordering, which keeps a wake from being lost: the poller calls
//! [`WakerSlot::update`] *before* checking its completion flag; the
//! signalling thread records completion *before* calling
//! [`WakerSlot::wake`]. Either the thread wakes the new waker, or the
//! poller's check sees the completion.

use std::sync::{Arc, Mutex};
use std::task::Waker;

/// Shared, replaceable waker; cheap to clone (one `Arc`).
#[derive(Clone, Default)]
pub struct WakerSlot(Arc<Mutex<Option<Waker>>>);

impl WakerSlot {
    /// An empty slot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Store `waker` unless the slot already holds one that wakes the same
    /// task.
    pub fn update(&self, waker: &Waker) {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !slot.as_ref().is_some_and(|w| w.will_wake(waker)) {
            *slot = Some(waker.clone());
        }
    }

    /// Wake the stored waker, if any.
    pub fn wake(&self) {
        let waker = self.0.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl std::fmt::Debug for WakerSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WakerSlot").finish_non_exhaustive()
    }
}
