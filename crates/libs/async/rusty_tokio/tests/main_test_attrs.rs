use rusty_tokio::sync::oneshot;
use rusty_tokio::task::JoinSet;
use std::time::Duration;

// No `#[test]` written here -- `#[rusty_tokio::test]` emits it itself.

#[rusty_tokio::test]
async fn basic_body_runs_on_the_runtime() {
    let (tx, rx) = oneshot::channel::<i32>();
    rusty_tokio::spawn(async move {
        let _ = tx.send(42);
    });
    assert_eq!(rx.await.unwrap(), 42);
}

#[rusty_tokio::test]
async fn can_return_a_result() -> Result<(), String> {
    rusty_tokio::time::sleep(Duration::from_millis(1)).await;
    Ok(())
}

#[rusty_tokio::test(worker_threads = 1)]
async fn accepts_a_worker_threads_argument() {
    let (tx, rx) = oneshot::channel::<&str>();
    rusty_tokio::spawn(async move {
        let _ = tx.send("done");
    });
    assert_eq!(rx.await.unwrap(), "done");
}

#[rusty_tokio::test(worker_threads = 4)]
async fn many_concurrent_spawns_all_complete() {
    let mut set = JoinSet::new();
    for i in 0..20 {
        set.spawn(async move { i });
    }
    let mut sum = 0;
    while let Some(r) = set.join_next().await {
        sum += r.unwrap();
    }
    assert_eq!(sum, (0..20).sum());
}

// ---- `flavor` / `start_paused` (#899, gaps G4 and G5) ----

#[rusty_tokio::test(flavor = "current_thread")]
async fn current_thread_flavor_runs_on_a_current_thread_runtime() {
    let handle = rusty_tokio::Handle::current();
    assert_eq!(
        handle.runtime_flavor(),
        rusty_tokio::RuntimeFlavor::CurrentThread
    );
    // A spawned task runs on the calling thread too.
    let caller = std::thread::current().id();
    let ran_on = rusty_tokio::spawn(async { std::thread::current().id() })
        .await
        .unwrap();
    assert_eq!(ran_on, caller);
}

#[rusty_tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_thread_flavor_honors_worker_threads() {
    let handle = rusty_tokio::Handle::current();
    assert_eq!(
        handle.runtime_flavor(),
        rusty_tokio::RuntimeFlavor::MultiThread
    );
    assert_eq!(handle.metrics().num_workers(), 2);
}

#[rusty_tokio::test(flavor = "multi_thread")]
async fn multi_thread_flavor_without_worker_threads_builds() {
    assert_eq!(
        rusty_tokio::Handle::current().runtime_flavor(),
        rusty_tokio::RuntimeFlavor::MultiThread
    );
}

#[rusty_tokio::test]
async fn default_flavor_is_still_multi_thread() {
    assert_eq!(
        rusty_tokio::Handle::current().runtime_flavor(),
        rusty_tokio::RuntimeFlavor::MultiThread
    );
}

#[rusty_tokio::test(flavor = "current_thread", start_paused = true)]
async fn start_paused_makes_an_hour_long_sleep_complete_at_once() {
    let wall = std::time::Instant::now();
    let start = rusty_tokio::time::Instant::now();
    rusty_tokio::time::sleep(Duration::from_secs(3600)).await;
    assert_eq!(start.elapsed(), Duration::from_secs(3600));
    assert!(wall.elapsed() < Duration::from_secs(5));
}

#[rusty_tokio::test(start_paused = true)]
async fn start_paused_without_a_flavor_selects_current_thread_on_test() {
    assert_eq!(
        rusty_tokio::Handle::current().runtime_flavor(),
        rusty_tokio::RuntimeFlavor::CurrentThread
    );
    let wall = std::time::Instant::now();
    rusty_tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(wall.elapsed() < Duration::from_secs(5));
}

#[rusty_tokio::test(flavor = "current_thread", start_paused = false)]
async fn start_paused_false_leaves_the_clock_running() {
    let before = rusty_tokio::time::Instant::now();
    std::thread::sleep(Duration::from_millis(5));
    assert!(before.elapsed() >= Duration::from_millis(5));
}
