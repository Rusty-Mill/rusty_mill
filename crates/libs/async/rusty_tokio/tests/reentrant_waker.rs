//! `Notify` and `oneshot` neither clone nor drop a waker while holding a
//! lock that the waker's own code could take again (#578). A waker's
//! clone, wake and drop may run executor code that re-enters the
//! primitive, such as a waker that owns the channel's `Sender`. Polled by
//! hand, each case on its own thread so a deadlock fails instead of
//! hanging.

use rusty_tokio::sync::{oneshot, Notify};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker};
use std::time::Duration;

fn poll<F: Future>(fut: Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    fut.poll(&mut Context::from_waker(waker))
}

fn bounded(f: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("finished without deadlocking or panicking");
}

/// A waker whose last drop runs `on_drop`, as a user-defined one may.
struct OnDrop<F: Fn() + Send + Sync>(F);

#[allow(
    clippy::manual_noop_waker,
    reason = "the destructor is the point; `Waker::noop` has none"
)]
impl<F: Fn() + Send + Sync> Wake for OnDrop<F> {
    fn wake(self: Arc<Self>) {}
}

impl<F: Fn() + Send + Sync> Drop for OnDrop<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn on_drop(f: impl Fn() + Send + Sync + 'static) -> Waker {
    Waker::from(Arc::new(OnDrop(f)))
}

/// A waker whose clone runs `on_clone` once armed, as a custom
/// `RawWaker` may.
struct CloneHook {
    armed: AtomicBool,
    on_clone: Box<dyn Fn() + Send + Sync>,
}

fn clone_hook(on_clone: impl Fn() + Send + Sync + 'static) -> (Arc<CloneHook>, Waker) {
    unsafe fn clone(data: *const ()) -> RawWaker {
        // SAFETY: `data` is a live `Arc<CloneHook>` from `Arc::into_raw`.
        let hook = unsafe { &*data.cast::<CloneHook>() };
        if hook.armed.load(Ordering::SeqCst) {
            (hook.on_clone)();
        }
        // SAFETY: as above; the new `RawWaker` owns one more count.
        unsafe { Arc::increment_strong_count(data.cast::<CloneHook>()) };
        RawWaker::new(data, &VTABLE)
    }
    unsafe fn release(data: *const ()) {
        // SAFETY: each `RawWaker` owns one count of the `Arc`.
        unsafe { Arc::decrement_strong_count(data.cast::<CloneHook>()) };
    }
    unsafe fn wake_by_ref(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, release, wake_by_ref, release);

    let hook = Arc::new(CloneHook {
        armed: AtomicBool::new(false),
        on_clone: Box::new(on_clone),
    });
    let raw = RawWaker::new(Arc::into_raw(hook.clone()).cast(), &VTABLE);
    // SAFETY: `VTABLE` upholds the `RawWaker` contract over the `Arc`.
    (hook, unsafe { Waker::from_raw(raw) })
}

/// A waker that owns the channel's `Sender`, so dropping it drops the
/// `Sender`, which takes the channel's lock.
fn owning_sender(tx: oneshot::Sender<u8>) -> Waker {
    let tx = Mutex::new(Some(tx));
    on_drop(move || drop(tx.lock().unwrap().take()))
}

#[test]
fn oneshot_repoll_drops_the_old_waker_unlocked() {
    bounded(|| {
        let (tx, mut rx) = oneshot::channel::<u8>();
        assert!(poll(Pin::new(&mut rx), &owning_sender(tx)).is_pending());
        // The stored waker, and with it the `Sender`, is displaced.
        assert!(poll(Pin::new(&mut rx), Waker::noop()).is_pending());
        assert_eq!(
            poll(Pin::new(&mut rx), Waker::noop()),
            Poll::Ready(Err(oneshot::RecvError))
        );
    });
}

#[test]
fn oneshot_receiver_drop_drops_the_waker_unlocked() {
    bounded(|| {
        let (tx, mut rx) = oneshot::channel::<u8>();
        assert!(poll(Pin::new(&mut rx), &owning_sender(tx)).is_pending());
        drop(rx);
    });
}

#[test]
fn oneshot_never_clones_a_waker_under_its_lock() {
    bounded(|| {
        let (tx, mut rx) = oneshot::channel::<u8>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let sender = tx.clone();
        let (hook, waker) = clone_hook(move || {
            if let Some(tx) = sender.lock().unwrap().take() {
                let _ = tx.send(9);
            }
        });
        assert!(poll(Pin::new(&mut rx), Waker::noop()).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        // Cloning the waker sends, so the poll that clones it receives.
        assert_eq!(poll(Pin::new(&mut rx), &waker), Poll::Ready(Ok(9)));
    });
}

#[test]
fn notify_repoll_drops_the_old_waker_unlocked() {
    bounded(|| {
        let notify = Arc::new(Notify::new());
        let mut waiter = Box::pin(notify.notified_owned());
        let reenter = notify.clone();
        let waker = on_drop(move || reenter.notify_waiters());
        assert!(poll(waiter.as_mut(), &waker).is_pending());
        drop(waker);
        // The displaced waker's destructor notifies this very waiter.
        let _ = poll(waiter.as_mut(), Waker::noop());
        assert!(poll(waiter.as_mut(), Waker::noop()).is_ready());
    });
}

/// A guard: the old `*lock = Some(waker.clone())` already cloned before
/// locking (an assignment evaluates its right side first); the explicit
/// clone-then-lock that replaced it must keep that order.
#[test]
fn notify_never_clones_a_waker_under_its_waiter_lock() {
    bounded(|| {
        let notify = Arc::new(Notify::new());
        let mut waiter = Box::pin(notify.notified_owned());
        let reenter = notify.clone();
        let (hook, waker) = clone_hook(move || reenter.notify_one());
        assert!(poll(waiter.as_mut(), Waker::noop()).is_pending());
        hook.armed.store(true, Ordering::SeqCst);
        // Cloning the new waker notifies this very waiter.
        let _ = poll(waiter.as_mut(), &waker);
        hook.armed.store(false, Ordering::SeqCst);
        assert!(poll(waiter.as_mut(), Waker::noop()).is_ready());
    });
}
