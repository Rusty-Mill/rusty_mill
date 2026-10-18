//! Pushing metrics to a collector over OTLP/HTTP.
//!
//! On every interval, and on a flush or shutdown, the registry's current values
//! are POSTed as JSON by a [`worker`](super::worker) thread.
//!
//! A failed push is dropped. Values are cumulative, so the next push carries
//! everything this one would have.

use std::{convert::Infallible, io, sync::Arc, time::Duration};

use super::{
    instrument::{Metric, Registry},
    otlp,
    span::now_nanos,
    worker::{Job, Poster, Settings, Worker},
};

/// The handle on the thread that pushes metrics.
pub(crate) type Pusher = Worker<Infallible>;

/// Everything the metrics push needs.
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

/// Start the thread that pushes `settings.registry`.
pub(crate) fn spawn(settings: PushSettings) -> io::Result<Pusher> {
    let job = MetricsJob {
        resource: settings.resource,
        scope: settings.scope,
        registry: settings.registry,
        started: now_nanos(),
    };
    Worker::spawn(
        Settings {
            endpoint: settings.endpoint,
            interval: settings.interval,
            request_timeout: settings.request_timeout,
            capacity: 0,
        },
        job,
    )
}

/// Pushes the registry; has no items of its own.
struct MetricsJob {
    resource: Vec<(String, String)>,
    scope: &'static str,
    registry: Arc<Registry>,
    /// When the cumulative counts began.
    started: u64,
}

impl Job for MetricsJob {
    type Item = Infallible;

    fn accept(&mut self, item: Infallible, _post: &Poster) {
        match item {}
    }

    /// POST the registry's current values, unless there is nothing to say yet.
    fn send_pending(&mut self, post: &Poster) {
        let metrics = self.registry.collect();
        if metrics.iter().all(Metric::is_empty) {
            return;
        }
        post.post(&otlp::metrics_request(
            &self.resource,
            self.scope,
            self.started,
            now_nanos(),
            &metrics,
        ));
    }
}
