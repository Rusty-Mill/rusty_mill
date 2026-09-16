//! Resuming an event stream that dropped.
//!
//! The interesting part is not the replay — it is the seam where the replay
//! meets the live subscription. The server subscribes before reading the log,
//! which rules out a gap but guarantees an *overlap*: anything appended in
//! between arrives on both paths. Whether that overlap is deduped exactly is
//! what these tests are about.
//!
//! As with `ordering.rs`, the seam is made observable rather than raced. A
//! store whose log reads are slow holds the window open for a fixed 300ms, so a
//! run emitting throughout is certain to land events in both halves. A splice
//! that double-counts or drops them fails every time instead of occasionally.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{
    InMemoryStore, Notification, NotificationStream, RecoveryRecord, SessionRecord, Store,
    StoreResult,
};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, Event, Message, Run, RunId, RunStatus, Session, SessionId,
};

/// How long a log read is made to take. The agent emits throughout, so the
/// overlap between replay and live subscription is guaranteed rather than
/// likely.
const READ_DELAY: Duration = Duration::from_millis(300);

/// How many parts the `chatty` agent emits, and how far apart.
const PARTS: usize = 12;
const PART_GAP: Duration = Duration::from_millis(40);

/// An [`InMemoryStore`] whose log reads are slow.
///
/// Only `events_from` is touched — that is the read a resuming stream makes
/// between subscribing and attaching, and so the only place the overlap window
/// can be widened.
#[derive(Debug)]
struct SlowReadStore(InMemoryStore);

#[async_trait::async_trait]
impl Store for SlowReadStore {
    async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
        tokio::time::sleep(READ_DELAY).await;
        self.0.events_from(run_id, from).await
    }

    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        self.0.put_run(run).await
    }
    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        self.0.get_run(run_id).await
    }
    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
        self.0.append_event(run_id, event).await
    }
    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        self.0.events(run_id).await
    }
    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        self.0.publish(run_id, notification).await
    }
    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        self.0.subscribe(run_id).await
    }
    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        self.0.get_session(session_id).await
    }
    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        self.0.ensure_session(session).await
    }
    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        self.0.append_session_messages(session_id, base_url, messages).await
    }
    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        self.0.get_session_state(session_id).await
    }
    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        self.0.put_session_state(session_id, base_url, state).await
    }
    async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
        self.0.renew_lease(run_id, owner, ttl).await
    }
    async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
        self.0.lease_owner(run_id).await
    }
    async fn try_claim_lease(
        &self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> StoreResult<bool> {
        self.0.try_claim_lease(run_id, owner, ttl).await
    }
    async fn put_recovery_record(
        &self,
        run_id: RunId,
        record: Option<&RecoveryRecord>,
    ) -> StoreResult<()> {
        self.0.put_recovery_record(run_id, record).await
    }
    async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
        self.0.recovery_record(run_id).await
    }
    async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
        self.0.release_lease(run_id).await
    }
}

struct Harness {
    client: AcpClient,
    base_url: String,
    http: reqwest::Client,
}

async fn start_server() -> Harness {
    // Emits steadily, so events land on both sides of a slow log read.
    let chatty = agent_fn(
        AgentManifest::new(AgentName::new("chatty").unwrap(), "Emits a run's worth of parts"),
        |ctx: RunContext| async move {
            let mut writer = ctx.begin_message().await?;
            for index in 0..PARTS {
                writer.push_text(format!("part-{index} ")).await?;
                tokio::time::sleep(PART_GAP).await;
            }
            writer.finish().await?;
            Ok(())
        },
    );

    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(chatty)
        .agent(echo)
        .store(Arc::new(SlowReadStore(InMemoryStore::new(1024))))
        .build()
        .unwrap()
        .into_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let base_url = format!("http://{addr}");
    Harness {
        client: AcpClient::new(base_url.clone()).unwrap(),
        base_url,
        http: reqwest::Client::new(),
    }
}

impl Harness {
    /// Open an SSE stream on a run's log, optionally resuming after `last`.
    ///
    /// Returns `(id, event_type)` pairs — the id is what a resuming client
    /// hands back, so it is the thing worth asserting on.
    async fn stream_events(&self, run_id: RunId, last: Option<u64>) -> Vec<(Option<u64>, String)> {
        let mut request = self
            .http
            .get(format!("{}/runs/{run_id}/events", self.base_url))
            .header("accept", "text/event-stream");
        if let Some(last) = last {
            request = request.header("last-event-id", last.to_string());
        }

        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert!(response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"));

        let mut collected = Vec::new();
        let mut stream = response.bytes_stream().eventsource();
        while let Some(message) = stream.next().await {
            let message = message.unwrap();
            let id = message.id.trim().parse::<u64>().ok();
            collected.push((id, message.event.clone()));
            if is_terminal(&message.event) {
                break;
            }
        }
        collected
    }
}

fn is_terminal(event_type: &str) -> bool {
    matches!(event_type, "run.completed" | "run.failed" | "run.cancelled" | "run.awaiting")
}

/// Ids must be dense and strictly increasing: one assertion that catches both a
/// gap in the splice and a duplicate across it.
fn assert_contiguous_from(events: &[(Option<u64>, String)], first: u64) {
    let ids: Vec<u64> = events.iter().filter_map(|(id, _)| *id).collect();
    let expected: Vec<u64> = (first..first + ids.len() as u64).collect();
    assert_eq!(ids, expected, "event ids must be dense and in order, with nothing repeated");
    assert_eq!(
        ids.len(),
        events.len(),
        "every event on a run's log stream must carry an id, or a client cannot resume from it"
    );
}

/// The splice sends each event exactly once, with the log read held open long
/// enough that the overlap is certain.
#[tokio::test]
async fn a_resumed_stream_gets_every_event_exactly_once() {
    let harness = start_server().await;

    let started = harness.client.run_async("chatty", [Message::user("go")]).await.unwrap();
    let run_id = started.run_id;

    // Attach mid-run, from the very beginning of the log.
    let events = harness.stream_events(run_id, None).await;

    assert_contiguous_from(&events, 0);
    assert_eq!(events.first().unwrap().1, "run.created");
    assert_eq!(events.last().unwrap().1, "run.completed");
    assert_eq!(
        events.iter().filter(|(_, kind)| kind == "message.part").count(),
        PARTS,
        "every part the agent emitted must appear once"
    );
}

/// Resuming from an index sends what follows it, and nothing before it.
#[tokio::test]
async fn resuming_from_an_index_sends_only_what_follows() {
    let harness = start_server().await;

    let started = harness.client.run_async("chatty", [Message::user("go")]).await.unwrap();
    let run_id = started.run_id;

    let all = harness.stream_events(run_id, None).await;
    let resume_after = 3;

    // The run has finished by now, so this is a pure replay — the same path a
    // client takes when it reconnects after the run it was watching ended.
    let tail = harness.stream_events(run_id, Some(resume_after)).await;

    assert_contiguous_from(&tail, resume_after + 1);
    assert_eq!(
        tail.len(),
        all.len() - (resume_after as usize + 1),
        "resuming must send exactly the events after the one acknowledged"
    );
    assert_eq!(tail.last().unwrap().1, "run.completed");
}

/// A stream opened on a finished run replays it and closes, rather than holding
/// the connection open for events that can never come.
#[tokio::test]
async fn a_finished_run_replays_and_closes() {
    let harness = start_server().await;

    let run = harness.client.run_sync("echo", [Message::user("hello")]).await.unwrap();
    assert_eq!(run.status, RunStatus::Completed);

    // No terminal-event break here: this reads until the *server* closes the
    // stream, so a stream that lingered would hang the test rather than pass
    // it. The timeout is the assertion.
    let events = tokio::time::timeout(Duration::from_secs(5), async {
        let response = harness
            .http
            .get(format!("{}/runs/{}/events", harness.base_url, run.run_id))
            .header("accept", "text/event-stream")
            .send()
            .await
            .unwrap();

        let mut collected = Vec::new();
        let mut stream = response.bytes_stream().eventsource();
        while let Some(message) = stream.next().await {
            collected.push(message.unwrap().event);
        }
        collected
    })
    .await
    .expect("a stream on a finished run must close on its own");

    assert_eq!(events.first().unwrap(), "run.created");
    assert_eq!(events.last().unwrap(), "run.completed");
}

/// A server that hangs up mid-stream, so the client's reconnection can be
/// observed instead of waited for.
///
/// Cutting a real connection from the outside is not something a test can do
/// reliably, and retrying until it happens would be exactly the racing
/// `CLAUDE.md` warns off. This stands in for the proxy timeout or recycled
/// connection that does it in production: the first response ends after two
/// events with no terminal event, which is indistinguishable, to the client,
/// from the pipe being cut.
mod hangup {
    use axum::response::sse::{Event as SseEvent, Sse};
    use axum::routing::{get, post};
    use axum::{extract::State, Json, Router};
    use rusty_acp::types::{AgentName, Event, MessagePart, Run, RunStatus};
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    /// The `Last-Event-ID` values the server was asked to resume from.
    pub type Resumes = Arc<Mutex<Vec<Option<String>>>>;

    /// Serialised through the crate's own types, so the stub cannot drift from
    /// the wire format it is standing in for.
    fn sse(id: u64, event: Event) -> SseEvent {
        SseEvent::default()
            .id(id.to_string())
            .event(event.event_type())
            .data(serde_json::to_string(&event).unwrap())
    }

    fn run(status: RunStatus) -> Box<Run> {
        let mut run = Run::new(AgentName::new("stub").unwrap(), None);
        run.status = status;
        Box::new(run)
    }

    pub async fn start() -> (String, Resumes) {
        let resumes: Resumes = Arc::new(Mutex::new(Vec::new()));

        let router = Router::new()
            // Two events, then the response simply ends — no terminal event.
            .route(
                "/runs",
                post(|| async {
                    Sse::new(futures_util::stream::iter(vec![
                        Ok::<_, Infallible>(sse(
                            0,
                            Event::RunCreated { run: run(RunStatus::Created) },
                        )),
                        Ok(sse(1, Event::RunInProgress { run: run(RunStatus::InProgress) })),
                    ]))
                }),
            )
            // What the client asks for when it notices the hang-up.
            .route(
                "/runs/{run_id}/events",
                get(|State(resumes): State<Resumes>, headers: axum::http::HeaderMap| async move {
                    let last = headers
                        .get("last-event-id")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    resumes.lock().unwrap().push(last);

                    Sse::new(futures_util::stream::iter(vec![
                        Ok::<_, Infallible>(sse(
                            2,
                            Event::MessagePart { part: MessagePart::text("resumed") },
                        )),
                        Ok(sse(3, Event::RunCompleted { run: run(RunStatus::Completed) })),
                    ]))
                }),
            )
            .route("/ping", get(|| async { Json(serde_json::json!({})) }))
            .with_state(Arc::clone(&resumes));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        (format!("http://{addr}"), resumes)
    }
}

/// A stream cut short is picked up again, from the last event the client saw.
#[tokio::test]
async fn the_client_resumes_a_dropped_stream() {
    let (base_url, resumes) = hangup::start().await;
    let client = AcpClient::new(base_url).unwrap();

    let mut stream = client.stream("stub", [Message::user("go")]).await.unwrap();

    let mut kinds = Vec::new();
    while let Some(event) = stream.next().await {
        kinds.push(event_kind(&event.unwrap()));
    }

    assert_eq!(
        kinds,
        ["run.created", "run.in-progress", "message.part", "run.completed"],
        "the events after the hang-up must arrive as if nothing happened"
    );

    let resumes = resumes.lock().unwrap();
    assert_eq!(resumes.len(), 1, "exactly one reconnection");
    assert_eq!(
        resumes[0].as_deref(),
        Some("1"),
        "the client must resume after the last event it actually saw"
    );
}

/// With resumption switched off, the same hang-up ends the stream.
#[tokio::test]
async fn reconnection_can_be_turned_off() {
    let (base_url, resumes) = hangup::start().await;
    let client = AcpClient::builder(base_url)
        .reconnect(rusty_acp::client::ReconnectPolicy::disabled())
        .build()
        .unwrap();

    let mut stream = client.stream("stub", [Message::user("go")]).await.unwrap();
    let mut kinds = Vec::new();
    while let Some(event) = stream.next().await {
        kinds.push(event_kind(&event.unwrap()));
    }

    assert_eq!(kinds, ["run.created", "run.in-progress"]);
    assert!(resumes.lock().unwrap().is_empty(), "a disabled policy must not reconnect");
}

fn event_kind(event: &Event) -> &'static str {
    match event {
        Event::RunCreated { .. } => "run.created",
        Event::RunInProgress { .. } => "run.in-progress",
        Event::MessagePart { .. } => "message.part",
        Event::MessageCompleted { .. } => "message.completed",
        Event::RunCompleted { .. } => "run.completed",
        other => panic!("unexpected event: {other:?}"),
    }
}

/// Without `Accept: text/event-stream`, the endpoint is the JSON list the
/// specification describes. The streaming form is an extension and must not
/// become the default.
#[tokio::test]
async fn the_endpoint_is_still_a_json_list_by_default() {
    let harness = start_server().await;

    let run = harness.client.run_sync("echo", [Message::user("hello")]).await.unwrap();

    let events = harness.client.list_run_events(run.run_id).await.unwrap();
    assert!(events.iter().any(|event| matches!(event, Event::RunCompleted { .. })));

    let raw = harness
        .http
        .get(format!("{}/runs/{}/events", harness.base_url, run.run_id))
        .send()
        .await
        .unwrap();
    assert_eq!(raw.headers().get("content-type").unwrap(), "application/json");
}
