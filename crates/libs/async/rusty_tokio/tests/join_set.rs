use rusty_tokio::task::JoinSet;
use rusty_tokio::Runtime;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn join_next_returns_every_spawned_task_and_then_none() {
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    rt.block_on(async {
        let mut set = JoinSet::new();
        for i in 0..20 {
            set.spawn(async move { i });
        }
        assert_eq!(set.len(), 20);

        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        results.sort_unstable();
        assert_eq!(results, (0..20).collect::<Vec<_>>());
        assert!(set.is_empty());
        assert!(set.join_next().await.is_none());
    });
}

#[test]
fn join_next_resolves_in_completion_order_not_spawn_order() {
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    rt.block_on(async {
        let mut set = JoinSet::new();
        // Spawned in order "slow" then "fast" -- join_next should still
        // return "fast" first, since it finishes first.
        set.spawn(async {
            rusty_tokio::time::sleep(Duration::from_millis(100)).await;
            "slow"
        });
        set.spawn(async {
            rusty_tokio::time::sleep(Duration::from_millis(5)).await;
            "fast"
        });

        assert_eq!(set.join_next().await.unwrap().unwrap(), "fast");
        assert_eq!(set.join_next().await.unwrap().unwrap(), "slow");
    });
}

#[test]
fn abort_all_cancels_every_outstanding_task() {
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    rt.block_on(async {
        let mut set = JoinSet::new();
        let ran = Arc::new(AtomicUsize::new(0));
        for _ in 0..5 {
            let ran = ran.clone();
            set.spawn(async move {
                rusty_tokio::time::sleep(Duration::from_secs(60)).await;
                ran.fetch_add(1, Ordering::SeqCst);
            });
        }
        // Give every task a chance to actually start (register its
        // sleep) before aborting.
        rusty_tokio::time::sleep(Duration::from_millis(20)).await;
        set.abort_all();

        let mut cancelled = 0;
        while let Some(r) = set.join_next().await {
            assert!(r.unwrap_err().is_cancelled());
            cancelled += 1;
        }
        assert_eq!(cancelled, 5);
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn shutdown_aborts_and_drains_the_whole_set() {
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    rt.block_on(async {
        let mut set = JoinSet::new();
        for _ in 0..5 {
            set.spawn(async {
                rusty_tokio::time::sleep(Duration::from_secs(60)).await;
            });
        }
        rusty_tokio::time::sleep(Duration::from_millis(20)).await;
        set.shutdown().await;
        assert!(set.is_empty());
    });
}

#[test]
fn dropping_the_set_aborts_tasks_still_in_it() {
    // Unlike a bare JoinHandle (which never aborts on drop), a JoinSet
    // going out of scope should abort everything still in it.
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    let ran = Arc::new(AtomicUsize::new(0));

    rt.block_on(async {
        let mut set = JoinSet::new();
        let ran = ran.clone();
        set.spawn(async move {
            rusty_tokio::time::sleep(Duration::from_millis(50)).await;
            ran.fetch_add(1, Ordering::SeqCst);
        });
        // Let the task actually start (register its sleep) before the
        // set drops.
        rusty_tokio::time::sleep(Duration::from_millis(20)).await;
        drop(set);
        // Long enough that, if the task hadn't been aborted, it would
        // have finished and incremented `ran` by now.
        rusty_tokio::time::sleep(Duration::from_millis(100)).await;
    });

    assert_eq!(ran.load(Ordering::SeqCst), 0);
}

/// `try_join_next` hands back only tasks that have already finished, without
/// waiting (design review 3.7: `rusty_stream`'s accept loop reaps with it).
#[test]
fn try_join_next_returns_only_finished_tasks() {
    let rt = Runtime::builder().worker_threads(2).build().unwrap();
    rt.block_on(async {
        let mut set = JoinSet::new();
        assert!(set.try_join_next().is_none(), "empty set");
        let (tx, rx) = rusty_tokio::sync::oneshot::channel::<()>();
        set.spawn(async move {
            let _ = rx.await;
            1
        });
        set.spawn(async { 2 });
        let mut done = None;
        for _ in 0..100 {
            if let Some(result) = set.try_join_next() {
                done = Some(result.unwrap());
                break;
            }
            rusty_tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(done, Some(2));
        assert!(set.try_join_next().is_none(), "the other is still waiting");
        assert_eq!(set.len(), 1);
        tx.send(()).unwrap();
        assert_eq!(set.join_next().await.unwrap().unwrap(), 1);
    });
}
