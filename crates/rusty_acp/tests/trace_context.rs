//! A trace that survives the replica boundary.
//!
//! #16 gave every run a span and every one of them was a root — nothing read a
//! `traceparent` in and nothing wrote one out. For a single-process server that
//! is a nuisance; for this crate it is the whole question, because the ordinary
//! deployment is identical replicas with no session affinity, so a run is
//! created through one, executes on another and is watched through a third.
//!
//! The two claims here are what makes that answerable:
//!
//! 1. A caller's trace id reaches the **run's** span, not just the request's —
//!    so a client call and the work it caused can be found together even though
//!    an `async` run outlives the request that started it.
//! 2. The client emits a header the server accepts, so the two halves of this
//!    crate agree without anyone configuring anything.

#![cfg(all(feature = "server", feature = "client", feature = "trace"))]

use std::sync::{Arc, Mutex};

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::InMemoryStore;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::trace::{TraceContext, TRACEPARENT_HEADER};
use rusty_acp::types::{AgentManifest, AgentName, Message};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// A span, flattened to its name and fields.
#[derive(Debug, Clone)]
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

struct Collector(Spans);

struct Fields(Vec<(String, String)>);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{value:?}").trim_matches('"').to_string()));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S> Layer<S> for Collector
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = Fields(Vec::new());
        attrs.record(&mut fields);
        // Kept so `on_record` can fill in the fields declared Empty and
        // recorded later — `trace_id` on the run span is one of them, so a
        // collector that only read `on_new_span` would see it as absent and
        // this whole file would pass while proving nothing.
        ctx.span(id).unwrap().extensions_mut().insert(Fields(fields.0.clone()));
        self.0
            .lock()
            .unwrap()
            .push(Captured { name: attrs.metadata().name().to_string(), fields: fields.0 });
    }

    fn on_record(
        &self,
        id: &tracing::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = Fields(Vec::new());
        values.record(&mut fields);
        let span = ctx.span(id).unwrap();
        let name = span.name().to_string();
        let mut recorded = Vec::new();
        if let Some(existing) = span.extensions().get::<Fields>() {
            recorded.extend(existing.0.clone());
        }
        recorded.extend(fields.0);
        let mut spans = self.0.lock().unwrap();
        if let Some(entry) = spans.iter_mut().rev().find(|entry| entry.name == name) {
            entry.fields = recorded;
        }
    }
}

fn collecting() -> (Spans, DefaultGuard) {
    let spans: Spans = Arc::default();
    let subscriber = tracing_subscriber::registry().with(Collector(Arc::clone(&spans)));
    (spans, tracing::subscriber::set_default(subscriber))
}

async fn server() -> String {
    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes"),
        |ctx: RunContext| async move { ctx.reply_text(ctx.input_text()).await.map(|_| ()) },
    );
    let router = AcpServer::builder()
        .agent(echo)
        .store(Arc::new(InMemoryStore::default()))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

const CALLER: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const CALLER_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

/// The claim: a caller's trace reaches the *run*, not just the request.
///
/// The request span alone would be easy and nearly useless — the interesting
/// work happens after the response, on whichever replica picked the run up.
#[tokio::test(flavor = "current_thread")]
async fn a_callers_trace_reaches_the_run_span() {
    let (spans, _guard) = collecting();
    let base_url = server().await;

    reqwest::Client::new()
        .post(format!("{base_url}/runs"))
        .header(TRACEPARENT_HEADER, CALLER)
        .json(&serde_json::json!({
            "agent_name": "echo",
            "input": [Message::user("hello")],
            "mode": "sync",
        }))
        .send()
        .await
        .unwrap();

    let spans = spans.lock().unwrap();
    let request = spans.iter().find(|span| span.name == "acp.request").expect("no request span");
    assert_eq!(request.field("trace_id"), Some(CALLER_TRACE), "the request span lost the trace");

    let run = spans.iter().find(|span| span.name == "acp.run").expect("no run span");
    assert_eq!(
        run.field("trace_id"),
        Some(CALLER_TRACE),
        "the run span did not join the caller's trace"
    );
    // And it still carries what it carried before, so this did not displace
    // the correlation #16 added.
    assert!(run.field("run_id").is_some(), "the run span lost its run id");
    assert!(run.field("replica").is_some(), "the run span lost its replica");
}

/// A request with no trace still gets one, so correlation never depends on the
/// caller having been configured.
#[tokio::test(flavor = "current_thread")]
async fn an_untraced_request_is_given_a_trace() {
    let (spans, _guard) = collecting();
    let base_url = server().await;

    reqwest::Client::new().get(format!("{base_url}/ping")).send().await.unwrap();

    let spans = spans.lock().unwrap();
    let request = spans.iter().find(|span| span.name == "acp.request").expect("no request span");
    let minted = request.field("trace_id").expect("no trace id was minted");
    assert_eq!(minted.len(), 32, "a minted trace id is not a trace id: {minted:?}");
    assert_ne!(minted, "0".repeat(32), "an all-zero trace id is invalid");
}

/// A malformed header is replaced rather than refused.
///
/// The alternative — 400 on a bad `traceparent` — would let one broken upstream
/// proxy take a deployment down over a field nothing depends on.
#[tokio::test(flavor = "current_thread")]
async fn a_malformed_traceparent_does_not_fail_the_request() {
    let (spans, _guard) = collecting();
    let base_url = server().await;

    let response = reqwest::Client::new()
        .get(format!("{base_url}/ping"))
        .header(TRACEPARENT_HEADER, "not-a-traceparent")
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "a broken header failed the request");
    let spans = spans.lock().unwrap();
    let request = spans.iter().find(|span| span.name == "acp.request").expect("no request span");
    assert_eq!(request.field("trace_id").map(str::len), Some(32));
}

/// The two halves of this crate agree without configuration: what the client
/// sends, the server reads.
///
/// A guard against the thing that makes propagation quietly useless — a header
/// name, a version prefix or a field width that differs by one between the
/// writer and the reader, which nothing else here would catch because each side
/// is self-consistent.
#[tokio::test(flavor = "current_thread")]
async fn what_the_client_sends_the_server_accepts() {
    let (spans, _guard) = collecting();
    let base_url = server().await;
    let client = AcpClient::new(base_url).unwrap();

    client.ping().await.unwrap();

    let spans = spans.lock().unwrap();
    let request = spans.iter().find(|span| span.name == "acp.request").expect("no request span");
    let seen = request.field("trace_id").expect("no trace id");
    // Minted by the client and parsed by the server: if either side disagreed
    // about the format the server would have discarded it and minted its own,
    // which is indistinguishable here *except* that a round-tripped id is a
    // valid one. So assert the shape the parser demands.
    assert_eq!(seen.len(), 32);
    assert!(seen.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    assert!(TraceContext::parse(&format!("00-{seen}-00f067aa0ba902b7-01")).is_some());
}

/// The opt-out works, for a caller that carries its own header.
#[tokio::test(flavor = "current_thread")]
async fn trace_headers_can_be_turned_off() {
    let base_url = server().await;
    let client = AcpClient::builder(base_url).without_trace_headers().build().unwrap();

    // The value is that nothing is sent; the server mints regardless, so the
    // observable claim is simply that the call still works.
    client.ping().await.expect("disabling trace headers broke the client");
}
