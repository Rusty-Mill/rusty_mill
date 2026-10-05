//! Audit records: one before a run is forwarded, one after it ends.
//!
//! Both are structured `tracing` events on the `agentgateway::audit`
//! target, so a log pipeline can route them apart from the rest. The
//! before-record exists before the upstream call is made, so a run that
//! never completes still has its decision on record. The after-record is
//! written when the response stream ends, or when the client goes away
//! first, with what the gateway saw of the run: how many events, how it
//! ended, and whether the agent kept the protocol.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use rusty_agui::sse::Decoder;
use rusty_agui::{EventKind, Verifier};

use crate::RunSummary;

/// Log target of every audit record.
pub const TARGET: &str = "agentgateway::audit";

/// Record the decision, before anything is forwarded.
pub fn record_decision(route: &str, summary: Option<&RunSummary>, permitted: bool, reason: &str) {
    let (thread_id, run_id, subject, tools, messages) = match summary {
        Some(s) => (
            s.thread_id.as_str(),
            s.run_id.as_str(),
            s.subject.as_deref().unwrap_or(""),
            s.tools.join(","),
            s.messages,
        ),
        None => ("", "", "", String::new(), 0),
    };
    tracing::info!(
        target: TARGET,
        route,
        decision = if permitted { "permit" } else { "refuse" },
        reason,
        thread_id,
        run_id,
        subject,
        tools = %tools,
        messages,
        "agui run decided"
    );
}

/// How a forwarded run ended, as the gateway saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// Canonical events seen on the stream.
    pub events: u64,
    /// `finished`, `error` (the agent sent `RUN_ERROR`), `incomplete` (the
    /// stream ended before either), `disconnected` (the client went away),
    /// `transport` (the upstream failed mid-stream) or `invalid` (the agent
    /// broke the protocol's ordering rules).
    pub outcome: &'static str,
    /// The `RUN_ERROR` message, the sequence error, or the transport error.
    pub error: Option<String>,
    /// From the decision to the end of the stream.
    pub elapsed_ms: u128,
}

/// Record how a run ended.
pub fn record_outcome(route: &str, summary: &RunSummary, status: u16, report: &RunReport) {
    tracing::info!(
        target: TARGET,
        route,
        thread_id = %summary.thread_id,
        run_id = %summary.run_id,
        subject = summary.subject.as_deref().unwrap_or(""),
        status,
        outcome = report.outcome,
        events = report.events,
        error = report.error.as_deref().unwrap_or(""),
        elapsed_ms = report.elapsed_ms,
        "agui run ended"
    );
}

/// A response body that watches the event stream go by and reports once it
/// ends. The bytes are passed through unchanged: the gateway records, it
/// does not re-encode.
pub struct AuditedBody<B> {
    inner: B,
    decoder: Decoder,
    verifier: Verifier,
    events: u64,
    started: Instant,
    /// The outcome once known; the stream keeps flowing after it.
    outcome: Option<(&'static str, Option<String>)>,
    report: Option<Box<dyn FnOnce(RunReport) + Send>>,
}

impl<B> AuditedBody<B> {
    /// Wrap `inner`; `report` is called exactly once, when the stream ends
    /// or the body is dropped.
    pub fn new(inner: B, report: impl FnOnce(RunReport) + Send + 'static) -> Self {
        AuditedBody {
            inner,
            decoder: Decoder::new(),
            verifier: Verifier::new(),
            events: 0,
            started: Instant::now(),
            outcome: None,
            report: Some(Box::new(report)),
        }
    }

    fn observe(&mut self, bytes: &[u8]) {
        if self.outcome.is_some() {
            return;
        }
        let raw = match self.decoder.feed(bytes) {
            Ok(raw) => raw,
            Err(e) => {
                self.outcome = Some(("invalid", Some(e.to_string())));
                return;
            }
        };
        for event in raw {
            match self.verifier.push(event) {
                Ok(canonical) => {
                    for event in canonical {
                        self.events += 1;
                        match &event.kind {
                            EventKind::RunFinished { .. } => {
                                self.outcome = Some(("finished", None))
                            }
                            EventKind::RunError { message, .. } => {
                                self.outcome = Some(("error", Some(message.clone())));
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    self.outcome = Some(("invalid", Some(e.to_string())));
                    return;
                }
            }
        }
    }

    fn finish(&mut self, if_unknown: &'static str, error: Option<String>) {
        let Some(report) = self.report.take() else {
            return;
        };
        let (outcome, err) = self.outcome.take().unwrap_or((if_unknown, error));
        report(RunReport {
            events: self.events,
            outcome,
            error: err,
            elapsed_ms: self.started.elapsed().as_millis(),
        });
    }
}

impl<B> http_body::Body for AuditedBody<B>
where
    B: http_body::Body<Data = Bytes> + Unpin,
    B::Error: std::fmt::Display,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    this.observe(data);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(err))) => {
                this.finish("transport", Some(err.to_string()));
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(None) => {
                this.finish("incomplete", None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

impl<B> Drop for AuditedBody<B> {
    fn drop(&mut self) {
        // A client that disconnects drops the body before the stream ends;
        // the run still gets its record.
        self.finish("disconnected", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;
    use std::sync::{Arc, Mutex};

    fn frames(events: &[&str]) -> Vec<Bytes> {
        events
            .iter()
            .map(|e| Bytes::from(format!("data: {e}\n\n")))
            .collect()
    }

    async fn drain(chunks: Vec<Bytes>, cut: bool) -> (Vec<u8>, RunReport) {
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let body = AuditedBody::new(Chunks(chunks.into_iter()), move |report| {
            *sink.lock().expect("lock") = Some(report);
        });
        // Boxed, not `pin!`ed: `drop(body)` below must drop the body itself.
        let mut body = Box::pin(body);
        let mut out = Vec::new();
        let mut n = 0;
        while let Some(frame) = body.frame().await {
            let frame = frame.expect("frame");
            if let Some(data) = frame.data_ref() {
                out.extend_from_slice(data);
            }
            n += 1;
            if cut && n == 1 {
                break;
            }
        }
        drop(body);
        let report = seen.lock().expect("lock").take().expect("reported once");
        (out, report)
    }

    /// A body that yields each chunk as its own frame.
    struct Chunks(std::vec::IntoIter<Bytes>);

    impl http_body::Body for Chunks {
        type Data = Bytes;
        type Error = std::convert::Infallible;
        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
            Poll::Ready(self.0.next().map(|b| Ok(http_body::Frame::data(b))))
        }
    }

    #[tokio::test]
    async fn passes_bytes_through_and_reports_a_finished_run() {
        let chunks = frames(&[
            r#"{"type":"RUN_STARTED","threadId":"t","runId":"r"}"#,
            r#"{"type":"TEXT_MESSAGE_CHUNK","messageId":"m","delta":"hi"}"#,
            r#"{"type":"RUN_FINISHED","threadId":"t","runId":"r"}"#,
        ]);
        let expected: Vec<u8> = chunks.iter().flat_map(|b| b.to_vec()).collect();
        let (out, report) = drain(chunks, false).await;
        assert_eq!(out, expected, "bytes are untouched");
        assert_eq!(report.outcome, "finished");
        // RUN_STARTED, START, CONTENT, END, RUN_FINISHED: the chunk expanded.
        assert_eq!(report.events, 5);
        assert_eq!(report.error, None);
    }

    #[tokio::test]
    async fn reports_errors_invalid_sequences_and_disconnects() {
        let (_, report) = drain(
            frames(&[
                r#"{"type":"RUN_STARTED","threadId":"t","runId":"r"}"#,
                r#"{"type":"RUN_ERROR","message":"boom"}"#,
            ]),
            false,
        )
        .await;
        assert_eq!(
            (report.outcome, report.error.as_deref()),
            ("error", Some("boom"))
        );

        let (_, report) = drain(
            frames(&[r#"{"type":"TEXT_MESSAGE_END","messageId":"m"}"#]),
            false,
        )
        .await;
        assert_eq!(report.outcome, "invalid");
        assert!(
            report
                .error
                .as_deref()
                .is_some_and(|e| e.contains("before RUN_STARTED"))
        );

        let (_, report) = drain(
            frames(&[r#"{"type":"RUN_STARTED","threadId":"t","runId":"r"}"#]),
            false,
        )
        .await;
        assert_eq!(report.outcome, "incomplete");

        let (_, report) = drain(
            frames(&[
                r#"{"type":"RUN_STARTED","threadId":"t","runId":"r"}"#,
                r#"{"type":"RUN_FINISHED","threadId":"t","runId":"r"}"#,
            ]),
            true,
        )
        .await;
        assert_eq!(report.outcome, "disconnected");
        assert_eq!(report.events, 1);
    }
}
