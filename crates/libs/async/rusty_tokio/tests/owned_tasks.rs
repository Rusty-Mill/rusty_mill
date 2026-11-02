//! The runtime owns every task it spawns until the task finishes or the
//! runtime shuts down (card #405), as tokio does:
//! - a task that is pending with no waker stored anywhere stays alive,
//!   and so does everything its future holds;
//! - shutdown cancels every task still alive, so its `JoinHandle`
//!   resolves to a cancelled `JoinError` instead of hanging;
//! - a spawn after shutdown is cancelled straight away.

use rusty_tokio::runtime::{Builder, Runtime};
use rusty_tokio::task::{JoinError, JoinHandle};
use std::future::{pending, poll_fn};
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::time::Duration;

/// Awaits `handle` on a fresh runtime, giving up after 5 s: the runtime
/// that spawned it is gone, so this only resolves if shutdown finished
/// the join.
fn join_elsewhere<T: Send + 'static>(handle: JoinHandle<T>) -> Result<T, JoinError> {
    Runtime::new()
        .unwrap()
        .block_on(async { rusty_tokio::time::timeout(Duration::from_secs(5), handle).await })
        .expect("the JoinHandle hangs instead of resolving after shutdown")
}

/// A future that is pending forever and never stores the waker it is
/// given, so nothing but the runtime keeps its task alive.
async fn wakerless() {
    poll_fn(|_| Poll::<()>::Pending).await;
}

/// A value for a task's future to hold, and a probe that sees whether
/// the future (and so the value) has been dropped.
fn probe() -> (Arc<()>, Weak<()>) {
    let keep = Arc::new(());
    let probe = Arc::downgrade(&keep);
    (keep, probe)
}

fn assert_cancelled<T: std::fmt::Debug>(result: Result<T, JoinError>) {
    let err = result.expect_err("a task cancelled at shutdown has no output");
    assert!(
        err.is_cancelled(),
        "expected a cancelled JoinError, got {err:?}"
    );
}

fn runtimes() -> [(&'static str, Runtime); 2] {
    [
        ("multi-thread", Runtime::new().unwrap()),
        (
            "current-thread",
            Builder::new_current_thread().enable_all().build().unwrap(),
        ),
    ]
}

#[test]
fn a_wakerless_pending_task_stays_alive_until_shutdown() {
    for (flavor, rt) in runtimes() {
        let (keep, probe) = probe();
        let handle = rt.spawn(async move {
            let _keep = keep;
            wakerless().await;
        });
        rt.block_on(async { rusty_tokio::time::sleep(Duration::from_millis(100)).await });
        assert!(
            probe.upgrade().is_some(),
            "{flavor}: the task's future was dropped while it was still alive"
        );
        assert_eq!(rt.metrics().num_alive_tasks(), 1, "{flavor}");

        drop(rt);
        assert!(
            probe.upgrade().is_none(),
            "{flavor}: shutdown drops the task's future"
        );
        assert_cancelled(join_elsewhere(handle));
    }
}

#[test]
fn a_task_never_polled_before_shutdown_is_cancelled() {
    for (flavor, rt) in runtimes() {
        // The current-thread runtime only runs tasks inside block_on, so
        // this one is still sitting in the run queue at shutdown.
        let handle = rt.spawn(async {});
        let metrics = rt.metrics();
        drop(rt);
        assert_eq!(metrics.num_alive_tasks(), 0, "{flavor}");
        let result = join_elsewhere(handle);
        if flavor == "current-thread" {
            assert_cancelled(result);
        }
    }
}

#[test]
fn a_task_pending_on_a_live_waker_is_cancelled_at_shutdown() {
    for (flavor, rt) in runtimes() {
        let handle = rt.spawn(pending::<()>());
        rt.block_on(async { rusty_tokio::time::sleep(Duration::from_millis(50)).await });
        let metrics = rt.metrics();
        drop(rt);
        assert_eq!(metrics.num_alive_tasks(), 0, "{flavor}");
        assert_cancelled(join_elsewhere(handle));
    }
}

#[test]
fn spawning_after_shutdown_returns_a_cancelled_handle() {
    for (flavor, rt) in runtimes() {
        let handle = rt.handle().clone();
        drop(rt);
        let (keep, probe) = probe();
        let joined = join_elsewhere(handle.spawn(async move {
            let _keep = keep;
        }));
        assert_cancelled(joined);
        assert!(
            probe.upgrade().is_none(),
            "{flavor}: the never-run future is dropped, not leaked"
        );
    }
}

/// `shutdown_background` doesn't wait for a task that is mid-poll; that
/// task is cancelled as soon as its poll returns. The poll is held open
/// until `shutdown_background` has returned, so the cancel always comes
/// from the poller, not from shutdown finding the task idle.
#[test]
fn a_task_mid_poll_at_shutdown_background_is_cancelled_after_its_poll() {
    let rt = Runtime::new().unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    // In a Mutex only so the future is Send.
    let release_rx = std::sync::Mutex::new(release_rx);
    let handle = rt.spawn(async move {
        let mut first = true;
        poll_fn(|_| {
            if first {
                first = false;
                started_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }
            Poll::<()>::Pending
        })
        .await;
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let metrics = rt.metrics();
    rt.shutdown_background();
    assert_eq!(metrics.num_alive_tasks(), 1, "still mid-poll");
    release_tx.send(()).unwrap();
    assert_cancelled(join_elsewhere(handle));
}

#[test]
fn shutdown_timeout_cancels_what_is_left_after_its_wait() {
    let rt = Runtime::new().unwrap();
    let (keep, probe) = probe();
    let handle = rt.spawn(async move {
        let _keep = keep;
        wakerless().await;
    });
    let metrics = rt.metrics();
    rt.shutdown_timeout(Duration::from_millis(50));
    assert!(probe.upgrade().is_none(), "the future is dropped");
    assert_eq!(metrics.num_alive_tasks(), 0);
    assert_cancelled(join_elsewhere(handle));
}

/// Futures are dropped inside the runtime's context, so drop code can
/// still reach the runtime; a spawn from there is cancelled at once.
#[test]
fn a_future_dropped_at_shutdown_can_still_reach_its_runtime() {
    struct SpawnOnDrop(std::sync::mpsc::Sender<JoinHandle<()>>);
    impl Drop for SpawnOnDrop {
        fn drop(&mut self) {
            let late = rusty_tokio::runtime::Handle::current().spawn(async {});
            let _ = self.0.send(late);
        }
    }

    for (flavor, rt) in runtimes() {
        let (tx, rx) = std::sync::mpsc::channel();
        let guard = SpawnOnDrop(tx);
        let handle = rt.spawn(async move {
            let _guard = guard;
            wakerless().await;
        });
        let metrics = rt.metrics();
        drop(rt);
        let late = rx.try_recv().expect("the future was dropped at shutdown");
        assert_cancelled(join_elsewhere(late));
        assert_cancelled(join_elsewhere(handle));
        assert_eq!(metrics.num_alive_tasks(), 0, "{flavor}");
    }
}

#[cfg(feature = "thread-per-core")]
#[test]
fn every_core_cancels_its_tasks_at_shutdown() {
    let rt = Builder::new_thread_per_core()
        .worker_threads(2)
        .build()
        .unwrap();
    let (keep, probe) = probe();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let keep = keep.clone();
            rt.spawn(async move {
                let _keep = keep;
                wakerless().await;
            })
        })
        .collect();
    drop(keep);
    drop(rt);
    assert!(probe.upgrade().is_none(), "every core dropped its futures");
    for handle in handles {
        assert_cancelled(join_elsewhere(handle));
    }
}

/// Panics when dropped.
struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("PanicOnDrop");
    }
}

/// Spawns a wakerless task that holds `probe`'s value, for checking that
/// it is still cancelled after another task's cancellation panicked.
fn spawn_survivor(rt: &Runtime) -> (JoinHandle<()>, Weak<()>) {
    let (keep, probe) = probe();
    let handle = rt.spawn(async move {
        let _keep = keep;
        wakerless().await;
    });
    (handle, probe)
}

/// A future whose destructor panics while shutdown cancels it: its
/// handle reports that panic (it is delivered there, as on tokio, not
/// raised out of shutdown), every other task is still cancelled and the
/// alive count still reaches zero.
#[test]
fn a_panicking_future_destructor_does_not_stop_the_shutdown_sweep() {
    for (flavor, rt) in runtimes() {
        let bomb = PanicOnDrop;
        let panicky = rt.spawn(async move {
            let _bomb = bomb;
            wakerless().await;
        });
        let (survivor, probe) = spawn_survivor(&rt);
        rt.block_on(async { rusty_tokio::time::sleep(Duration::from_millis(50)).await });
        let metrics = rt.metrics();

        drop(rt);
        assert!(
            probe.upgrade().is_none(),
            "{flavor}: the other task was cancelled"
        );
        assert_eq!(metrics.num_alive_tasks(), 0, "{flavor}");
        assert_cancelled(join_elsewhere(survivor));
        let err = join_elsewhere(panicky).expect_err("no output");
        assert!(
            err.is_panic(),
            "{flavor}: expected the drop panic, got {err:?}"
        );
    }
}

/// A join waker that panics when shutdown resolves its handle.
struct PanicWake;

impl std::task::Wake for PanicWake {
    fn wake(self: Arc<Self>) {
        panic!("PanicWake");
    }
}

#[test]
fn a_panicking_join_waker_does_not_stop_the_shutdown_sweep() {
    use std::future::Future;
    use std::task::{Context, Waker};

    for (flavor, rt) in runtimes() {
        let mut watched = rt.spawn(wakerless());
        let (survivor, probe) = spawn_survivor(&rt);
        rt.block_on(async { rusty_tokio::time::sleep(Duration::from_millis(50)).await });
        // Register the panicking waker as the handle's join waker.
        let waker = Waker::from(Arc::new(PanicWake));
        assert!(std::pin::Pin::new(&mut watched)
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
        let metrics = rt.metrics();

        let shutdown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(rt)));
        assert!(
            shutdown.is_err(),
            "{flavor}: the waker's panic is re-raised"
        );
        assert!(
            probe.upgrade().is_none(),
            "{flavor}: the other task was cancelled"
        );
        assert_eq!(metrics.num_alive_tasks(), 0, "{flavor}");
        assert_cancelled(join_elsewhere(survivor));
        assert_cancelled(join_elsewhere(watched));
    }
}

/// A panicking join waker on one core doesn't stop the other cores from
/// being swept: every core is closed and cancelled before the panic is
/// re-raised.
#[cfg(feature = "thread-per-core")]
#[test]
fn a_panicking_join_waker_on_one_core_does_not_skip_the_others() {
    use std::future::Future;
    use std::task::{Context, Waker};

    let rt = Builder::new_thread_per_core()
        .worker_threads(2)
        .build()
        .unwrap();
    let mut watched = rt.core_handle(0).spawn(wakerless());
    let (keep, probe) = probe();
    let survivor = rt.core_handle(1).spawn(async move {
        let _keep = keep;
        wakerless().await;
    });
    let core1 = rt.core_handle(1).metrics();
    std::thread::sleep(Duration::from_millis(50));
    let waker = Waker::from(Arc::new(PanicWake));
    assert!(std::pin::Pin::new(&mut watched)
        .poll(&mut Context::from_waker(&waker))
        .is_pending());

    let shutdown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(rt)));
    assert!(shutdown.is_err(), "the waker's panic is re-raised");
    assert!(probe.upgrade().is_none(), "core 1's task was cancelled");
    assert_eq!(core1.num_alive_tasks(), 0);
    assert_cancelled(join_elsewhere(survivor));
    assert_cancelled(join_elsewhere(watched));
}

/// A runtime kept in a thread-local can be dropped after the runtime's
/// own ambient-context thread-local is gone (thread-locals are destroyed
/// in reverse order of first use). Shutdown must still cancel its tasks
/// rather than panic, which would abort the process from a thread-local
/// destructor.
#[test]
fn a_runtime_dropped_during_thread_local_teardown_still_cancels_its_tasks() {
    use std::cell::RefCell;

    thread_local! {
        static RT: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    }

    let (keep, probe) = probe();
    let handle = std::thread::spawn(move || {
        // `RT` is touched first, so it is destroyed last: after the
        // ambient context that `block_on` below touches.
        RT.with(|slot| {
            let rt = Builder::new_current_thread().enable_all().build().unwrap();
            let handle = rt.spawn(async move {
                let _keep = keep;
                wakerless().await;
            });
            rt.block_on(async { rusty_tokio::task::yield_now().await });
            *slot.borrow_mut() = Some(rt);
            handle
        })
    })
    .join()
    .expect("the runtime's thread exits cleanly");
    assert!(probe.upgrade().is_none(), "the task's future was dropped");
    assert_cancelled(join_elsewhere(handle));
}
