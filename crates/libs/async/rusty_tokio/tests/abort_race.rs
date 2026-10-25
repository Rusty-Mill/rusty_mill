//! Bounded cancellation regressions. Deadlines report abandoned joins as
//! failures; no retry can hide a panic, timeout, unexpected result or drop.
use rusty_tokio::{task::JoinHandle, Runtime};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Probe {
    polls: AtomicUsize,
    drops: AtomicUsize,
    waker: Mutex<Option<Waker>>,
}

struct Pending(Arc<Probe>);

impl Future for Pending {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.polls.fetch_add(1, Ordering::SeqCst);
        *self.0.waker.lock().unwrap() = Some(cx.waker().clone());
        // Exercise repeated QUEUED -> RUNNING transitions and coalescing.
        cx.waker().wake_by_ref();
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        assert_eq!(self.0.drops.fetch_add(1, Ordering::SeqCst), 0);
    }
}

#[derive(Debug, Default)]
struct Outcomes {
    cancelled: usize,
    completed: usize,
    panicked: usize,
    timed_out: usize,
}

impl Outcomes {
    async fn record(&mut self, handle: JoinHandle<()>, deadline: Instant) {
        match rusty_tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), handle)
            .await
        {
            Ok(Err(e)) if e.is_cancelled() => self.cancelled += 1,
            Ok(Err(_)) => self.panicked += 1,
            Ok(Ok(())) => self.completed += 1,
            Err(_) => self.timed_out += 1,
        }
    }
}

fn assert_drained(rt: &Runtime) {
    let deadline = Instant::now() + DEADLINE;
    while rt.metrics().num_alive_tasks() != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(rt.metrics().num_alive_tasks(), 0);
}

#[test]
fn abort_and_join_stress_records_every_outcome() {
    const BATCH: usize = 256;
    const ROUNDS: usize = 12;
    for workers in [1, 4] {
        let rt = Runtime::builder().worker_threads(workers).build().unwrap();
        let mut outcomes = Outcomes::default();
        for _ in 0..ROUNDS {
            let mut handles = Vec::new();
            let mut probes = Vec::new();
            for _ in 0..BATCH {
                let probe = Arc::new(Probe::default());
                handles.push(rt.spawn(Pending(probe.clone())));
                probes.push(probe);
            }
            // Runs on the test thread, racing workers which poll and requeue.
            for handle in &handles {
                handle.abort();
                handle.abort_handle().abort();
            }
            let deadline = Instant::now() + DEADLINE;
            rt.block_on(async {
                for handle in handles {
                    outcomes.record(handle, deadline).await;
                }
            });
            // Report all result kinds before asserting; never rerun a batch.
            eprintln!("workers={workers}, cumulative outcomes={outcomes:?}");
            assert_eq!(outcomes.panicked, 0);
            assert_eq!(outcomes.completed, 0);
            assert_eq!(outcomes.timed_out, 0);
            assert_drained(&rt);
            for probe in probes {
                assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
                let polls = probe.polls.load(Ordering::SeqCst);
                if let Some(waker) = probe.waker.lock().unwrap().take() {
                    // Keep the task alive after cancellation, then wake it.
                    waker.wake_by_ref();
                    waker.wake_by_ref();
                    assert_eq!(probe.polls.load(Ordering::SeqCst), polls);
                }
            }
        }
        assert_eq!(outcomes.cancelled, BATCH * ROUNDS);
        assert_drained(&rt);
    }
}

#[test]
fn abort_before_first_poll_completes_current_thread_and_local_joins() {
    let rt = rusty_tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let probe = Arc::new(Probe::default());
    let handle = rt.spawn(Pending(probe.clone()));
    handle.abort();
    let mut outcomes = Outcomes::default();
    rt.block_on(outcomes.record(handle, Instant::now() + DEADLINE));
    eprintln!("pre-poll outcomes={outcomes:?}");
    assert_eq!(outcomes.cancelled, 1);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_drained(&rt);

    let local = rusty_tokio::task::LocalSet::new();
    let probe = Arc::new(Probe::default());
    let handle = local.spawn_local(Pending(probe.clone()));
    handle.abort();
    // LocalSet has no timer context by itself, so nest it inside block_on.
    rt.block_on(async { local.run_until(outcomes.record(handle, Instant::now() + DEADLINE)) });
    eprintln!("including local pre-poll outcomes={outcomes:?}");
    assert_eq!(outcomes.cancelled, 2);
    assert_eq!(probe.polls.load(Ordering::SeqCst), 0);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn abort_during_poll_preserves_pending_and_ready_semantics() {
    for ready in [false, true] {
        let rt = Runtime::builder().worker_threads(1).build().unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let probe = Arc::new(Probe::default());
        let held = Pending(probe.clone());
        let handle = rt.spawn(std::future::poll_fn(move |cx| {
            let _keep_until_drop = &held;
            assert_eq!(held.0.polls.fetch_add(1, Ordering::SeqCst), 0);
            *held.0.waker.lock().unwrap() = Some(cx.waker().clone());
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(DEADLINE).unwrap();
            if ready {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }));
        entered_rx.recv_timeout(DEADLINE).unwrap();
        // No self-wake: abort must leave the notification that drives cleanup
        // after Pending. Ready may win an abort racing the active poll.
        handle.abort();
        handle.abort_handle().abort();
        release_tx.send(()).unwrap();
        let mut outcomes = Outcomes::default();
        rt.block_on(outcomes.record(handle, Instant::now() + DEADLINE));
        eprintln!("abort during poll, ready={ready}, outcomes={outcomes:?}");
        assert_eq!(outcomes.cancelled, usize::from(!ready));
        assert_eq!(outcomes.completed, usize::from(ready));
        assert_eq!(outcomes.panicked, 0);
        assert_eq!(outcomes.timed_out, 0);
        assert_drained(&rt);
        assert_eq!(probe.polls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
        probe.waker.lock().unwrap().take().unwrap().wake();
        assert_drained(&rt);
    }
}
