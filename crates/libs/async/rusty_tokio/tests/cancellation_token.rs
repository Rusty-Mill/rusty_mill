//! `sync::CancellationToken` (#901, gap G8): tokio-util's API and
//! semantics.

use rusty_tokio::sync::{CancellationToken, WaitForCancellationFutureOwned};
use rusty_tokio::time::{sleep, timeout};
use std::future::Future;
use std::time::Duration;

fn assert_send<T: Send>(_: &T) {}
fn assert_static<T: 'static>(_: &T) {}

#[test]
fn a_new_token_is_not_cancelled_until_cancel() {
    let token = CancellationToken::new();
    assert!(!token.is_cancelled());
    token.cancel();
    assert!(token.is_cancelled());
    // Idempotent.
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn clones_share_state() {
    let token = CancellationToken::new();
    let clone = token.clone();
    clone.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn default_and_debug() {
    let token = CancellationToken::default();
    assert_eq!(
        format!("{token:?}"),
        "CancellationToken { is_cancelled: false }"
    );
    token.cancel();
    assert_eq!(
        format!("{token:?}"),
        "CancellationToken { is_cancelled: true }"
    );
}

#[rusty_tokio::test]
async fn cancelled_resolves_at_once_when_already_cancelled() {
    let token = CancellationToken::new();
    token.cancel();
    timeout(Duration::from_secs(5), token.cancelled())
        .await
        .unwrap();
    timeout(Duration::from_secs(5), token.clone().cancelled_owned())
        .await
        .unwrap();
}

#[rusty_tokio::test]
async fn cancel_wakes_a_waiting_task() {
    let token = CancellationToken::new();
    let waiter = token.clone();
    let task = rusty_tokio::spawn(async move {
        waiter.cancelled().await;
        waiter.is_cancelled()
    });
    sleep(Duration::from_millis(20)).await;
    assert!(!task.is_finished());
    token.cancel();
    assert!(timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap());
}

#[rusty_tokio::test]
async fn futures_are_send_and_the_owned_one_is_static() {
    let token = CancellationToken::new();
    let borrowed = token.cancelled();
    assert_send(&borrowed);
    drop(borrowed);
    let owned: WaitForCancellationFutureOwned = token.clone().cancelled_owned();
    assert_send(&owned);
    assert_static(&owned);
    let task = rusty_tokio::spawn(owned);
    token.cancel();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[rusty_tokio::test]
async fn cancelling_a_parent_cancels_every_descendant() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let grandchild = child.child_token();
    let sibling = parent.child_token();

    let waiting = grandchild.clone();
    let task = rusty_tokio::spawn(async move { waiting.cancelled().await });

    parent.cancel();
    assert!(child.is_cancelled());
    assert!(grandchild.is_cancelled());
    assert!(sibling.is_cancelled());
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn cancelling_a_child_leaves_the_parent_and_siblings_alone() {
    let parent = CancellationToken::new();
    let child = parent.child_token();
    let sibling = parent.child_token();
    let grandchild = child.child_token();
    child.cancel();
    assert!(child.is_cancelled());
    assert!(grandchild.is_cancelled());
    assert!(!parent.is_cancelled());
    assert!(!sibling.is_cancelled());
}

#[test]
fn a_child_of_a_cancelled_token_starts_cancelled() {
    let parent = CancellationToken::new();
    parent.cancel();
    let child = parent.child_token();
    assert!(child.is_cancelled());
    assert!(child.child_token().is_cancelled());
}

#[test]
fn a_child_outlives_a_dropped_parent_handle_and_still_cancels_with_it() {
    let parent = CancellationToken::new();
    let keep = parent.clone();
    let child = parent.child_token();
    drop(parent);
    assert!(!child.is_cancelled());
    keep.cancel();
    assert!(child.is_cancelled());
}

#[test]
fn drop_guard_cancels_on_drop() {
    let token = CancellationToken::new();
    let guard = token.clone().drop_guard();
    assert!(!token.is_cancelled());
    drop(guard);
    assert!(token.is_cancelled());
}

#[test]
fn a_disarmed_drop_guard_does_not_cancel() {
    let token = CancellationToken::new();
    let returned = token.clone().drop_guard().disarm();
    assert!(!token.is_cancelled());
    returned.cancel();
    assert!(token.is_cancelled());
}

#[rusty_tokio::test]
async fn select_loop_on_cancelled_exits_once_cancelled() {
    let token = CancellationToken::new();
    let (tx, mut rx) = rusty_tokio::sync::mpsc::unbounded_channel::<u32>();
    // The consumer says when it has all 50, so cancelling cannot race them.
    let (all_in, all_in_rx) = rusty_tokio::sync::oneshot::channel::<()>();
    let mut all_in = Some(all_in);
    let loop_token = token.clone();
    let task = rusty_tokio::spawn(async move {
        let mut received = 0;
        loop {
            // This crate's `select!` arms are closures, so the arm
            // can't `return` from the task itself.
            let cancelled = rusty_tokio::select! {
                _ = loop_token.cancelled() => true,
                _ = rx.recv() => false,
            };
            if cancelled {
                return received;
            }
            received += 1;
            if received != 50 {
                continue;
            }
            if let Some(all_in) = all_in.take() {
                all_in.send(()).expect("the test is waiting");
            }
        }
    });
    for i in 0..50 {
        tx.send(i).unwrap();
    }
    timeout(Duration::from_secs(5), all_in_rx)
        .await
        .unwrap()
        .unwrap();
    token.cancel();
    let received = timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, 50);
}

#[test]
fn concurrent_cancel_wakes_every_waiter_across_the_tree() {
    let rt = rusty_tokio::Runtime::builder()
        .worker_threads(4)
        .build()
        .unwrap();
    rt.block_on(async {
        let root = CancellationToken::new();
        let mut tasks = Vec::new();
        for _ in 0..50 {
            let child = root.child_token();
            for _ in 0..4 {
                let grandchild = child.child_token();
                tasks.push(rusty_tokio::spawn(grandchild.cancelled_owned()));
            }
            tasks.push(rusty_tokio::spawn(child.cancelled_owned()));
        }
        sleep(Duration::from_millis(20)).await;
        let canceller = root.clone();
        std::thread::spawn(move || canceller.cancel())
            .join()
            .unwrap();
        for task in tasks {
            timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap();
        }
    });
}

#[test]
fn cancelled_is_pending_until_cancel() {
    let token = CancellationToken::new();
    let mut fut = Box::pin(token.cancelled());
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(fut.as_mut().poll(&mut cx).is_pending());
    token.cancel();
    assert!(fut.as_mut().poll(&mut cx).is_ready());
}
