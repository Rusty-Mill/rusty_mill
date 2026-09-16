//! What a run's span carries, and what it covers.
//!
//! The point of the span is correlation: an agent's own log output is emitted
//! from inside `agent.run`, so without a span wrapping that call it interleaves
//! with every other concurrent run and cannot be told apart afterwards. Two
//! things therefore have to hold, and neither is obvious from reading the code:
//! the span must carry the fields that identify the run, and it must still be
//! current *inside the agent*.
//!
//! Asserted by collecting spans with a subscriber rather than by eye, because
//! the failure mode is silent — a span that covers the wrong scope still logs,
//! just uncorrelated.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::{Arc, Mutex};

use rusty_acp::client::AcpClient;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Message, RunStatus, SessionId};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// One span, flattened to its name and fields.
#[derive(Debug, Clone, PartialEq)]
struct Captured {
    name: String,
    fields: Vec<(String, String)>,
}

impl Captured {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }
}

type Spans = Arc<Mutex<Vec<Captured>>>;

/// An event's message, paired with the span names current when it fired.
type EventScopes = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// Records every span created, and every span that was current when an event
/// was emitted.
struct SpanCollector {
    spans: Spans,
    /// Spans that were current at the moment an event fired, by event message.
    at_event: EventScopes,
}

struct FieldVisitor(Vec<(String, String)>);

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{value:?}").trim_matches('"').to_string()));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S> Layer<S> for SpanCollector
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor(Vec::new());
        attrs.record(&mut visitor);
        self.spans.lock().unwrap().push(Captured {
            name: attrs.metadata().name().to_string(),
            fields: visitor.0.clone(),
        });
        // Keep the fields on the span itself so `record` calls made later, and
        // lookups at event time, both see them.
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(FieldStore(visitor.0));
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldVisitor(Vec::new());
        values.record(&mut visitor);
        let Some(span) = ctx.span(id) else { return };

        let name = span.name().to_string();
        let mut extensions = span.extensions_mut();
        if let Some(FieldStore(fields)) = extensions.get_mut::<FieldStore>() {
            fields.extend(visitor.0.clone());
            let merged = fields.clone();
            drop(extensions);
            // Replace the recorded copy so late-recorded fields are visible.
            let mut spans = self.spans.lock().unwrap();
            if let Some(entry) = spans.iter_mut().rev().find(|entry| entry.name == name) {
                entry.fields = merged;
            }
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = FieldVisitor(Vec::new());
        event.record(&mut visitor);
        let message = visitor
            .0
            .iter()
            .find(|(key, _)| key == "message")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();

        let current: Vec<String> = ctx
            .event_scope(event)
            .map(|scope| scope.from_root().map(|span| span.name().to_string()).collect())
            .unwrap_or_default();
        self.at_event.lock().unwrap().push((message, current));
    }
}

struct FieldStore(Vec<(String, String)>);

/// Install a collector for the duration of the returned guard.
fn collect() -> (Spans, EventScopes, DefaultGuard) {
    let spans: Spans = Arc::new(Mutex::new(Vec::new()));
    let at_event: EventScopes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry()
        .with(SpanCollector { spans: Arc::clone(&spans), at_event: Arc::clone(&at_event) });
    let guard = tracing::subscriber::set_default(subscriber);
    (spans, at_event, guard)
}

async fn start_server() -> AcpClient {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            // The line that matters: emitted from inside the agent, and only
            // useful to an operator if it lands under the run's span.
            tracing::info!("agent speaking");
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(echo)
        .replica_id("replica-under-test")
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    AcpClient::new(format!("http://{addr}")).unwrap()
}

fn run_span(spans: &Spans) -> Captured {
    spans
        .lock()
        .unwrap()
        .iter()
        .find(|span| span.name == "acp.run")
        .cloned()
        .expect("a run must open an `acp.run` span")
}

/// The span identifies the run, the agent and the replica.
#[tokio::test]
async fn a_run_span_carries_the_run_agent_and_replica() {
    let (spans, _events, _guard) = collect();
    let client = start_server().await;

    let run = client.run_sync("echo", [Message::user("hello")]).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    let span = run_span(&spans);
    assert_eq!(span.field("run_id"), Some(run.run_id.to_string().as_str()));
    assert_eq!(span.field("agent"), Some("echo"));
    assert_eq!(span.field("replica"), Some("replica-under-test"));
}

/// A run without a session records no `session_id`, rather than the string
/// "None".
#[tokio::test]
async fn a_run_without_a_session_has_no_session_field() {
    let (spans, _events, _guard) = collect();
    let client = start_server().await;

    client.run_sync("echo", [Message::user("hello")]).await.unwrap();

    let span = run_span(&spans);
    assert_eq!(
        span.field("session_id"),
        None,
        "an absent session should be an absent field, not a literal None"
    );
}

/// A run in a session records it, so every line from that run can be traced
/// back to the conversation it belongs to.
#[tokio::test]
async fn a_run_in_a_session_records_the_session() {
    use rusty_acp::types::RunCreateRequest;

    let (spans, _events, _guard) = collect();
    let client = start_server().await;
    let session_id = SessionId::new();

    client
        .create_run(
            RunCreateRequest::new(AgentName::new("echo").unwrap(), [Message::user("hello")])
                .with_session_id(session_id),
        )
        .await
        .unwrap();

    let span = run_span(&spans);
    assert_eq!(span.field("session_id"), Some(session_id.to_string().as_str()));
}

/// The span is current *inside the agent*, which is the whole point.
///
/// A span that opened and closed around the setup would satisfy every field
/// assertion above and still leave the agent's own output uncorrelated.
#[tokio::test]
async fn the_span_is_current_inside_the_agent() {
    let (_spans, events, _guard) = collect();
    let client = start_server().await;

    client.run_sync("echo", [Message::user("hello")]).await.unwrap();

    let events = events.lock().unwrap();
    let (_, scope) = events
        .iter()
        .find(|(message, _)| message == "agent speaking")
        .expect("the agent's own log line must have been captured");

    assert!(
        scope.iter().any(|name| name == "acp.run"),
        "an agent's output must be emitted inside the run's span, got scope {scope:?}"
    );
}
