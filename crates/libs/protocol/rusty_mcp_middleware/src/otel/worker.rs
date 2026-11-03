//! The background thread that sends telemetry to a collector over OTLP/HTTP.
//!
//! One thread owns a small `rusty_tokio` runtime and a `rusty_request` client.
//! What it sends is a [`Job`]'s business: the metrics push and the span batcher
//! are the two. Callers stay synchronous and need no async context:
//! [`Worker::flush`] and [`Worker::shutdown`] hand the thread a command and wait
//! for its acknowledgement, with a bound so a dead collector cannot hold a
//! shutdown.
//!
//! A failed POST is logged and forgotten. Metrics are cumulative, so the next
//! push carries everything this one would have, and retrying spans would only
//! queue work behind a collector that is already struggling.

use std::{
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rusty_json::Value;

use super::redact;

/// Where and how often the thread sends.
pub(crate) struct Settings {
    /// The full URL of the signal's endpoint (`/v1/metrics`, `/v1/traces`).
    pub(crate) endpoint: String,
    /// The longest anything pending waits: the thread sends at least this often,
    /// however steadily items arrive.
    pub(crate) interval: Duration,
    /// How long one POST may take before it is abandoned.
    pub(crate) request_timeout: Duration,
    /// How many items may wait for the thread; more are refused, not queued.
    pub(crate) capacity: usize,
}

/// What a worker thread does with what it is given.
pub(crate) trait Job: Send + 'static {
    /// What callers hand over between sends.
    type Item: Send + 'static;

    /// Take one item. May send a batch.
    fn accept(&mut self, item: Self::Item, post: &Poster);

    /// Send whatever is pending: on the timer, on a flush, and a last time.
    fn send_pending(&mut self, post: &Poster);
}

/// Posts JSON bodies to the endpoint, from the worker thread.
pub(crate) struct Poster {
    runtime: rusty_tokio::Runtime,
    client: rusty_request::Client,
    endpoint: String,
}

impl Poster {
    /// POST `body`. A failure is logged and not returned: telemetry is not
    /// load-bearing.
    pub(crate) fn post(&self, body: &Value) {
        let result = self.runtime.block_on(async {
            self.client
                .post(&self.endpoint)?
                .json(body)?
                .send()
                .await?
                .error_for_status()
        });
        if let Err(err) = result {
            rusty_log::warn!("{}", failure_message(&self.endpoint, &err));
        }
    }
}

/// The log line for a failed POST. The endpoint can carry a credential, and so
/// can the error's text, so it names the origin and the kind of failure only.
fn failure_message(endpoint: &str, err: &rusty_request::Error) -> String {
    format!(
        "failed to send telemetry to {}: {}",
        redact::origin(endpoint),
        redact::reason(err)
    )
}

enum Message<I> {
    /// Hand over an item.
    Item(I),
    /// Send now, then signal.
    Flush(Sender<()>),
    /// Send once more, signal, and stop.
    Shutdown(Sender<()>),
}

/// The handle on the worker thread.
pub(crate) struct Worker<I> {
    messages: Sender<Message<I>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Items sent and not yet taken by the thread.
    waiting: Arc<AtomicUsize>,
    capacity: usize,
}

impl<I> std::fmt::Debug for Worker<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("waiting", &self.waiting.load(Ordering::Relaxed))
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl<I: Send + 'static> Worker<I> {
    /// Start the thread.
    pub(crate) fn spawn(settings: Settings, job: impl Job<Item = I>) -> io::Result<Self> {
        Self::spawn_with(settings, job, |_| {})
    }

    /// [`Self::spawn`], with a hook that sees the thread's runtime before the
    /// loop starts (so a test can put work on its blocking pool).
    fn spawn_with(
        settings: Settings,
        job: impl Job<Item = I>,
        setup: impl FnOnce(&rusty_tokio::Runtime) + Send + 'static,
    ) -> io::Result<Self> {
        let (messages, inbox) = mpsc::channel();
        let waiting = Arc::new(AtomicUsize::new(0));
        let capacity = settings.capacity;
        let thread = thread::Builder::new()
            .name("rusty-mcp-otlp".to_owned())
            .spawn({
                let waiting = Arc::clone(&waiting);
                move || run(&settings, job, &inbox, &waiting, setup)
            })?;
        Ok(Self {
            messages,
            thread: Mutex::new(Some(thread)),
            waiting,
            capacity,
        })
    }

    /// Hand `item` to the thread without blocking. Returns whether it was taken:
    /// `false` when the thread is behind by `capacity` items or has stopped.
    pub(crate) fn submit(&self, item: I) -> bool {
        if self.waiting.fetch_add(1, Ordering::AcqRel) >= self.capacity {
            self.waiting.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        if self.messages.send(Message::Item(item)).is_err() {
            self.waiting.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    /// Send now and wait up to `wait` for it to finish. Returns whether it did.
    pub(crate) fn flush(&self, wait: Duration) -> bool {
        self.ask(Message::Flush, wait)
    }

    /// Send a last time and stop the thread, waiting up to `wait`. Idempotent.
    pub(crate) fn shutdown(&self, wait: Duration) -> bool {
        let done = self.ask(Message::Shutdown, wait);
        let handle = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // Only join a thread that said it is finished; one stuck on a dead
        // collector is left to die with the process rather than block here.
        if done && let Some(handle) = handle {
            let _ = handle.join();
        }
        done
    }

    fn ask(&self, message: impl FnOnce(Sender<()>) -> Message<I>, wait: Duration) -> bool {
        let (ack, acknowledged) = mpsc::channel();
        // The thread is gone (already shut down): nothing is left to send.
        if self.messages.send(message(ack)).is_err() {
            return true;
        }
        acknowledged.recv_timeout(wait).is_ok()
    }
}

impl<I> Drop for Worker<I> {
    fn drop(&mut self) {
        // Best effort: an explicit `shutdown` reports whether it finished.
        let (ack, acknowledged) = mpsc::channel();
        if self.messages.send(Message::Shutdown(ack)).is_ok() {
            let _ = acknowledged.recv_timeout(Duration::from_secs(1));
        }
    }
}

/// The thread's loop.
///
/// A shutdown is acknowledged only after the runtime is gone. Dropping it waits
/// for its blocking pool, and a DNS lookup that outlived its request timeout can
/// still be in there; acknowledging first would let `shutdown` report done, and
/// join, while that wait is still ahead.
fn run<I: Send + 'static>(
    settings: &Settings,
    mut job: impl Job<Item = I>,
    inbox: &mpsc::Receiver<Message<I>>,
    waiting: &AtomicUsize,
    setup: impl FnOnce(&rusty_tokio::Runtime),
) {
    let runtime = match rusty_tokio::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            rusty_log::error!(%err, "could not start the telemetry runtime; nothing will be exported");
            return;
        }
    };
    setup(&runtime);
    let client = rusty_request::Client::builder()
        .timeout(settings.request_timeout)
        .no_retry()
        .build();
    let post = Poster {
        runtime,
        client,
        endpoint: settings.endpoint.clone(),
    };

    // An absolute deadline, not a timeout restarted per message: with one item a
    // second and a five-second interval, a fresh timeout would never expire.
    let mut deadline = Instant::now() + settings.interval;
    let ack = loop {
        match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Err(RecvTimeoutError::Timeout) => {
                job.send_pending(&post);
                deadline = Instant::now() + settings.interval;
            }
            Ok(Message::Item(item)) => {
                waiting.fetch_sub(1, Ordering::AcqRel);
                job.accept(item, &post);
                if Instant::now() >= deadline {
                    job.send_pending(&post);
                    deadline = Instant::now() + settings.interval;
                }
            }
            Ok(Message::Flush(ack)) => {
                job.send_pending(&post);
                deadline = Instant::now() + settings.interval;
                let _ = ack.send(());
            }
            Ok(Message::Shutdown(ack)) => {
                job.send_pending(&post);
                break Some(ack);
            }
            // Every handle is gone without a shutdown: send once more and stop.
            Err(RecvTimeoutError::Disconnected) => {
                job.send_pending(&post);
                break None;
            }
        }
    };
    drop(post);
    if let Some(ack) = ack {
        let _ = ack.send(());
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        time::Instant,
    };

    use super::*;

    /// Counts what it is given; sends nothing.
    #[derive(Default)]
    struct Counting {
        accepted: Arc<AtomicUsize>,
        sent: Arc<AtomicUsize>,
    }

    impl Job for Counting {
        type Item = u32;

        fn accept(&mut self, _item: u32, _post: &Poster) {
            self.accepted.fetch_add(1, Ordering::SeqCst);
        }

        fn send_pending(&mut self, _post: &Poster) {
            self.sent.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn settings(capacity: usize) -> Settings {
        Settings {
            endpoint: "http://127.0.0.1:1/v1/metrics".to_owned(),
            interval: Duration::from_secs(3600),
            request_timeout: Duration::from_secs(1),
            capacity,
        }
    }

    /// Run `f` on a thread and say whether it finished within `limit`.
    fn finishes_within<T: Send + 'static>(
        limit: Duration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(f());
        });
        finished.recv_timeout(limit).ok()
    }

    #[test]
    fn a_shutdown_stops_waiting_when_the_runtime_cannot_drain() {
        // Work on the runtime's blocking pool that does not finish: what a DNS
        // lookup that outlived its request timeout looks like. Dropping the
        // runtime waits for it, so the thread cannot acknowledge, and
        // `shutdown` must give up at its own deadline instead of joining.
        let release = Arc::new(Barrier::new(2));
        let held = Arc::clone(&release);
        let worker = Worker::<u32>::spawn_with(settings(8), Counting::default(), move |runtime| {
            runtime.spawn_blocking(move || {
                held.wait();
            });
        })
        .expect("spawn");

        let started = Instant::now();
        let outcome = finishes_within(Duration::from_secs(10), move || {
            let done = worker.shutdown(Duration::from_millis(200));
            (worker, done)
        });
        let (worker, done) = outcome.expect("shutdown overran its deadline");
        assert!(!done, "reported done while the runtime could not drain");
        assert!(started.elapsed() < Duration::from_secs(5));

        // Let the stuck work finish; the thread then exits on its own.
        release.wait();
        drop(worker);
    }

    #[test]
    fn a_steady_trickle_is_still_sent_on_the_interval() {
        // Items arrive faster than the interval and never fill a batch, so the
        // thread is never idle for a whole interval. A timeout restarted per
        // message would never fire; an absolute deadline does.
        let job = Counting::default();
        let sent = Arc::clone(&job.sent);
        let worker = Worker::<u32>::spawn(
            Settings {
                interval: Duration::from_millis(100),
                ..settings(64)
            },
            job,
        )
        .expect("spawn");

        for n in 0..40 {
            assert!(worker.submit(n));
            thread::sleep(Duration::from_millis(20));
        }

        // About 800 ms of traffic at a 100 ms interval; no flush was asked for.
        assert!(
            sent.load(Ordering::SeqCst) >= 3,
            "sent {} times without ever being idle",
            sent.load(Ordering::SeqCst)
        );
        assert!(worker.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_shutdown_with_nothing_outstanding_reports_done() {
        let worker = Worker::<u32>::spawn(settings(8), Counting::default()).expect("spawn");
        assert!(worker.shutdown(Duration::from_secs(5)));
        // Idempotent.
        assert!(worker.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_shutdown_sends_what_is_pending_and_a_flush_does_too() {
        let job = Counting::default();
        let sent = Arc::clone(&job.sent);
        let worker = Worker::<u32>::spawn(settings(8), job).expect("spawn");

        assert!(worker.flush(Duration::from_secs(5)));
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        assert!(worker.shutdown(Duration::from_secs(5)));
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn items_reach_the_job() {
        let job = Counting::default();
        let accepted = Arc::clone(&job.accepted);
        let worker = Worker::<u32>::spawn(settings(8), job).expect("spawn");

        assert!(worker.submit(1));
        assert!(worker.submit(2));
        // A flush is handled after the items sent before it.
        assert!(worker.flush(Duration::from_secs(5)));
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn items_over_capacity_are_refused_not_queued() {
        // A thread that is busy: its blocking setup holds it before the loop.
        let release = Arc::new(Barrier::new(2));
        let held = Arc::clone(&release);
        let worker = Worker::<u32>::spawn_with(settings(2), Counting::default(), move |_| {
            held.wait();
        })
        .expect("spawn");

        assert!(worker.submit(1));
        assert!(worker.submit(2));
        assert!(!worker.submit(3), "a third item should be refused");

        release.wait();
        assert!(worker.flush(Duration::from_secs(5)));
        // The queue drained, so there is room again.
        assert!(worker.submit(4));
    }

    #[test]
    fn a_failure_message_quotes_neither_the_endpoint_nor_the_error_text() {
        let err = rusty_request::Error::InvalidUrl("https://h/?token=SECRET_IN_ERROR".to_owned());
        let said = failure_message("https://collector:4318/v1/p?token=SECRET_IN_URL", &err);
        assert!(!said.contains("SECRET"), "{said}");
        assert!(said.contains("https://collector:4318"), "{said}");
    }

    #[test]
    fn submitting_after_shutdown_is_refused() {
        let worker = Worker::<u32>::spawn(settings(8), Counting::default()).expect("spawn");
        assert!(worker.shutdown(Duration::from_secs(5)));
        assert!(!worker.submit(1));
    }
}
