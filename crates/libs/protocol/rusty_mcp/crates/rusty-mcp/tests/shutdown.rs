//! Shutdown behaviour: the hook runs, and tasks get a chance to finish.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use rmcp::{ServerHandler, model::CallToolResult, task_manager::TaskExit};
use rusty_mcp::{ServerConfig, tasks::TaskSupport};

/// A handler with no tools; these tests are about the runtime, not dispatch.
#[derive(Clone)]
struct BareServer;

impl ServerHandler for BareServer {}

#[tokio::test]
async fn the_shutdown_hook_runs_when_the_transport_stops() {
    let ran = Arc::new(AtomicBool::new(false));

    // Bind and drop, so `serve` fails to bind and returns promptly. The hook
    // must still run: work spawned before a failure still needs draining.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let config = ServerConfig::http(addr).with_shutdown_hook({
        let ran = Arc::clone(&ran);
        move || {
            let ran = Arc::clone(&ran);
            Box::pin(async move {
                ran.store(true, Ordering::SeqCst);
            })
        }
    });

    // The port is still held, so binding fails and `serve` returns an error.
    let result = rusty_mcp::serve(|| Ok(BareServer), config).await;

    assert!(result.is_err(), "expected the bind to fail");
    assert!(
        ran.load(Ordering::SeqCst),
        "the shutdown hook must run even when the transport failed"
    );
}

#[tokio::test]
async fn drain_lets_a_running_task_finish() {
    let tasks = TaskSupport::new();
    let finished = Arc::new(AtomicBool::new(false));

    tasks.spawn({
        let finished = Arc::clone(&finished);
        move |_ctx| async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            finished.store(true, Ordering::SeqCst);
            Ok(CallToolResult::success(vec![]))
        }
    });

    // Grace comfortably exceeds the task's runtime.
    let abandoned = tasks.drain(Duration::from_secs(5)).await;

    assert_eq!(abandoned, 0, "nothing should have been abandoned");
    assert!(
        finished.load(Ordering::SeqCst),
        "the task should have run to completion"
    );
}

#[tokio::test]
async fn drain_reports_tasks_that_outlast_the_grace_period() {
    let tasks = TaskSupport::new();
    let completed = Arc::new(AtomicUsize::new(0));

    for _ in 0..3 {
        tasks.spawn({
            let completed = Arc::clone(&completed);
            move |_ctx| async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                completed.fetch_add(1, Ordering::SeqCst);
                Ok(CallToolResult::success(vec![]))
            }
        });
    }

    // Far too short for 30-second tasks.
    let abandoned = tasks.drain(Duration::from_millis(100)).await;

    // The count is what tells an operator that clients polling those ids will
    // never get a result — silently dropping them is the failure this avoids.
    assert_eq!(abandoned, 3);
    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "tasks should have been aborted, not allowed to finish"
    );
}

#[tokio::test]
async fn drain_returns_immediately_when_nothing_is_running() {
    let tasks = TaskSupport::new();

    let started = std::time::Instant::now();
    let abandoned = tasks.drain(Duration::from_secs(30)).await;

    assert_eq!(abandoned, 0);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "an idle drain must not wait out the whole grace period"
    );
}

#[tokio::test]
async fn a_cooperative_task_settles_as_cancelled_during_drain() {
    let tasks = TaskSupport::new();

    let handle = tasks.spawn(|ctx| async move {
        tokio::select! {
            _ = ctx.cancelled() => Err(TaskExit::Cancelled),
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                Ok(CallToolResult::success(vec![]))
            }
        }
    });

    tasks
        .cancel(rmcp::model::CancelTaskParams::new(
            handle.task.task_id.clone(),
        ))
        .expect("cancel is acknowledged");

    // The body cooperates, so the drain finds nothing left running.
    let abandoned = tasks.drain(Duration::from_secs(5)).await;
    assert_eq!(abandoned, 0);
}
