//! [`CancellationToken`]: tokio-util's cooperative-cancellation handle,
//! here so code written against `tokio_util::sync::CancellationToken`
//! needs no second crate (#901, gap G8).
//!
//! ## Why not built on [`Notify`](super::Notify)
//!
//! The obvious build is a flag plus `Notify::notify_waiters`. It leaks:
//! a [`Notified`](super::Notified) that is dropped unpolled-to-completion
//! stays in `Notify`'s waiter queue until the next notification, and the
//! common pattern -- `select!` on `token.cancelled()` inside a loop --
//! creates and drops one per iteration on a token that may never be
//! cancelled. So each node keeps its own waiter map instead, and a
//! dropped [`WaitForCancellationFuture`] removes its entry.
//!
//! ## The tree
//!
//! Every token handle points at a [`Node`]. A [`child_token`] node holds
//! a *strong* reference to its parent and the parent holds only a *weak*
//! one back, keyed in a map -- so there is no reference cycle, a parent
//! stays alive exactly as long as some descendant might still need to
//! unlink from it, and a child node removes its own entry when its last
//! handle (token, owned future or guard) goes away. A long-lived parent
//! that hands out short-lived children therefore holds only the live
//! ones, never an ever-growing list of dead ones.
//!
//! ## Locking
//!
//! Each node has its own lock, and no code path ever holds two: child
//! creation locks only the parent, unlinking locks only the parent, and
//! [`CancellationToken::cancel`] walks the subtree one node at a time. No
//! waker is woken or dropped and no node is dropped while a lock is held
//! (this crate has had review findings on exactly that): wakers are
//! collected while marking and woken after the whole subtree is marked,
//! which also means every task woken by a cancellation already sees every
//! descendant token cancelled.
//!
//! [`child_token`]: CancellationToken::child_token

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};

struct Node {
    /// Mirrors `State::cancelled`, set while holding the lock, so
    /// `is_cancelled` and an already-cancelled poll never lock.
    cancelled: AtomicBool,
    state: Mutex<State>,
    /// `None` for a root, and for a child created from an
    /// already-cancelled parent (it starts cancelled and has nothing to
    /// unlink from).
    parent: Option<Arc<Node>>,
    /// This node's key in its parent's `children`.
    key: u64,
}

struct State {
    cancelled: bool,
    children: HashMap<u64, Weak<Node>>,
    waiters: HashMap<u64, Waker>,
    /// Keys for both maps; never reused within one node.
    next_key: u64,
}

impl Node {
    fn new(cancelled: bool, parent: Option<Arc<Node>>, key: u64) -> Node {
        Node {
            cancelled: AtomicBool::new(cancelled),
            state: Mutex::new(State {
                cancelled,
                children: HashMap::new(),
                waiters: HashMap::new(),
                next_key: 0,
            }),
            parent,
            key,
        }
    }

    /// Nothing that can panic runs under these locks (no user code, only
    /// map operations and waker clones), so a poisoned lock holds
    /// consistent data; recover it rather than propagate a panic.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let Some(parent) = &self.parent else {
            return;
        };
        // The removed `Weak` is dropped after the lock is released. A
        // cancelled parent has already emptied its map, so this is a
        // harmless no-op then.
        let removed = parent.lock().children.remove(&self.key);
        drop(removed);
    }
}

/// A token for cooperative cancellation, as in
/// `tokio_util::sync::CancellationToken`.
///
/// [`cancel`](Self::cancel) marks this token and every token derived from
/// it with [`child_token`](Self::child_token) cancelled, and resolves
/// every [`cancelled`](Self::cancelled) future waiting on any of them.
/// Clones share state: cancelling any clone cancels them all. Cancelling
/// a child leaves its parent alone. Dropping a token never cancels it --
/// use [`drop_guard`](Self::drop_guard) for that.
#[derive(Clone)]
pub struct CancellationToken {
    node: Arc<Node>,
}

impl CancellationToken {
    /// A new, uncancelled root token.
    pub fn new() -> CancellationToken {
        CancellationToken {
            node: Arc::new(Node::new(false, None, 0)),
        }
    }

    /// A token cancelled when this one is (or any ancestor is), and
    /// otherwise independent of it: cancelling the child does not cancel
    /// this token. A child of an already-cancelled token starts
    /// cancelled.
    pub fn child_token(&self) -> CancellationToken {
        let mut state = self.node.lock();
        if state.cancelled {
            drop(state);
            return CancellationToken {
                node: Arc::new(Node::new(true, None, 0)),
            };
        }
        let key = state.next_key;
        state.next_key += 1;
        let child = Arc::new(Node::new(false, Some(self.node.clone()), key));
        state.children.insert(key, Arc::downgrade(&child));
        drop(state);
        CancellationToken { node: child }
    }

    /// Cancels this token and all its descendants, waking every task
    /// waiting on any of them. Idempotent.
    pub fn cancel(&self) {
        let mut wakers = Vec::new();
        let mut pending = vec![self.node.clone()];
        while let Some(node) = pending.pop() {
            let mut state = node.lock();
            if state.cancelled {
                continue;
            }
            state.cancelled = true;
            node.cancelled.store(true, Ordering::Release);
            wakers.extend(std::mem::take(&mut state.waiters).into_values());
            // Upgraded under the lock, but the `Arc`s live in `pending`
            // and are dropped only after it's released (a child whose
            // last handle is already gone fails to upgrade and is
            // skipped; its own drop unlinks it).
            let children = std::mem::take(&mut state.children);
            pending.extend(children.values().filter_map(Weak::upgrade));
            drop(state);
            drop(children);
        }
        for waker in wakers {
            waker.wake();
        }
    }

    /// Whether this token (or an ancestor) has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.node.cancelled.load(Ordering::Acquire)
    }

    /// Resolves once this token is cancelled -- at once if it already
    /// is. Dropping the future before then unregisters it.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        WaitForCancellationFuture {
            token: self,
            waiter: Waiter { key: None },
        }
    }

    /// Like [`cancelled`](Self::cancelled), but owns the token, so the
    /// future is `'static` (e.g. to move into a spawned task).
    pub fn cancelled_owned(self) -> WaitForCancellationFutureOwned {
        WaitForCancellationFutureOwned {
            token: self,
            waiter: Waiter { key: None },
        }
    }

    /// Wraps this token in a guard that cancels it when dropped, unless
    /// [`DropGuard::disarm`]ed first.
    pub fn drop_guard(self) -> DropGuard {
        DropGuard {
            token: self,
            armed: true,
        }
    }
}

impl Default for CancellationToken {
    fn default() -> CancellationToken {
        CancellationToken::new()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("is_cancelled", &self.is_cancelled())
            .finish()
    }
}

/// One waiting future's registration in its node's `waiters`, shared by
/// both future types.
struct Waiter {
    key: Option<u64>,
}

impl Waiter {
    fn poll(&mut self, node: &Node, cx: &mut Context<'_>) -> Poll<()> {
        if node.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        let mut state = node.lock();
        if state.cancelled {
            return Poll::Ready(());
        }
        let replaced = match self.key {
            Some(key) => match state.waiters.get_mut(&key) {
                Some(existing) if existing.will_wake(cx.waker()) => None,
                Some(existing) => Some(std::mem::replace(existing, cx.waker().clone())),
                None => state.waiters.insert(key, cx.waker().clone()),
            },
            None => {
                let key = state.next_key;
                state.next_key += 1;
                self.key = Some(key);
                state.waiters.insert(key, cx.waker().clone())
            }
        };
        drop(state);
        // A stale waker is dropped only now, with the lock released.
        drop(replaced);
        Poll::Pending
    }

    fn unregister(&mut self, node: &Node) {
        let Some(key) = self.key.take() else {
            return;
        };
        let removed = node.lock().waiters.remove(&key);
        drop(removed);
    }
}

/// The future returned by [`CancellationToken::cancelled`].
#[must_use = "futures do nothing unless polled"]
pub struct WaitForCancellationFuture<'a> {
    token: &'a CancellationToken,
    waiter: Waiter,
}

impl Future for WaitForCancellationFuture<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        this.waiter.poll(&this.token.node, cx)
    }
}

impl Drop for WaitForCancellationFuture<'_> {
    fn drop(&mut self) {
        self.waiter.unregister(&self.token.node);
    }
}

impl fmt::Debug for WaitForCancellationFuture<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitForCancellationFuture").finish()
    }
}

/// The future returned by [`CancellationToken::cancelled_owned`].
#[must_use = "futures do nothing unless polled"]
pub struct WaitForCancellationFutureOwned {
    token: CancellationToken,
    waiter: Waiter,
}

impl Future for WaitForCancellationFutureOwned {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        this.waiter.poll(&this.token.node, cx)
    }
}

impl Drop for WaitForCancellationFutureOwned {
    fn drop(&mut self) {
        self.waiter.unregister(&self.token.node);
    }
}

impl fmt::Debug for WaitForCancellationFutureOwned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitForCancellationFutureOwned").finish()
    }
}

/// Cancels its token when dropped -- see
/// [`CancellationToken::drop_guard`].
#[derive(Debug)]
pub struct DropGuard {
    token: CancellationToken,
    /// Cleared by `disarm`, which consumes the guard; a flag rather than
    /// an `Option` so getting the token back can't fail.
    armed: bool,
}

impl DropGuard {
    /// Returns the token without cancelling it.
    pub fn disarm(mut self) -> CancellationToken {
        self.armed = false;
        self.token.clone()
    }
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_count(token: &CancellationToken) -> usize {
        token.node.lock().children.len()
    }

    fn waiter_count(token: &CancellationToken) -> usize {
        token.node.lock().waiters.len()
    }

    #[test]
    fn dropped_children_unlink_from_their_parent() {
        let parent = CancellationToken::new();
        for _ in 0..1000 {
            let child = parent.child_token();
            let grandchild = child.child_token();
            drop(child);
            // The grandchild keeps the child's node (and its entry in
            // the parent) alive until it goes too.
            assert_eq!(child_count(&parent), 1);
            drop(grandchild);
        }
        assert_eq!(child_count(&parent), 0);
    }

    #[test]
    fn a_dropped_cancelled_future_unregisters_its_waker() {
        let token = CancellationToken::new();
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        for _ in 0..1000 {
            let mut fut = Box::pin(token.cancelled());
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            assert_eq!(waiter_count(&token), 1);
        }
        assert_eq!(waiter_count(&token), 0);
    }

    #[test]
    fn cancel_releases_children_and_waiters() {
        let token = CancellationToken::new();
        let _child = token.child_token();
        let waker = Waker::noop();
        let mut fut = Box::pin(token.cancelled());
        assert!(fut
            .as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_pending());
        token.cancel();
        assert_eq!(child_count(&token), 0);
        assert_eq!(waiter_count(&token), 0);
        assert!(fut
            .as_mut()
            .poll(&mut Context::from_waker(waker))
            .is_ready());
    }
}
