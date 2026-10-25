//! `bridge::spawn`: work for the transport's runtime, awaited from a `tokio` caller. The caller
//! and the work meet only through the returned future, and dropping it cancels the work.
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusty_tonic::bridge::spawn;
use rusty_tonic::Code;

/// Sets a flag when dropped.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Waits (up to five seconds) for `flag`, and fails rather than hang if it never comes.
async fn flag_is_set(flag: &AtomicBool) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn the_result_comes_back_to_a_tokio_caller() {
    let out = spawn(async { 7 }).unwrap().await.unwrap();
    assert_eq!(out, 7);
}

#[tokio::test]
async fn the_work_runs_on_the_transports_runtime_not_the_callers() {
    let in_tokio = spawn(async { tokio::runtime::Handle::try_current().is_ok() })
        .unwrap()
        .await
        .unwrap();
    assert!(
        !in_tokio,
        "the work must not run inside the caller's runtime"
    );
    let in_rusty = spawn(async { rusty_tokio::time::sleep(Duration::from_millis(5)).await })
        .unwrap()
        .await;
    assert!(in_rusty.is_ok(), "the work has rusty_tokio's timers");
}

#[tokio::test]
async fn many_calls_at_once_all_finish() {
    let calls: Vec<_> = (0..200u32)
        .map(|n| spawn(async move { n * 2 }).unwrap())
        .collect();
    let mut total = 0;
    for call in calls {
        total += call.await.unwrap();
    }
    assert_eq!(total, (0..200u32).map(|n| n * 2).sum::<u32>());
}

#[tokio::test]
async fn dropping_the_wait_cancels_the_work() {
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    let started = Arc::new(AtomicBool::new(false));
    let seen = started.clone();
    let call = spawn(async move {
        let _guard = guard;
        seen.store(true, Ordering::SeqCst);
        rusty_tokio::time::sleep(Duration::from_secs(60)).await;
    })
    .unwrap();
    // The work has begun, then the caller gives up.
    assert!(tokio::time::timeout(Duration::from_millis(100), call)
        .await
        .is_err());
    assert!(started.load(Ordering::SeqCst));
    assert!(flag_is_set(&dropped).await, "the work kept running");
}

#[tokio::test]
async fn a_panic_in_the_work_is_an_internal_error() {
    let out = spawn(async {
        if true {
            panic!("boom");
        }
    })
    .unwrap()
    .await;
    assert_eq!(out.unwrap_err().code(), Code::Internal);
}
