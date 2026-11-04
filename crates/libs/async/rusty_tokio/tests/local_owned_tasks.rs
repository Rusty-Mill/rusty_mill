//! A `LocalSet` owns every task spawned on it until the task finishes or
//! the set drops (card #591), as `Runtime` does since #405:
//! - a `spawn_local` task pending with no waker stored anywhere stays
//!   alive, and so does everything its future holds;
//! - dropping the set cancels every task still alive, so its
//!   `JoinHandle` resolves to a cancelled `JoinError` instead of hanging;
//! - a spawn while the set is dropping is cancelled straight away.

use rusty_tokio::task::{spawn_local, yield_now, JoinError, JoinHandle, LocalSet};
use rusty_tokio::{Builder, LocalOptions};
use std::cell::RefCell;
use std::future::{poll_fn, Future};
use std::pin::pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

/// A future that is pending forever and never stores the waker it is
/// given, so nothing but the set keeps its task alive.
async fn wakerless() {
    poll_fn(|_| Poll::<()>::Pending).await;
}

/// A value for a task's future to hold, and a probe that sees whether
/// the future (and so the value) has been dropped.
fn probe() -> (Rc<()>, Weak<()>) {
    let keep = Rc::new(());
    let probe = Rc::downgrade(&keep);
    (keep, probe)
}

/// Polls `handle` once. The set that spawned it is gone, so only a join
/// that was already resolved can be ready.
fn poll_once<T>(handle: JoinHandle<T>) -> Result<T, JoinError> {
    let mut handle = pin!(handle);
    match handle
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("the JoinHandle hangs instead of resolving when its set drops"),
    }
}

fn assert_cancelled<T: std::fmt::Debug>(result: Result<T, JoinError>) {
    let err = result.expect_err("a task cancelled with its set has no output");
    assert!(
        err.is_cancelled(),
        "expected a cancelled JoinError, got {err:?}"
    );
}

/// Runs `local` long enough for every queued task to be polled.
fn settle(local: &LocalSet) {
    local.run_until(async {
        for _ in 0..10 {
            yield_now().await;
        }
    });
}

#[test]
fn a_wakerless_pending_task_stays_alive_until_the_set_drops() {
    let local = LocalSet::new();
    let (keep, probe) = probe();
    let handle = local.spawn_local(async move {
        let _keep = keep;
        wakerless().await;
    });
    settle(&local);
    assert!(
        probe.upgrade().is_some(),
        "the task's future was dropped while it was still alive"
    );

    drop(local);
    assert!(
        probe.upgrade().is_none(),
        "dropping the set drops the future"
    );
    assert_cancelled(poll_once(handle));
}

#[test]
fn a_task_never_polled_is_cancelled_when_the_set_drops() {
    let local = LocalSet::new();
    let (keep, probe) = probe();
    let handle = local.spawn_local(async move {
        let _keep = keep;
    });
    drop(local);
    assert!(probe.upgrade().is_none());
    assert_cancelled(poll_once(handle));
}

#[test]
fn a_finished_task_keeps_its_output() {
    let local = LocalSet::new();
    let handle = local.spawn_local(async { 7 });
    settle(&local);
    drop(local);
    assert_eq!(poll_once(handle).unwrap(), 7);
}

/// Spawns, from its destructor, a task holding `keep` onto the ambient
/// set, and stores that task's handle in `slot`.
struct SpawnOnDrop {
    keep: Option<Rc<()>>,
    slot: Rc<RefCell<Option<JoinHandle<()>>>>,
}

impl Drop for SpawnOnDrop {
    fn drop(&mut self) {
        let keep = self.keep.take();
        *self.slot.borrow_mut() = Some(spawn_local(async move {
            let _keep = keep;
        }));
    }
}

#[test]
fn a_spawn_while_the_set_drops_is_cancelled() {
    let local = LocalSet::new();
    let _ambient = local.enter();
    let (keep, probe) = probe();
    let slot = Rc::new(RefCell::new(None));
    let spawner = SpawnOnDrop {
        keep: Some(keep),
        slot: slot.clone(),
    };
    let outer = local.spawn_local(async move {
        let _spawner = spawner;
        wakerless().await;
    });

    drop(local);
    assert_cancelled(poll_once(outer));
    let inner = slot.borrow_mut().take().expect("the destructor spawned");
    assert!(
        probe.upgrade().is_none(),
        "the late spawn's future is dropped"
    );
    assert_cancelled(poll_once(inner));
}

struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("destructor boom");
    }
}

#[test]
fn a_panicking_destructor_goes_to_its_handle_and_the_rest_are_cancelled() {
    let local = LocalSet::new();
    let panicky = local.spawn_local(async {
        let _bomb = PanicOnDrop;
        wakerless().await;
    });
    let (keep, probe) = probe();
    let quiet = local.spawn_local(async move {
        let _keep = keep;
        wakerless().await;
    });
    settle(&local);

    drop(local);
    assert!(poll_once(panicky).unwrap_err().is_panic());
    assert!(probe.upgrade().is_none());
    assert_cancelled(poll_once(quiet));
}

#[test]
fn dropping_a_local_runtime_cancels_its_local_tasks() {
    let rt = Builder::new_current_thread()
        .build_local(&LocalOptions::new())
        .unwrap();
    let (keep, probe) = probe();
    let handle = rt.spawn_local(async move {
        let _keep = keep;
        wakerless().await;
    });
    rt.block_on(async {
        for _ in 0..10 {
            yield_now().await;
        }
    });
    assert!(probe.upgrade().is_some());

    drop(rt);
    assert!(probe.upgrade().is_none());
    assert_cancelled(poll_once(handle));
}
