use rusty_tokio::Runtime;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn is_shutting_down_reflects_current_state() {
    let rt = Runtime::new().unwrap();
    let handle = rt.handle();
    assert!(!handle.is_shutting_down());
    rt.shutdown_background();
    assert!(handle.is_shutting_down());
}

#[test]
fn shutdown_notified_gives_a_task_a_real_chance_to_run_cleanup() {
    let rt = Runtime::new().unwrap();
    let handle = rt.handle();
    let cleaned_up = Arc::new(AtomicBool::new(false));

    rt.block_on({
        let cleaned_up = cleaned_up.clone();
        let handle = handle.clone();
        async move {
            rusty_tokio::spawn(async move {
                handle.shutdown_notified().await;
                cleaned_up.store(true, Ordering::SeqCst);
            });
            // Give the cleanup task a chance to actually start polling
            // (and register itself as a waiter) before shutdown fires.
            rusty_tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    rt.shutdown_timeout(Duration::from_secs(1));
    assert!(
        cleaned_up.load(Ordering::SeqCst),
        "a task awaiting shutdown_notified() should run its cleanup before shutdown_timeout returns"
    );
}

#[test]
fn shutdown_timeout_waits_for_an_outstanding_task_to_finish_naturally() {
    let rt = Runtime::new().unwrap();
    let finished = Arc::new(AtomicBool::new(false));

    rt.block_on({
        let finished = finished.clone();
        async move {
            rusty_tokio::spawn(async move {
                rusty_tokio::time::sleep(Duration::from_millis(50)).await;
                finished.store(true, Ordering::SeqCst);
            });
        }
    });

    rt.shutdown_timeout(Duration::from_secs(1));
    assert!(
        finished.load(Ordering::SeqCst),
        "shutdown_timeout should wait for an outstanding task to finish before tearing down"
    );
}

#[test]
fn shutdown_timeout_gives_up_after_its_deadline_on_a_stuck_task() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        rusty_tokio::spawn(async {
            // Never resolves: nothing ever calls notify_one/notify_waiters.
            let notify = rusty_tokio::sync::Notify::new();
            notify.notified().await;
        });
    });

    let start = Instant::now();
    rt.shutdown_timeout(Duration::from_millis(100));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "shutdown_timeout should stop waiting once its deadline passes, not hang on a task that never finishes"
    );
}

#[test]
fn shutdown_background_returns_immediately_without_waiting_for_outstanding_tasks() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        rusty_tokio::spawn(async {
            rusty_tokio::time::sleep(Duration::from_secs(5)).await;
        });
    });

    let start = Instant::now();
    rt.shutdown_background();
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "shutdown_background should never wait on outstanding tasks"
    );
}

/// `shutdown_notified()` stays `Unpin + Send + Sync`, as it was before
/// #583, so `select! { _ = &mut fut => .. }` in a loop and `Pin::new`
/// keep compiling.
#[test]
fn shutdown_notified_is_unpin_send_and_sync() {
    fn assert_traits<F: std::future::Future + Unpin + Send + Sync>(_: &F) {}
    let rt = Runtime::new().unwrap();
    let handle = rt.handle();
    assert_traits(&handle.shutdown_notified());
}

/// Runs `f` on its own thread and fails if it hasn't returned within
/// `limit`, so a teardown deadlock fails the test instead of hanging it.
fn within(limit: Duration, f: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = done.send(());
    });
    finished
        .recv_timeout(limit)
        .expect("runtime teardown deadlocked");
}

/// A blocking job waits on a channel whose sender a pending task owns;
/// the runtime has to cancel that task before it can drain the pool.
fn blocking_job_waiting_on_a_task(rt: &Runtime) -> std::sync::mpsc::Receiver<Option<()>> {
    let (tx, rx) = rusty_tokio::sync::mpsc::channel::<()>(1);
    let (ended, job_ended) = std::sync::mpsc::channel();
    rt.block_on(async move {
        rusty_tokio::spawn(async move {
            let _tx = tx;
            std::future::pending::<()>().await;
        });
        rusty_tokio::spawn_blocking(move || {
            let mut rx = rx;
            let _ = ended.send(rx.blocking_recv());
        });
    });
    job_ended
}

#[test]
fn dropping_a_runtime_cancels_tasks_before_waiting_on_blocking_jobs() {
    for multi_thread in [true, false] {
        within(Duration::from_secs(10), move || {
            let rt = if multi_thread {
                Runtime::new().unwrap()
            } else {
                rusty_tokio::Builder::new_current_thread().build().unwrap()
            };
            let job_ended = blocking_job_waiting_on_a_task(&rt);
            drop(rt);
            assert_eq!(
                job_ended.try_recv(),
                Ok(None),
                "the job saw the channel close"
            );
        });
    }
}

/// `shutdown_timeout` waits its deadline for the pending task, then
/// cancels it, which frees the blocking job instead of leaving it behind.
#[test]
fn shutdown_timeout_cancels_tasks_before_waiting_on_blocking_jobs() {
    within(Duration::from_secs(10), || {
        let rt = Runtime::new().unwrap();
        let job_ended = blocking_job_waiting_on_a_task(&rt);
        rt.shutdown_timeout(Duration::from_millis(200));
        assert_eq!(job_ended.recv_timeout(Duration::from_secs(2)), Ok(None));
    });
}
