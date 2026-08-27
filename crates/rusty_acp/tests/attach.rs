//! Watching a run this client did not start.
//!
//! The server has served a resumable SSE stream at `GET /runs/{id}/events`
//! since #13, and the client has used it — but only from inside its private
//! reconnection path. A caller who started a run with `run_async`, or was handed
//! a run id by something else, could only poll.
//!
//! The property worth the most attention is the one that differs from
//! `stream_run`: attaching knows the run id before the first byte arrives, so a
//! connection that drops before any event can still be resumed. `stream_run`
//! learns the id from the first `run.*` event and cannot. That is tested here
//! against a hand-written server, because a working one will not drop a
//! response on request.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rusty_acp::client::{collect_run, AcpClient, ReconnectPolicy};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Event, Message, Run, RunId, RunStatus};
use tokio::sync::{mpsc, oneshot, Mutex};

/// How many parts the streaming agent emits.
const PARTS: usize = 6;

struct Fixture {
    client: AcpClient,
    /// Signalled once the agent has emitted its first part, so a test attaches
    /// to a run that is provably under way rather than one it hopes is.
    started: Mutex<mpsc::UnboundedReceiver<()>>,
    /// Lets the agent finish. Until sent, the run cannot reach a terminal
    /// state, so "attached mid-run" is a fact rather than a race.
    release: Mutex<Option<oneshot::Sender<()>>>,
}

impl Fixture {
    async fn new() -> Self {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = oneshot::channel();
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));

        let writer = agent_fn(
            AgentManifest::new(AgentName::new("writer").unwrap(), "Emits parts, then waits"),
            move |ctx: RunContext| {
                let started = started_tx.clone();
                let release = Arc::clone(&release_rx);
                async move {
                    let mut message = ctx.begin_message().await?;
                    message.push_text("part-0 ").await?;
                    let _ = started.send(());
                    if let Some(release) = release.lock().await.take() {
                        let _ = release.await;
                    }
                    for index in 1..PARTS {
                        message.push_text(format!("part-{index} ")).await?;
                    }
                    message.finish().await?;
                    Ok(())
                }
            },
        );

        let router = AcpServer::builder()
            .agent(writer)
            .base_url("http://acp.example")
            .build()
            .unwrap()
            .into_router();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self {
            client: AcpClient::new(format!("http://{addr}")).unwrap(),
            started: Mutex::new(started_rx),
            release: Mutex::new(Some(release_tx)),
        }
    }

    /// Start a run and return once its agent has emitted something.
    async fn start(&self) -> Run {
        let run = self.client.run_async("writer", [Message::user("go")]).await.unwrap();
        self.started.lock().await.recv().await.expect("the agent starts");
        run
    }

    async fn release(&self) {
        if let Some(release) = self.release.lock().await.take() {
            let _ = release.send(());
        }
    }
}

/// The gap the issue named: a run started elsewhere, watched live.
#[tokio::test]
async fn a_run_started_elsewhere_can_be_watched() {
    let fixture = Fixture::new().await;
    let run = fixture.start().await;

    let stream = fixture.client.attach(run.run_id).await.expect("attached");

    // Released only once attached, so everything after this point arrives live
    // rather than out of the replayed log.
    fixture.release().await;

    let events: Vec<Event> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .expect("no error reaches the caller");

    assert!(events.last().is_some_and(Event::is_terminal), "ended without a terminal: {events:?}");
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            Event::MessagePart { part } => part.as_text().map(str::to_string),
            _ => None,
        })
        .collect();
    for index in 0..PARTS {
        assert!(text.contains(&format!("part-{index}")), "missing part-{index} in {text:?}");
    }
}

/// Attaching to a finished run replays its log and closes, which is the useful
/// answer — the caller gets the whole run rather than an error saying they were
/// too late.
#[tokio::test]
async fn attaching_to_a_finished_run_replays_it() {
    let fixture = Fixture::new().await;
    let run = fixture.start().await;
    fixture.release().await;
    let finished = fixture.client.wait_for_run(run.run_id, Default::default()).await.unwrap();
    assert_eq!(finished.status, RunStatus::Completed);

    let stream = fixture.client.attach(run.run_id).await.expect("attached after the fact");
    let replayed = collect_run(stream).await.expect("the log carries a terminal run event");

    assert_eq!(replayed.run_id, run.run_id);
    assert_eq!(replayed.status, RunStatus::Completed);
}

/// `attach_after` skips what the caller has already read.
#[tokio::test]
async fn attaching_after_an_index_skips_what_came_before() {
    let fixture = Fixture::new().await;
    let run = fixture.start().await;
    fixture.release().await;
    fixture.client.wait_for_run(run.run_id, Default::default()).await.unwrap();

    let whole: Vec<Event> = fixture
        .client
        .attach(run.run_id)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .unwrap();

    // Everything after the third event, by the index the server tags.
    let rest: Vec<Event> = fixture
        .client
        .attach_after(run.run_id, 2)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(rest.len(), whole.len() - 3, "expected exactly the first three to be skipped");
    assert_eq!(rest.first(), whole.get(3));
    assert!(rest.last().is_some_and(Event::is_terminal));
}

#[tokio::test]
async fn attaching_to_an_unknown_run_is_an_error() {
    let fixture = Fixture::new().await;

    let error = fixture.client.attach(RunId::new()).await.err().expect("no such run");

    assert!(error.is_not_found(), "{error}");
}

/// The property `stream_run` cannot have.
///
/// The response is cut off before a single event arrives. Attaching knows the
/// run id already, so it can ask the log for the same run again; `stream_run`
/// would have nothing to reconnect *to*, because the id is carried by the first
/// event and the first event never came.
///
/// Served by hand: a working server does not truncate a response on request.
#[tokio::test]
async fn a_drop_before_the_first_event_is_still_resumable() {
    use axum::response::IntoResponse;

    let run = Run::new(AgentName::new("writer").unwrap(), None);
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&attempts);
    let served = run.clone();

    let router = axum::Router::new().route(
        "/runs/{run_id}/events",
        axum::routing::get(move || {
            let attempt = seen.fetch_add(1, Ordering::SeqCst);
            let run = served.clone();
            async move {
                if attempt == 0 {
                    // Headers, then nothing: the stream is attached and then
                    // dropped without ever delivering an event.
                    return ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], "")
                        .into_response();
                }
                let completed = Run { status: RunStatus::Completed, ..run };
                let body = format!(
                    "id: 0\nevent: run.completed\ndata: {}\n\n",
                    serde_json::to_string(&Event::RunCompleted { run: Box::new(completed) })
                        .unwrap()
                );
                ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], body).into_response()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = AcpClient::builder(format!("http://{addr}"))
        .reconnect(ReconnectPolicy {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(10),
        })
        .build()
        .unwrap();

    let stream = client.attach(run.run_id).await.expect("attached");
    let events: Vec<Event> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .expect("the drop is resumed, not surfaced");

    assert!(
        events.last().is_some_and(Event::is_terminal),
        "the stream gave up on a drop it had the run id to recover from: {events:?}"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2, "one truncated attempt, then one that worked");
}

/// A drop before the first event must resume from where the caller asked, not
/// from the beginning.
///
/// The starting index reaches the server in the request header, so the *first*
/// attempt is right whether or not the client remembers it. Only a reconnection
/// that happens before any event has arrived — with no index learned from the
/// stream to fall back on — depends on the client having kept it. Without that,
/// a caller resuming at event 900 silently gets the whole log again.
#[tokio::test]
async fn a_drop_before_the_first_event_resumes_from_where_it_was_asked_to() {
    use axum::response::IntoResponse;

    const RESUME_FROM: u64 = 41;

    let run = Run::new(AgentName::new("writer").unwrap(), None);
    // Every `last-event-id` the server was sent, in order.
    let asked: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let recorder = Arc::clone(&asked);
    let served = run.clone();

    let router = axum::Router::new().route(
        "/runs/{run_id}/events",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let attempt = {
                let mut asked = recorder.lock().unwrap();
                asked.push(
                    headers
                        .get("last-event-id")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                );
                asked.len() - 1
            };
            let run = served.clone();
            async move {
                if attempt == 0 {
                    return ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], "")
                        .into_response();
                }
                let completed = Run { status: RunStatus::Completed, ..run };
                let body = format!(
                    "id: 42\nevent: run.completed\ndata: {}\n\n",
                    serde_json::to_string(&Event::RunCompleted { run: Box::new(completed) })
                        .unwrap()
                );
                ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], body).into_response()
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = AcpClient::builder(format!("http://{addr}"))
        .reconnect(ReconnectPolicy {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(10),
        })
        .build()
        .unwrap();

    let stream = client.attach_after(run.run_id, RESUME_FROM).await.expect("attached");
    let events: Vec<Event> =
        stream.collect::<Vec<_>>().await.into_iter().collect::<Result<_, _>>().unwrap();
    assert!(events.last().is_some_and(Event::is_terminal), "{events:?}");

    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![Some(RESUME_FROM.to_string()), Some(RESUME_FROM.to_string())],
        "the reconnection forgot where the caller had asked to start"
    );
}
