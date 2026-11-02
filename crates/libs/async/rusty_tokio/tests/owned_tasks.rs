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
/// task is cancelled as soon as its poll returns.
#[test]
fn a_task_mid_poll_at_shutdown_background_is_cancelled_after_its_poll() {
    let rt = Runtime::new().unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let handle = rt.spawn(async move {
        let mut first = true;
        poll_fn(|_| {
            if first {
                first = false;
                started_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(200));
            }
            Poll::<()>::Pending
        })
        .await;
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    rt.shutdown_background();
    assert_cancelled(join_elsewhere(handle));
}
