//! Batching finished spans for export.
//!
//! The [`worker`](super::worker) thread collects spans as the layer closes
//! them and sends a batch when [`BATCH_SIZE`] have built up, when the interval
//! passes with some waiting, or on a flush or shutdown.

use std::{io, time::Duration};

use super::{
    otlp,
    span::FinishedSpan,
    worker::{Job, Poster, Settings, Worker},
};

/// Spans per POST.
pub(crate) const BATCH_SIZE: usize = 512;

/// Spans that may wait for the thread before more are dropped.
pub(crate) const QUEUE_CAPACITY: usize = 8 * BATCH_SIZE;

/// The handle on the thread that exports spans.
pub(crate) type SpanExporter = Worker<FinishedSpan>;

/// Everything the span exporter needs.
pub(crate) struct ExportSettings {
    /// The full URL of the traces endpoint, `/v1/traces` included.
    pub(crate) endpoint: String,
    /// Attributes describing the process (`service.name`, ...).
    pub(crate) resource: Vec<(String, String)>,
    /// The instrumentation scope's name.
    pub(crate) scope: &'static str,
    /// How long spans wait for company before they are sent anyway.
    pub(crate) interval: Duration,
    /// How long one POST may take before it is abandoned.
    pub(crate) request_timeout: Duration,
}

/// Start the thread that exports spans.
pub(crate) fn spawn(settings: ExportSettings) -> io::Result<SpanExporter> {
    let job = SpanBatch {
        resource: settings.resource,
        scope: settings.scope,
        pending: Vec::new(),
    };
    Worker::spawn(
        Settings {
            endpoint: settings.endpoint,
            interval: settings.interval,
            request_timeout: settings.request_timeout,
            capacity: QUEUE_CAPACITY,
        },
        job,
    )
}

struct SpanBatch {
    resource: Vec<(String, String)>,
    scope: &'static str,
    pending: Vec<FinishedSpan>,
}

impl Job for SpanBatch {
    type Item = FinishedSpan;

    fn accept(&mut self, span: FinishedSpan, post: &Poster) {
        self.pending.push(span);
        if self.pending.len() >= BATCH_SIZE {
            self.send_pending(post);
        }
    }

    fn send_pending(&mut self, post: &Poster) {
        if self.pending.is_empty() {
            return;
        }
        // Taken before the POST, and not put back if it fails: a batch that
        // could not be delivered is dropped rather than retried behind a
        // collector that is already struggling.
        let batch = std::mem::take(&mut self.pending);
        post.post(&otlp::traces_request(&self.resource, self.scope, &batch));
    }
}
