//! The blocking-task thread pool `spawn_blocking` offloads onto,
//! separate from the async worker pool. Threads are grown lazily (one
//! per queued job, up to a cap) and shrink back down after sitting idle
//! for a while, rather than either spawning a fresh OS thread per call
//! (real overhead for anything short-lived) or keeping a fixed-size
//! pool alive forever (wasted threads when nothing's blocking).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

struct Inner {
    queue: Mutex<VecDeque<Job>>,
    condvar: Condvar,
    shutdown: AtomicBool,
    live_threads: AtomicUsize,
    /// Jobs queued or running, not yet returned -- read by the paused
    /// clock's auto-advance (see [`BlockingPool::has_in_flight`]).
    in_flight: AtomicUsize,
    max_threads: usize,
    idle_timeout: Duration,
    thread_config: super::ThreadConfig,
}

pub(crate) struct BlockingPool {
    inner: Arc<Inner>,
}

impl BlockingPool {
    pub(crate) fn new(
        max_threads: usize,
        thread_config: super::ThreadConfig,
        idle_timeout: Duration,
    ) -> Self {
        assert!(max_threads > 0, "a blocking pool needs at least one thread");
        BlockingPool {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                condvar: Condvar::new(),
                shutdown: AtomicBool::new(false),
                live_threads: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                max_threads,
                idle_timeout,
                thread_config,
            }),
        }
    }

    pub(crate) fn spawn(&self, job: Job) {
        // Counted before it's visible to a worker, so there's no window
        // where the job exists but `has_in_flight` reads false.
        self.inner.in_flight.fetch_add(1, Ordering::AcqRel);
        self.inner.queue.lock().unwrap().push_back(job);
        self.inner.condvar.notify_one();
        self.grow_if_needed();
    }

    /// How many blocking-pool OS threads are currently alive -- backs
    /// [`super::RuntimeMetrics::num_blocking_threads`].
    pub(crate) fn live_threads(&self) -> usize {
        self.inner.live_threads.load(Ordering::Acquire)
    }

    /// Whether any `spawn_blocking` job is queued or still running.
    /// Auto-advance of a paused clock holds off while this is true, as
    /// tokio's does: blocking work runs in real time, so jumping virtual
    /// time past a timeout while it's still running would fire that
    /// timeout for work that was never actually late.
    pub(crate) fn has_in_flight(&self) -> bool {
        self.inner.in_flight.load(Ordering::Acquire) > 0
    }

    /// Adds one more worker thread if the pool is under its cap. Called
    /// on every `spawn`, so it may occasionally add a thread that turns
    /// out not to be needed (another idle thread picks up the job
    /// first) -- harmless, the extra thread just idles out later.
    fn grow_if_needed(&self) {
        loop {
            let current = self.inner.live_threads.load(Ordering::Acquire);
            if current >= self.inner.max_threads {
                return;
            }
            if self
                .inner
                .live_threads
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let inner = self.inner.clone();
                inner
                    .thread_config
                    .thread_builder(|| "rusty_tokio-blocking".to_string())
                    .spawn(move || Self::worker_loop(inner))
                    .expect("failed to spawn rusty_tokio blocking-pool thread");
                return;
            }
        }
    }

    fn worker_loop(inner: Arc<Inner>) {
        loop {
            let job = {
                let mut guard = inner.queue.lock().unwrap();
                loop {
                    if let Some(job) = guard.pop_front() {
                        break Some(job);
                    }
                    if inner.shutdown.load(Ordering::Acquire) {
                        break None;
                    }
                    let (new_guard, timeout) = inner
                        .condvar
                        .wait_timeout(guard, inner.idle_timeout)
                        .unwrap();
                    guard = new_guard;
                    if timeout.timed_out() && guard.is_empty() {
                        break None;
                    }
                }
            };
            match job {
                Some(job) => {
                    // Decremented only after `job` returns -- by then it
                    // has already woken whatever awaits its result, so
                    // a scheduler that sees zero here also sees that
                    // task queued. The guard keeps the count right even
                    // if `job` unwinds.
                    let _done = InFlightGuard(&inner.in_flight);
                    job();
                }
                None => {
                    inner.live_threads.fetch_sub(1, Ordering::AcqRel);
                    // Wake anyone in `shutdown()` waiting for the count
                    // to reach zero.
                    inner.condvar.notify_all();
                    return;
                }
            }
        }
    }

    /// Tells every blocking-pool thread to stop once its queue empties,
    /// without waiting for any of them to actually do so -- see
    /// [`BlockingPool::wait_for_drain`] for the part that waits.
    pub(crate) fn signal_shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.condvar.notify_all();
    }

    /// Waits for every blocking-pool thread to exit, or until `deadline`
    /// passes -- whichever comes first (waits unboundedly if `deadline`
    /// is `None`). Threads are transient and self-terminating (unlike
    /// the fixed reactor/timer threads), so there's nothing to `join`
    /// here, only this count to watch. The bounded poll interval is a
    /// safety net in case a decrement-then-notify on another thread
    /// races this wait's setup.
    pub(crate) fn wait_for_drain(&self, deadline: Option<Instant>) {
        let mut guard = self.inner.queue.lock().unwrap();
        while self.inner.live_threads.load(Ordering::Acquire) > 0 {
            let wait = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return;
                    }
                    (deadline - now).min(Duration::from_millis(50))
                }
                None => Duration::from_millis(50),
            };
            guard = self.inner.condvar.wait_timeout(guard, wait).unwrap().0;
        }
    }

    /// Signals shutdown and waits (unboundedly) for every thread to
    /// drain -- used by [`super::Runtime`]'s `Drop`, where relying on it
    /// as a clean-shutdown point (tests, in particular) matters more
    /// than bounding the wait.
    pub(crate) fn shutdown(&self) {
        self.signal_shutdown();
        self.wait_for_drain(None);
    }
}

/// Decrements [`Inner::in_flight`] on drop -- see `worker_loop`.
struct InFlightGuard<'a>(&'a AtomicUsize);

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
