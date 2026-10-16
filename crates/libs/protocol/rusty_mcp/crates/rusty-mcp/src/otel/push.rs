//! Pushing metrics to a collector over OTLP/HTTP.
//!
//! One background thread owns a small `rusty_tokio` runtime and a
//! `rusty_request` client, wakes every interval, and POSTs the registry's
//! current values as JSON. Callers stay synchronous and need no async context:
//! [`Pusher::flush`] and [`Pusher::shutdown`] hand the thread a command and wait
//! for its acknowledgement, with a bound so a dead collector cannot hold a
//! shutdown.
//!
//! A failed push is logged and forgotten. Values are cumulative, so the next
//! push carries everything this one would have; retrying would only queue work
//! behind a collector that is already struggling.

use std::{
    io,
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::{
    instrument::{Metric, Registry},
    otlp, redact,
};

/// Everything the push thread needs.
pub(crate) struct PushSettings {
    /// The full URL of the metrics endpoint, `/v1/metrics` included.
    pub(crate) endpoint: String,
    /// Attributes describing the process (`service.name`, ...).
    pub(crate) resource: Vec<(String, String)>,
    /// The instrumentation scope's name.
    pub(crate) scope: &'static str,
    /// How often to push.
    pub(crate) interval: Duration,
    /// How long one POST may take before it is abandoned.
    pub(crate) request_timeout: Duration,
    /// What to push.
    pub(crate) registry: Arc<Registry>,
}

enum Command {
    /// Push now, then signal.
    Flush(Sender<()>),
    /// Push once more, signal, and stop.
    Shutdown(Sender<()>),
}

/// The handle on the push thread.
#[derive(Debug)]
pub(crate) struct Pusher {
    commands: Sender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Pusher {
    /// Start the push thread.
    pub(crate) fn spawn(settings: PushSettings) -> io::Result<Self> {
        Self::spawn_with(settings, |_| {})
    }

    /// [`Self::spawn`], with a hook that sees the thread's runtime before the
    /// loop starts (so a test can put work on its blocking pool).
    fn spawn_with(
        settings: PushSettings,
        setup: impl FnOnce(&rusty_tokio::Runtime) + Send + 'static,
    ) -> io::Result<Self> {
        let (commands, inbox) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("rusty-mcp-otlp-metrics".to_owned())
            .spawn(move || run(&settings, &inbox, setup))?;
        Ok(Self {
            commands,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Push now and wait up to `wait` for it to finish. Returns whether it did.
    pub(crate) fn flush(&self, wait: Duration) -> bool {
        self.ask(Command::Flush, wait)
    }

    /// Push a last time and stop the thread, waiting up to `wait`. Idempotent.
    pub(crate) fn shutdown(&self, wait: Duration) -> bool {
        let done = self.ask(Command::Shutdown, wait);
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

    fn ask(&self, command: impl FnOnce(Sender<()>) -> Command, wait: Duration) -> bool {
        let (ack, acknowledged) = mpsc::channel();
        // The thread is gone (already shut down): nothing is left to flush.
        if self.commands.send(command(ack)).is_err() {
            return true;
        }
        acknowledged.recv_timeout(wait).is_ok()
    }
}

impl Drop for Pusher {
    fn drop(&mut self) {
        // Best effort: an explicit `shutdown` reports whether it finished.
        let _ = self.shutdown(Duration::from_secs(1));
    }
}

/// The push thread's loop.
///
/// A shutdown is acknowledged only after the runtime is gone. Dropping it waits
/// for its blocking pool, and a DNS lookup that outlived its request timeout can
/// still be in there; acknowledging first would let `shutdown` report done, and
/// join, while that wait is still ahead.
fn run(
    settings: &PushSettings,
    inbox: &mpsc::Receiver<Command>,
    setup: impl FnOnce(&rusty_tokio::Runtime),
) {
    let runtime = match rusty_tokio::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(%err, "could not start the metrics push runtime; metrics will not be exported");
            return;
        }
    };
    setup(&runtime);
    let client = rusty_request::Client::builder()
        .timeout(settings.request_timeout)
        .no_retry()
        .build();
    let started = now_nanos();

    let ack = loop {
        match inbox.recv_timeout(settings.interval) {
            Err(RecvTimeoutError::Timeout) => push(&runtime, &client, settings, started),
            Ok(Command::Flush(ack)) => {
                push(&runtime, &client, settings, started);
                let _ = ack.send(());
            }
            Ok(Command::Shutdown(ack)) => {
                push(&runtime, &client, settings, started);
                break Some(ack);
            }
            // Every handle is gone without a shutdown: push once more and stop.
            Err(RecvTimeoutError::Disconnected) => {
                push(&runtime, &client, settings, started);
                break None;
            }
        }
    };
    drop(client);
    drop(runtime);
    if let Some(ack) = ack {
        let _ = ack.send(());
    }
}

/// POST the registry's current values, unless there is nothing to say yet.
fn push(
    runtime: &rusty_tokio::Runtime,
    client: &rusty_request::Client,
    settings: &PushSettings,
    started: u64,
) {
    let metrics = settings.registry.collect();
    if metrics.iter().all(Metric::is_empty) {
        return;
    }
    let body = otlp::metrics_request(
        &settings.resource,
        settings.scope,
        started,
        now_nanos(),
        &metrics,
    );

    let result = runtime.block_on(async {
        client
            .post(&settings.endpoint)?
            .json(&body)?
            .send()
            .await?
            .error_for_status()
    });
    if let Err(err) = result {
        tracing::warn!("{}", failure_message(&settings.endpoint, &err));
    }
}

/// The log line for a failed push. The endpoint can carry a credential, and so
/// can the error's text, so it names the origin and the kind of failure only.
fn failure_message(endpoint: &str, err: &rusty_request::Error) -> String {
    format!(
        "failed to push metrics to {}: {}",
        redact::origin(endpoint),
        redact::reason(err)
    )
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        time::Instant,
    };

    use super::*;

    fn settings() -> PushSettings {
        PushSettings {
            endpoint: "http://127.0.0.1:1/v1/metrics".to_owned(),
            resource: Vec::new(),
            scope: "test",
            interval: Duration::from_secs(3600),
            request_timeout: Duration::from_secs(1),
            registry: Arc::new(Registry::default()),
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
        let pusher = Pusher::spawn_with(settings(), move |runtime| {
            runtime.spawn_blocking(move || {
                held.wait();
            });
        })
        .expect("spawn");

        let started = Instant::now();
        let outcome = finishes_within(Duration::from_secs(10), move || {
            let done = pusher.shutdown(Duration::from_millis(200));
            (pusher, done)
        });
        let (pusher, done) = outcome.expect("shutdown overran its deadline");
        assert!(!done, "reported done while the runtime could not drain");
        assert!(started.elapsed() < Duration::from_secs(5));

        // Let the stuck work finish; the thread then exits on its own.
        release.wait();
        drop(pusher);
    }

    #[test]
    fn a_failure_message_quotes_neither_the_endpoint_nor_the_error_text() {
        let err = rusty_request::Error::InvalidUrl("https://h/?token=SECRET_IN_ERROR".to_owned());
        let said = failure_message("https://collector:4318/v1/p?token=SECRET_IN_URL", &err);
        assert!(!said.contains("SECRET"), "{said}");
        assert!(said.contains("https://collector:4318"), "{said}");
    }

    #[test]
    fn a_shutdown_with_nothing_outstanding_reports_done() {
        let pusher = Pusher::spawn(settings()).expect("spawn");
        assert!(pusher.shutdown(Duration::from_secs(5)));
        // Idempotent.
        assert!(pusher.shutdown(Duration::from_secs(5)));
    }
}
