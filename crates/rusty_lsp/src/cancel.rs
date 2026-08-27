//! Cooperative request cancellation.
//!
//! When a `$/cancelRequest` arrives, the [`crate::Server`] hard-aborts the
//! handler task at its next `.await` point and answers the request with
//! `RequestCancelled` — that part needs nothing from the backend. What an
//! abort **cannot** reach is work the handler moved elsewhere: a
//! `tokio::task::spawn_blocking` computation, a helper task it spawned, or a
//! long CPU-bound stretch between `.await` points. For those, every request
//! handler runs inside a task-local [`CancelToken`] scope:
//!
//! ```rust,ignore
//! async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
//!     let token = rusty_lsp::cancel::current().unwrap_or_default();
//!     let hits = tokio::task::spawn_blocking(move || {
//!         let mut hits = Vec::new();
//!         for file in big_index {
//!             if token.is_cancelled() {
//!                 break; // stop burning CPU; the server already answered
//!             }
//!             // ... scan file ...
//!         }
//!         hits
//!     }).await??;
//!     Ok(Some(hits))
//! }
//! ```
//!
//! The token is set (and the abort issued) by the time `$/cancelRequest` is
//! processed, so `is_cancelled` flips even while the blocking work keeps
//! running. [`CancelToken::cancelled`] offers the `select!`-friendly async
//! form of the same signal.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

tokio::task_local! {
    static CURRENT: CancelToken;
}

/// A cancellation signal shared between the server loop and a request
/// handler. Cheap to clone; all clones observe the same signal.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    /// Build a fresh, uncancelled token.
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// Trip the token. Idempotent; wakes every task waiting in
    /// [`cancelled`](Self::cancelled).
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    /// Whether the token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Wait until the token is cancelled. Returns immediately if it already
    /// was.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            // Register interest before re-checking, so a `cancel` racing
            // with this call cannot slip between the check and the wait.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// The [`CancelToken`] of the request handler currently running, if the
/// caller is inside one. Returns `None` outside a request scope (e.g. in a
/// notification handler or a free-standing task) — `unwrap_or_default()`
/// gives a never-cancelled token for that case.
pub fn current() -> Option<CancelToken> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Run `future` with `token` installed as the ambient request token.
pub(crate) async fn scope<F: Future>(token: CancelToken, future: F) -> F::Output {
    CURRENT.scope(token, future).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancel_flips_flag_and_wakes_waiters() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());

        let waiter = {
            let token = token.clone();
            tokio::spawn(async move { token.cancelled().await })
        };
        token.cancel();
        assert!(token.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter wakes")
            .expect("waiter task");
    }

    #[tokio::test]
    async fn cancelled_returns_immediately_when_already_tripped() {
        let token = CancelToken::new();
        token.cancel();
        tokio::time::timeout(Duration::from_millis(50), token.cancelled())
            .await
            .expect("no wait needed");
    }

    #[tokio::test]
    async fn current_is_none_outside_a_request_scope() {
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn scope_makes_the_token_visible() {
        let token = CancelToken::new();
        let seen = scope(token.clone(), async { current().expect("in scope") }).await;
        token.cancel();
        assert!(seen.is_cancelled());
    }
}
