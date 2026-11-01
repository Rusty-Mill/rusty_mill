//! An async condition variable: [`Notify::notified`] parks until
//! [`Notify::notify_one`] or [`Notify::notify_waiters`] fires.
//!
//! Each `notified()` call gets its own [`WaiterState`], holding a
//! `woken` flag and the waker to fire once notified -- stored behind a
//! shared `Arc` (not just copied once at registration time) so
//! [`Notified::enable`]/[`OwnedNotified::enable`] can register interest
//! *before* a real waker exists yet (nothing's actually parked, waiting,
//! at that point -- `enable` just wants a concurrent `notify_one`/
//! `notify_waiters` to see this waiter and mark it, closing the race
//! between "check some condition" and "start waiting" a caller might
//! otherwise have), with the real waker filled in later once `poll` is
//! actually called and one exists.
//!
//! A poll that runs because it was woken always finds its own flag
//! already true and returns `Ready` immediately, regardless of
//! `permits` or whether anyone else registered after it. Without a
//! per-waiter flag, `notify_waiters` has a real bug: it wakes every
//! currently-registered waker but (unlike `notify_one`) leaves nothing
//! else behind for that specific waiter to see, so re-polling it
//! afterward would find `permits == 0`, register nothing new (it's
//! already marked `registered`), and return `Pending` forever -- a
//! wakeup that fires but is then silently lost on the very next poll.
//!
//! A `notify_one` with nobody currently waiting still banks a permit for
//! the next `notified()` call, so a wakeup that "arrives early" (before
//! the other side started waiting) isn't lost either.
//!
//! Dropping a `Notified` before it resolves takes it out of the queue,
//! so a later `notify_one` reaches a live waiter instead of a dead one.
//! If a `notify_one` or `notify_last` had already reached it, that wakeup
//! passes on as tokio 1.53's `Notified::drop` does: to the next waiter
//! at the same end of the queue (the oldest for `notify_one`, the newest
//! for `notify_last`), or banked as a permit if none waits. A wakeup the
//! waiter already consumed is not passed on: a permit it took, or a
//! wakeup that `poll` returned `Ready` for or `enable` returned `true`
//! for. Nor is a `notify_waiters` wakeup.
//!
//! As in tokio, a `Notified` receives `notify_waiters` wakeups from the
//! moment it is created, even if it has not been polled or enabled yet:
//! `notify_waiters` counts its calls, each waiter keeps the count it saw
//! when it was created, and a waiter that finds the count changed when it
//! first takes the queue lock is notified instead of queued.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Shared per-waiter state, so [`Notified::enable`]/
/// [`OwnedNotified::enable`] can register a waiter before any real
/// waker exists (see this module's own docs), with the real waker
/// filled in later once `poll` actually runs.
struct WaiterState {
    /// [`Inner::notify_waiters_calls`] when this waiter was created.
    notify_waiters_calls: usize,
    woken: AtomicBool,
    /// Set, under the queue lock as the waiter leaves the queue, to the
    /// [`End`] a `notify_one` or `notify_last` took it from, so dropping
    /// it unconsumed passes the wakeup on to that same end, even before
    /// `fire` has run. [`NO_END`] otherwise. Like tokio's per-waiter
    /// `Notification::One(strategy)`.
    forward: AtomicU8,
    waker: Mutex<Option<Waker>>,
}

impl WaiterState {
    fn new(notify_waiters_calls: usize) -> Arc<Self> {
        Arc::new(WaiterState {
            notify_waiters_calls,
            woken: AtomicBool::new(false),
            forward: AtomicU8::new(NO_END),
            waker: Mutex::new(None),
        })
    }
}

/// Which waiter a single wakeup goes to.
#[derive(Clone, Copy)]
#[repr(u8)]
enum End {
    Oldest = 1,
    Newest = 2,
}

/// [`WaiterState::forward`] when there is no wakeup to pass on.
const NO_END: u8 = 0;

impl End {
    fn from_u8(end: u8) -> Option<End> {
        match end {
            1 => Some(End::Oldest),
            2 => Some(End::Newest),
            _ => None,
        }
    }
}

struct Inner {
    permits: usize,
    /// How many times `notify_waiters` ran, wrapping. Like tokio 1.53's
    /// `NOTIFY_WAITERS_CALLS_MASK` bits of `Notify::state`, but guarded
    /// by this lock rather than atomic.
    notify_waiters_calls: usize,
    waiters: VecDeque<Arc<WaiterState>>,
}

pub struct Notify {
    inner: Mutex<Inner>,
}

impl Notify {
    pub fn new() -> Self {
        Notify {
            inner: Mutex::new(Inner {
                permits: 0,
                notify_waiters_calls: 0,
                waiters: VecDeque::new(),
            }),
        }
    }

    /// Identical to [`new`](Self::new), but usable in a `const` context
    /// (e.g. a `static NOTIFY: Notify = Notify::const_new();`) -- there's
    /// no lazy-initialization trick here to work around, `Mutex::new` and
    /// `VecDeque::new` are already both `const fn` themselves, so this is
    /// just [`new`](Self::new) spelled as one.
    pub const fn const_new() -> Self {
        Notify {
            inner: Mutex::new(Inner {
                permits: 0,
                notify_waiters_calls: 0,
                waiters: VecDeque::new(),
            }),
        }
    }

    /// Wakes one waiter, or -- if nobody is currently waiting -- banks a
    /// permit so the very next `notified().await` returns immediately.
    pub fn notify_one(&self) {
        self.give(End::Oldest);
    }

    /// Like [`notify_one`](Self::notify_one), but wakes the most
    /// recently registered waiter (LIFO) instead of the oldest (FIFO).
    /// Still just banks a permit, exactly like `notify_one`, if nobody
    /// is currently waiting.
    pub fn notify_last(&self) {
        self.give(End::Newest);
    }

    /// Gives one wakeup to the waiter at `end`, or banks a permit.
    fn give(&self, end: End) {
        if let Some(state) = self.take(end) {
            Self::fire(&state);
        }
    }

    /// Gives one wakeup, under the queue lock: takes the waiter at `end`
    /// out of the queue and records `end` as where the wakeup goes if the
    /// waiter is dropped unconsumed, for the caller to fire once the lock
    /// is released, or banks a permit if none waits. Recording it here,
    /// not in `fire`, means a waiter dropped before it is fired still
    /// passes the wakeup on.
    fn take(&self, end: End) -> Option<Arc<WaiterState>> {
        let mut guard = self.inner.lock().unwrap();
        let state = match end {
            End::Oldest => guard.waiters.pop_front(),
            End::Newest => guard.waiters.pop_back(),
        };
        match &state {
            Some(state) => state.forward.store(end as u8, Ordering::Relaxed),
            None => guard.permits += 1,
        }
        state
    }

    /// Wakes every task currently waiting -- each one's own `woken` flag
    /// is set first (see this module's docs for why that's required,
    /// unlike a naive port of `notify_one`'s wake-and-hope approach).
    /// That includes every `Notified` created before this call, even one
    /// not yet polled or enabled. Nothing is banked for a `notified()`
    /// call made *after* this returns; that call waits for the next
    /// notification, same as tokio's semantics.
    pub fn notify_waiters(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.notify_waiters_calls = guard.notify_waiters_calls.wrapping_add(1);
        let waiters = std::mem::take(&mut guard.waiters);
        drop(guard);
        for state in waiters {
            Self::fire(&state);
        }
    }

    /// Marks `state` notified and wakes whatever real waker (if any)
    /// has been stored there -- `None` if [`Notified::enable`]
    /// registered it but nothing has actually polled (and thus stored a
    /// real waker) yet, in which case there's nothing to wake: the next
    /// poll will see `woken` already set and return `Ready` immediately
    /// without ever needing a wakeup delivered.
    fn fire(state: &Arc<WaiterState>) {
        state.woken.store(true, Ordering::Release);
        // Taken in its own statement, so the waker lock is released
        // before `wake` runs: on edition 2021 an `if let` scrutinee's
        // guard would live through the whole block.
        let waker = state.waker.lock().unwrap().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub fn notified(&self) -> Notified<'_> {
        Notified {
            notify: self,
            registered: false,
            done: false,
            state: WaiterState::new(self.notify_waiters_calls()),
        }
    }

    /// The current `notify_waiters` call count, which a new waiter keeps
    /// to tell whether a `notify_waiters` ran after it was created.
    fn notify_waiters_calls(&self) -> usize {
        self.inner.lock().unwrap().notify_waiters_calls
    }

    /// Like [`notified`](Self::notified), but the returned future owns
    /// an `Arc` clone of this `Notify` instead of borrowing it -- usable
    /// past this `Notify`'s own lifetime, e.g. moved into a spawned
    /// task.
    pub fn notified_owned(self: &Arc<Self>) -> OwnedNotified {
        OwnedNotified {
            notify: self.clone(),
            registered: false,
            done: false,
            state: WaiterState::new(self.notify_waiters_calls()),
        }
    }
}

impl std::fmt::Debug for Notify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notify").finish_non_exhaustive()
    }
}

impl Default for Notify {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared poll body for [`Notified`]/[`OwnedNotified`] -- identical
/// either way, since both only ever need `&Notify`.
///
/// `done` is sticky: once set it stays set and short-cuts every later
/// poll to `Ready`, before the coop budget is consulted, as tokio's
/// `State::Done` does. Otherwise a poll the budget turned away could
/// clear it, and a drop would then pass on a wakeup already consumed.
fn poll_notified(
    notify: &Notify,
    registered: &mut bool,
    done: &mut bool,
    state: &Arc<WaiterState>,
    cx: &mut Context<'_>,
) -> Poll<()> {
    if *done {
        return Poll::Ready(());
    }
    let poll = poll_waiter(notify, registered, state, cx);
    *done = poll.is_ready();
    poll
}

fn poll_waiter(
    notify: &Notify,
    registered: &mut bool,
    state: &Arc<WaiterState>,
    cx: &mut Context<'_>,
) -> Poll<()> {
    if crate::coop::poll_proceed(cx).is_pending() {
        return Poll::Pending;
    }
    // Checked first, before ever touching the shared lock: this is what
    // makes re-polling after a `notify_waiters`-driven wake resolve
    // instead of registering (uselessly, since it's already
    // `registered`) and going back to sleep forever.
    if state.woken.load(Ordering::Acquire) {
        return Poll::Ready(());
    }
    let mut guard = notify.inner.lock().unwrap();
    if !*registered {
        if missed_notify_waiters(&guard, state) {
            return Poll::Ready(());
        }
        if guard.permits > 0 {
            guard.permits -= 1;
            return Poll::Ready(());
        }
        guard.waiters.push_back(state.clone());
        *registered = true;
    }
    drop(guard);
    // Always (re-)store the real waker -- whether just registered above
    // or already registered earlier (by a previous `poll` whose waker
    // may be stale, or by `enable`, which registers with no waker at
    // all yet).
    //
    // Cloned before locking, and the waker it displaces dropped after
    // unlocking (the guard is a temporary of the `let`): either may run
    // code that re-enters this `Notify`.
    let waker = cx.waker().clone();
    let displaced = state.waker.lock().unwrap().replace(waker);
    drop(displaced);
    // Re-check after storing it: a `notify_one`/`notify_waiters` call
    // may have already popped and fired this exact entry in the window
    // between the check above and now.
    if state.woken.load(Ordering::Acquire) {
        return Poll::Ready(());
    }
    Poll::Pending
}

/// Whether a `notify_waiters` ran after `state`'s waiter was created but
/// before it joined the queue, which notifies it as if it had been queued
/// (tokio 1.53 `poll_notified`, `State::Init`). Checked under the queue
/// lock, which `notify_waiters` counts under, and before a permit is
/// taken, so the permit is left for another waiter as tokio leaves it.
/// Marks the waiter woken, so later polls return `Ready` too.
fn missed_notify_waiters(guard: &Inner, state: &WaiterState) -> bool {
    if guard.notify_waiters_calls == state.notify_waiters_calls {
        return false;
    }
    state.woken.store(true, Ordering::Release);
    true
}

/// Registers `state` as a waiter (if it isn't already), *without* a
/// waker -- see this module's own docs for why that's still useful:
/// closes the race between "check some other condition" and "actually
/// start waiting" without needing anything to poll (and thus have a
/// real waker for) yet. Returns whether this waiter is already
/// notified, either because it already was before this call, or
/// because a banked permit was immediately available.
///
/// `true` consumes the wakeup, as a `Ready` poll does: it sets `done`,
/// so dropping the waiter afterwards passes nothing on. tokio's `enable`
/// moves its future to `Done` the same way.
fn enable(
    notify: &Notify,
    registered: &mut bool,
    done: &mut bool,
    state: &Arc<WaiterState>,
) -> bool {
    if !*done {
        *done = enable_waiter(notify, registered, state);
    }
    *done
}

fn enable_waiter(notify: &Notify, registered: &mut bool, state: &Arc<WaiterState>) -> bool {
    if state.woken.load(Ordering::Acquire) {
        return true;
    }
    if *registered {
        return false;
    }
    let mut guard = notify.inner.lock().unwrap();
    if missed_notify_waiters(&guard, state) {
        return true;
    }
    if guard.permits > 0 {
        guard.permits -= 1;
        // `enable` sets `done`, which makes every later poll `Ready`.
        return true;
    }
    guard.waiters.push_back(state.clone());
    *registered = true;
    false
}

/// Takes a waiter that never resolved out of `notify`'s queue, and
/// passes on a `notify_one`/`notify_last` wakeup it had already received
/// to the same end of the queue (see the module's docs).
fn release(notify: &Notify, registered: bool, done: bool, state: &Arc<WaiterState>) {
    if done || !registered {
        return;
    }
    let mut guard = notify.inner.lock().unwrap();
    if let Some(at) = guard.waiters.iter().position(|w| Arc::ptr_eq(w, state)) {
        guard.waiters.remove(at);
        return;
    }
    // Not queued: something took it out to fire it, perhaps not yet.
    let forward = End::from_u8(state.forward.load(Ordering::Relaxed));
    drop(guard);
    if let Some(end) = forward {
        notify.give(end);
    }
}

pub struct Notified<'a> {
    notify: &'a Notify,
    registered: bool,
    /// Whether `poll` returned `Ready` or `enable` returned `true`, which
    /// consumed the wakeup.
    done: bool,
    state: Arc<WaiterState>,
}

impl Drop for Notified<'_> {
    fn drop(&mut self) {
        release(self.notify, self.registered, self.done, &self.state);
    }
}

impl Notified<'_> {
    /// Registers this waiter now, without waiting for an actual
    /// `.await`/`poll` -- see this module's docs for why that's useful.
    /// Returns whether it's already notified (a banked permit, or an
    /// already-fired notification). `true` consumes that notification,
    /// as tokio's does: dropping this future afterwards passes nothing on.
    pub fn enable(self: Pin<&mut Self>) -> bool {
        let this = self.get_mut();
        enable(
            this.notify,
            &mut this.registered,
            &mut this.done,
            &this.state,
        )
    }
}

impl Future for Notified<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        poll_notified(
            this.notify,
            &mut this.registered,
            &mut this.done,
            &this.state,
            cx,
        )
    }
}

/// The [`Arc`]-owned counterpart of [`Notified`], returned by
/// [`Notify::notified_owned`].
pub struct OwnedNotified {
    notify: Arc<Notify>,
    registered: bool,
    /// Whether `poll` returned `Ready` or `enable` returned `true`, which
    /// consumed the wakeup.
    done: bool,
    state: Arc<WaiterState>,
}

impl Drop for OwnedNotified {
    fn drop(&mut self) {
        release(&self.notify, self.registered, self.done, &self.state);
    }
}

impl OwnedNotified {
    /// See [`Notified::enable`] -- identical semantics here.
    pub fn enable(self: Pin<&mut Self>) -> bool {
        let this = self.get_mut();
        enable(
            &this.notify,
            &mut this.registered,
            &mut this.done,
            &this.state,
        )
    }
}

impl Future for OwnedNotified {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        poll_notified(
            &this.notify,
            &mut this.registered,
            &mut this.done,
            &this.state,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::task::Waker;

    use super::*;

    fn poll(future: Pin<&mut impl Future<Output = ()>>) -> Poll<()> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn a_waiter_dropped_unfired_does_not_swallow_a_notify_one() {
        let notify = Notify::new();
        {
            let mut stale = pin!(notify.notified());
            assert!(poll(stale.as_mut()).is_pending());
        }
        let mut live = pin!(notify.notified());
        assert!(poll(live.as_mut()).is_pending());
        notify.notify_one();
        assert!(poll(live.as_mut()).is_ready());
    }

    #[test]
    fn a_notify_one_reaching_a_dropped_waiter_passes_to_the_next() {
        let notify = Notify::new();
        let mut first = Box::pin(notify.notified());
        let mut second = pin!(notify.notified());
        assert!(poll(first.as_mut()).is_pending());
        assert!(poll(second.as_mut()).is_pending());
        notify.notify_one();
        drop(first);
        assert!(poll(second.as_mut()).is_ready());
    }

    #[test]
    fn a_permit_taken_by_enable_stays_taken_when_dropped() {
        let notify = Notify::new();
        notify.notify_one();
        {
            let enabled = pin!(notify.notified());
            assert!(enabled.enable());
        }
        assert!(poll(pin!(notify.notified())).is_pending());
    }

    #[test]
    fn a_consumed_wakeup_and_a_notify_waiters_one_are_not_passed_on() {
        let notify = Notify::new();
        {
            let mut done = pin!(notify.notified());
            assert!(poll(done.as_mut()).is_pending());
            notify.notify_one();
            assert!(poll(done.as_mut()).is_ready());
        }
        {
            let mut broadcast = pin!(notify.notified());
            assert!(poll(broadcast.as_mut()).is_pending());
            notify.notify_waiters();
        }
        assert!(poll(pin!(notify.notified())).is_pending());
    }

    // tokio guarantees a `Notified` receives `notify_waiters` wakeups
    // from the moment it is created, polled or not (#576).

    #[test]
    fn notify_waiters_reaches_a_created_unpolled_notified() {
        let notify = Notify::new();
        let mut created = pin!(notify.notified());
        notify.notify_waiters();
        assert!(poll(created.as_mut()).is_ready());
    }

    #[test]
    fn notify_waiters_reaches_a_created_unpolled_owned_notified() {
        let notify = Arc::new(Notify::new());
        let mut created = pin!(notify.notified_owned());
        notify.notify_waiters();
        assert!(poll(created.as_mut()).is_ready());
    }

    #[test]
    fn enable_after_notify_waiters_reports_notified() {
        let notify = Notify::new();
        let mut created = pin!(notify.notified());
        notify.notify_waiters();
        assert!(created.as_mut().enable());
        assert!(poll(created.as_mut()).is_ready());
    }

    /// A waiter woken by `notify_waiters` leaves a banked permit for the
    /// next waiter, as tokio's `poll_notified` checks the call count
    /// before it takes a permit.
    #[test]
    fn a_notify_waiters_wakeup_leaves_the_banked_permit() {
        let notify = Notify::new();
        notify.notify_one();
        let mut created = pin!(notify.notified());
        notify.notify_waiters();
        assert!(poll(created.as_mut()).is_ready());
        assert!(poll(pin!(notify.notified())).is_ready(), "permit left");
    }

    #[test]
    fn a_notified_created_after_notify_waiters_still_waits() {
        let notify = Notify::new();
        notify.notify_waiters();
        let mut later = pin!(notify.notified());
        assert!(poll(later.as_mut()).is_pending());
        assert!(!later.as_mut().enable());
    }

    // A `notify_waiters` wakeup a created waiter missed (#1322) is
    // consumed like any other (#1320): sticky through a poll the coop
    // budget turns away, and never passed on or charged to a permit.

    #[test]
    fn a_missed_notify_waiters_wakeup_survives_a_poll_with_no_budget() {
        let notify = Notify::new();
        notify.notify_one();
        let mut created = pin!(notify.notified());
        notify.notify_waiters();
        assert!(poll(created.as_mut()).is_ready());
        assert!(poll_without_budget(created.as_mut()).is_ready());
        assert!(poll(pin!(notify.notified())).is_ready(), "permit left");
        assert!(poll(pin!(notify.notified())).is_pending(), "only one");
    }

    #[test]
    fn a_missed_notify_waiters_wakeup_enable_reported_leaves_the_permit() {
        let notify = Notify::new();
        notify.notify_one();
        {
            let mut created = pin!(notify.notified());
            notify.notify_waiters();
            assert!(created.as_mut().enable());
            assert!(created.as_mut().enable(), "still reported");
            assert!(poll_without_budget(created.as_mut()).is_ready());
        }
        assert!(poll(pin!(notify.notified())).is_ready(), "permit left");
        assert!(poll(pin!(notify.notified())).is_pending(), "only one");
    }

    /// Dropping a created waiter that missed a `notify_waiters` passes
    /// nothing on: the queued waiter still waits for its own wakeup.
    #[test]
    fn a_missed_notify_waiters_wakeup_is_not_passed_on_when_dropped() {
        let notify = Arc::new(Notify::new());
        let mut queued = Box::pin(notify.notified());
        let created = Box::pin(notify.notified());
        notify.notify_waiters();
        assert!(poll(queued.as_mut()).is_ready());
        let mut next = Box::pin(notify.notified());
        assert!(!next.as_mut().enable());
        drop(created);
        assert!(poll(next.as_mut()).is_pending());
    }

    /// Two waiters, registered by `enable` as tokio's borrowed and owned
    /// futures both allow.
    fn two(notify: &Arc<Notify>) -> (Pin<Box<OwnedNotified>>, Pin<Box<Notified<'_>>>) {
        let mut first = Box::pin(notify.notified_owned());
        let mut second = Box::pin(notify.notified());
        assert!(!first.as_mut().enable());
        assert!(!second.as_mut().enable());
        (first, second)
    }

    // Each case below drops a waiter in the window between `take` (under
    // the lock) and `fire` (after it), as a racing thread could.

    #[test]
    fn a_waiter_dropped_before_notify_one_fires_it_passes_it_on() {
        let notify = Arc::new(Notify::new());
        let (first, mut second) = two(&notify);
        let taken = notify.take(End::Oldest).expect("the first waiter");
        drop(first);
        Notify::fire(&taken);
        assert!(poll(second.as_mut()).is_ready());
    }

    #[test]
    fn a_waiter_dropped_before_notify_last_fires_it_passes_it_on() {
        let notify = Arc::new(Notify::new());
        let mut first = Box::pin(notify.notified());
        let mut last = Box::pin(notify.notified_owned());
        assert!(!first.as_mut().enable());
        assert!(!last.as_mut().enable());
        let taken = notify.take(End::Newest).expect("the last waiter");
        drop(last);
        Notify::fire(&taken);
        assert!(poll(first.as_mut()).is_ready());
    }

    /// A `notify_last` wakeup passes to the next-newest waiter, not the
    /// oldest, as tokio's does (#1303).
    #[test]
    fn a_notify_last_dropped_before_it_fires_passes_to_the_next_newest() {
        let notify = Arc::new(Notify::new());
        let (mut oldest, mut middle) = two(&notify);
        let mut newest = Box::pin(notify.notified());
        assert!(!newest.as_mut().enable());
        let taken = notify.take(End::Newest).expect("the newest waiter");
        drop(newest);
        Notify::fire(&taken);
        assert!(poll(middle.as_mut()).is_ready());
        assert!(poll(oldest.as_mut()).is_pending());
    }

    /// `enable` returning `true` consumed the wakeup, as tokio's does.
    #[test]
    fn a_wakeup_enable_reported_is_not_passed_on() {
        let notify = Arc::new(Notify::new());
        let (mut first, mut second) = two(&notify);
        notify.notify_one();
        assert!(first.as_mut().enable());
        assert!(first.as_mut().enable(), "still reported");
        drop(first);
        assert!(poll(second.as_mut()).is_pending());
    }

    /// Polls `future` inside a task budget that is already spent, so
    /// `coop::poll_proceed` would return `Pending`.
    fn poll_without_budget(future: Pin<&mut impl Future<Output = ()>>) -> Poll<()> {
        crate::coop::budget(|| {
            let mut cx = Context::from_waker(Waker::noop());
            while crate::coop::has_budget_remaining() {
                assert!(crate::coop::poll_proceed(&mut cx).is_ready());
            }
            future.poll(&mut cx)
        })
    }

    /// A wakeup `enable` reported stays consumed through a later poll
    /// that the coop budget would have turned away, as tokio's `Done`
    /// state does.
    #[test]
    fn a_wakeup_enable_reported_survives_a_poll_with_no_budget() {
        let notify = Arc::new(Notify::new());
        let (mut first, mut second) = two(&notify);
        notify.notify_one();
        assert!(first.as_mut().enable());
        assert!(poll_without_budget(first.as_mut()).is_ready());
        drop(first);
        assert!(poll(second.as_mut()).is_pending());
    }

    /// The same for a future that already returned `Ready` and is polled
    /// again with no budget.
    #[test]
    fn a_ready_waiter_repolled_with_no_budget_stays_ready() {
        let notify = Arc::new(Notify::new());
        let (mut first, mut second) = two(&notify);
        notify.notify_one();
        assert!(poll(first.as_mut()).is_ready());
        assert!(poll_without_budget(first.as_mut()).is_ready());
        drop(first);
        assert!(poll(second.as_mut()).is_pending());
    }

    /// A future that took a permit and returned `Ready` stays `Ready`
    /// when polled again, without taking a second permit.
    #[test]
    fn a_ready_waiter_repolled_does_not_take_another_permit() {
        let notify = Notify::new();
        let mut waiter = pin!(notify.notified());
        notify.notify_one();
        assert!(poll(waiter.as_mut()).is_ready());
        notify.notify_one();
        assert!(poll(waiter.as_mut()).is_ready());
        assert!(poll(pin!(notify.notified())).is_ready(), "still banked");
    }

    /// Passing a wakeup on goes through `give`, so the window above is
    /// covered for it too; a chain of drops passes it along.
    #[test]
    fn a_wakeup_passed_on_passes_on_again() {
        let notify = Arc::new(Notify::new());
        let (first, second) = two(&notify);
        let mut third = Box::pin(notify.notified());
        assert!(!third.as_mut().enable());
        notify.notify_one();
        drop(first);
        drop(second);
        assert!(poll(third.as_mut()).is_ready());
    }

    #[test]
    fn the_only_waiter_dropped_before_its_wakeup_fires_banks_it() {
        let notify = Notify::new();
        let mut only = Box::pin(notify.notified());
        assert!(!only.as_mut().enable());
        let taken = notify.take(End::Oldest).expect("the waiter");
        drop(only);
        Notify::fire(&taken);
        assert!(poll(pin!(notify.notified())).is_ready());
        assert!(poll(pin!(notify.notified())).is_pending(), "banked once");
    }
}
