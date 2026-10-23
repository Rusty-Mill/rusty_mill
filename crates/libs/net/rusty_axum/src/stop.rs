//! The server's stop signal: level-triggered, and a waiter that is dropped
//! before the signal (work won the race) leaves nothing behind.
//!
//! The server waits on it for every accept, request head and streamed
//! chunk. `rusty_tokio`'s `watch::Receiver::changed` keeps the waker of a
//! cancelled wait until the next send, which here would grow with traffic
//! for the life of the server (#934 review), so this keeps its own table
//! and removes a waiter's entry when the waiter is dropped.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

#[derive(Default)]
pub(crate) struct Stop {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    stopped: bool,
    next_id: u64,
    waiters: HashMap<u64, Waker>,
}

impl Stop {
    fn state(&self) -> MutexGuard<'_, State> {
        // No code panics while holding the lock, and the state stays
        // consistent if one did, so a poisoned lock is still usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Signals stop and wakes every waiter. Idempotent.
    pub(crate) fn stop(&self) {
        let waiters = {
            let mut state = self.state();
            state.stopped = true;
            std::mem::take(&mut state.waiters)
        };
        waiters.into_values().for_each(Waker::wake);
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.state().stopped
    }

    /// Completes once stopped, at once if already stopped.
    pub(crate) fn wait(self: &Arc<Self>) -> Wait {
        Wait {
            stop: self.clone(),
            id: None,
        }
    }

    /// Waiters registered now; for tests.
    #[cfg(test)]
    pub(crate) fn waiters(&self) -> usize {
        self.state().waiters.len()
    }
}

/// See [`Stop::wait`].
pub(crate) struct Wait {
    stop: Arc<Stop>,
    id: Option<u64>,
}

impl Future for Wait {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let mut state = this.stop.state();
        if state.stopped {
            return Poll::Ready(());
        }
        let id = *this.id.get_or_insert_with(|| {
            state.next_id += 1;
            state.next_id
        });
        // Replaced on every poll, so the latest waker is the one woken.
        state.waiters.insert(id, cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.stop.state().waiters.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(wait: &mut Wait) -> Poll<()> {
        Pin::new(wait).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn a_dropped_waiter_leaves_nothing() {
        let stop = Arc::new(Stop::default());
        for _ in 0..1000 {
            let mut wait = stop.wait();
            assert!(poll(&mut wait).is_pending());
            assert!(poll(&mut wait).is_pending());
        }
        assert_eq!(stop.waiters(), 0);
    }

    #[test]
    fn stop_is_level_triggered() {
        let stop = Arc::new(Stop::default());
        let mut early = stop.wait();
        assert!(poll(&mut early).is_pending());
        stop.stop();
        assert!(poll(&mut early).is_ready());
        assert!(
            poll(&mut stop.wait()).is_ready(),
            "a later waiter sees it too"
        );
        assert!(stop.is_stopped());
        assert_eq!(stop.waiters(), 0);
    }
}
