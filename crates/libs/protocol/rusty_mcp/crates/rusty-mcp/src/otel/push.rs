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
    otlp,
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
        let (commands, inbox) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("rusty-mcp-otlp-metrics".to_owned())
            .spawn(move || run(&settings, &inbox))?;
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
fn run(settings: &PushSettings, inbox: &mpsc::Receiver<Command>) {
    let runtime = match rusty_tokio::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(%err, "could not start the metrics push runtime; metrics will not be exported");
            return;
        }
    };
    let client = rusty_request::Client::builder()
        .timeout(settings.request_timeout)
        .no_retry()
        .build();
    let started = now_nanos();

    loop {
        match inbox.recv_timeout(settings.interval) {
            Err(RecvTimeoutError::Timeout) => push(&runtime, &client, settings, started),
            Ok(Command::Flush(ack)) => {
                push(&runtime, &client, settings, started);
                let _ = ack.send(());
            }
            Ok(Command::Shutdown(ack)) => {
                push(&runtime, &client, settings, started);
                let _ = ack.send(());
                return;
            }
            // Every handle is gone without a shutdown: push once more and stop.
            Err(RecvTimeoutError::Disconnected) => {
                push(&runtime, &client, settings, started);
                return;
            }
        }
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
        tracing::warn!(%err, endpoint = %settings.endpoint, "failed to push metrics");
    }
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}
