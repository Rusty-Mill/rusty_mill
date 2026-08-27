//! Retrying transient failures.
//!
//! Every test here drives a stub server that is scripted to fail a fixed number
//! of times and then succeed, and asserts on the **count of requests it saw**.
//! That is deliberate: a test that only asserted the call returned `Ok` would
//! pass just as happily against a client that never retried and a server that
//! never failed. The counter is what distinguishes "retried" from "got lucky".
//!
//! Nothing here races. The stub's script is a counter under a mutex, and each
//! test drives one client, so the number of attempts is a function of the
//! policy rather than of how the runtime happened to interleave.

#![cfg(feature = "client")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use rusty_acp::client::{AcpClient, ReconnectPolicy, RetryPolicy, WaitOptions};
use rusty_acp::types::{
    AgentName, Event, Message, MessagePart, Run, RunCreateRequest, RunId, RunStatus,
};

/// A server that fails a scripted number of times per path, then succeeds.
struct Stub {
    /// How many times each path fails before it starts answering.
    fail_first: usize,
    /// The status those failures carry.
    status: StatusCode,
    /// A `Retry-After` value to attach to them, if any.
    retry_after: Option<String>,
    /// What a successful request returns.
    run: Run,
    /// Requests seen, keyed by path.
    calls: Mutex<HashMap<String, usize>>,
}

impl Stub {
    fn new(fail_first: usize, status: StatusCode) -> Self {
        let mut run = Run::new(AgentName::new("stub").unwrap(), None);
        run.status = RunStatus::Completed;
        Self { fail_first, status, retry_after: None, run, calls: Mutex::new(HashMap::new()) }
    }

    fn retry_after(mut self, value: &str) -> Self {
        self.retry_after = Some(value.to_string());
        self
    }

    fn calls_to(&self, path: &str) -> usize {
        self.calls.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

/// Serve `stub` on an ephemeral port and return a client pointed at it.
async fn serve(stub: Arc<Stub>, retry: RetryPolicy) -> AcpClient {
    async fn handle(State(stub): State<Arc<Stub>>, request: Request) -> Response {
        let seen = {
            let mut calls = stub.calls.lock().unwrap();
            let seen = calls.entry(request.uri().path().to_string()).or_insert(0);
            *seen += 1;
            *seen
        };

        if seen > stub.fail_first {
            return Json(stub.run.clone()).into_response();
        }
        // A plain-text body on purpose: a load balancer's 503 is not an ACP
        // error object, and the client has to classify it from the status.
        let mut response = (stub.status, "unavailable").into_response();
        if let Some(value) = &stub.retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, value.parse().unwrap());
        }
        response
    }

    let router = axum::Router::new().fallback(axum::routing::any(handle)).with_state(stub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    AcpClient::builder(format!("http://{addr}")).retry(retry).build().unwrap()
}

/// A policy that retries without waiting, so the tests measure attempts rather
/// than sleeping through backoff.
fn prompt(max_retries: u32) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        initial_backoff: Duration::from_millis(1),
        jitter: 0.0,
        ..RetryPolicy::default()
    }
}

#[tokio::test]
async fn a_read_is_retried_until_it_succeeds() {
    let stub = Arc::new(Stub::new(2, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), prompt(3)).await;

    let run_id = RunId::new();
    let run = client.get_run(run_id).await.expect("the third attempt succeeds");

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 3);
}

/// The other statuses that mean "not now", so the classification is not just
/// 503 with a list around it.
#[tokio::test]
async fn the_whole_transient_set_is_retried() {
    for status in
        [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY, StatusCode::GATEWAY_TIMEOUT]
    {
        let stub = Arc::new(Stub::new(1, status));
        let client = serve(stub.clone(), prompt(3)).await;

        let run_id = RunId::new();
        client.get_run(run_id).await.unwrap_or_else(|err| panic!("{status} not retried: {err}"));
        assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 2, "{status}");
    }
}

/// 500 is what a server returns when the *agent* failed. Retrying reproduces it
/// and delays the error the caller needs, so it is excluded on purpose.
#[tokio::test]
async fn a_server_error_is_not_retried() {
    let stub = Arc::new(Stub::new(1, StatusCode::INTERNAL_SERVER_ERROR));
    let client = serve(stub.clone(), prompt(3)).await;

    let run_id = RunId::new();
    let error = client.get_run(run_id).await.expect_err("500 reaches the caller");

    assert!(matches!(error, rusty_acp::AcpError::Http { status: 500, .. }), "{error}");
    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 1);
}

/// The asymmetry the issue turned on: a submission that timed out may already
/// be running, and ACP has no idempotency key to collapse a second one into it.
#[tokio::test]
async fn creating_a_run_is_not_retried_by_default() {
    let stub = Arc::new(Stub::new(1, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), prompt(3)).await;

    let error = client
        .run_sync("stub", [Message::user("go")])
        .await
        .expect_err("a failed submission is not repeated");

    assert!(matches!(error, rusty_acp::AcpError::Http { status: 503, .. }), "{error}");
    assert_eq!(stub.calls_to("/runs"), 1);
}

#[tokio::test]
async fn creating_a_run_is_retried_when_the_caller_opts_in() {
    let stub = Arc::new(Stub::new(1, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), RetryPolicy { retry_run_submission: true, ..prompt(3) }).await;

    client.run_sync("stub", [Message::user("go")]).await.expect("the second attempt succeeds");

    assert_eq!(stub.calls_to("/runs"), 2);
}

/// Cancellation is retried despite being a POST: asking twice for a run to stop
/// is the same request, not a second one.
#[tokio::test]
async fn cancelling_is_retried() {
    let stub = Arc::new(Stub::new(2, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), prompt(3)).await;

    let run_id = RunId::new();
    client.cancel_run(run_id).await.expect("the third attempt succeeds");

    assert_eq!(stub.calls_to(&format!("/runs/{run_id}/cancel")), 3);
}

#[tokio::test]
async fn retries_are_bounded_and_the_server_error_survives() {
    let stub = Arc::new(Stub::new(usize::MAX, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), prompt(2)).await;

    let run_id = RunId::new();
    let error = client.get_run(run_id).await.expect_err("a server that never recovers");

    // The server's own answer, not an error invented by the retry loop.
    assert!(matches!(error, rusty_acp::AcpError::Http { status: 503, .. }), "{error}");
    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 3, "one attempt plus two retries");
}

#[tokio::test]
async fn retrying_can_be_switched_off() {
    let stub = Arc::new(Stub::new(1, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), RetryPolicy::disabled()).await;

    let run_id = RunId::new();
    client.get_run(run_id).await.expect_err("the first failure reaches the caller");

    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 1);
}

/// `Retry-After` overrides the policy's own backoff, which is observable
/// because the policy's is a millisecond and the header asks for a second.
#[tokio::test]
async fn retry_after_is_waited_out() {
    let stub = Arc::new(Stub::new(1, StatusCode::TOO_MANY_REQUESTS).retry_after("1"));
    let client = serve(stub.clone(), prompt(3)).await;

    let started = Instant::now();
    let run_id = RunId::new();
    client.get_run(run_id).await.expect("the second attempt succeeds");

    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 2);
    assert!(started.elapsed() >= Duration::from_secs(1), "the header was ignored");
}

/// A server that asks for longer than the ceiling is obeyed by giving up, not
/// by knocking again sooner than it asked.
#[tokio::test]
async fn a_retry_after_beyond_the_ceiling_ends_the_attempts() {
    let stub = Arc::new(Stub::new(1, StatusCode::SERVICE_UNAVAILABLE).retry_after("3600"));
    let client =
        serve(stub.clone(), RetryPolicy { max_backoff: Duration::from_secs(5), ..prompt(3) }).await;

    let started = Instant::now();
    let run_id = RunId::new();
    client.get_run(run_id).await.expect_err("the server asked for an hour");

    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 1);
    assert!(started.elapsed() < Duration::from_secs(1), "it waited rather than giving up");
}

/// The polling helpers' own tolerance, which is a separate thing from the retry
/// loop inside one request.
///
/// The stub outlasts a single `get_run`'s budget on purpose: with two attempts
/// per poll and five failures scripted, no one request can ride it out. Only a
/// wait that treats a failed poll as *not settled yet* gets to the answer.
#[tokio::test]
async fn waiting_rides_out_a_blip_that_outlasts_one_request() {
    let stub = Arc::new(Stub::new(5, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), prompt(1)).await;

    let run_id = RunId::new();
    let options = WaitOptions::default()
        .poll_every(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(10));
    let run = client.wait_for_run(run_id, options).await.expect("the wait outlasts the blip");

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 6);
}

/// When the wait does give up, it reports what it kept meeting rather than a
/// timeout that would hide it.
#[tokio::test]
async fn a_wait_that_gives_up_reports_the_failure() {
    let stub = Arc::new(Stub::new(usize::MAX, StatusCode::BAD_GATEWAY));
    let client = serve(stub.clone(), prompt(1)).await;

    let options = WaitOptions::default()
        .poll_every(Duration::from_millis(10))
        .with_timeout(Duration::from_millis(100));
    let error =
        client.wait_for_run(RunId::new(), options).await.expect_err("a server that never answers");

    assert!(matches!(error, rusty_acp::AcpError::Http { status: 502, .. }), "{error}");
}

/// With retrying off, a wait propagates the first failure — so
/// `RetryPolicy::disabled` means one thing everywhere rather than leaving the
/// polling helpers quietly forgiving.
#[tokio::test]
async fn a_wait_does_not_tolerate_blips_when_retrying_is_off() {
    let stub = Arc::new(Stub::new(1, StatusCode::SERVICE_UNAVAILABLE));
    let client = serve(stub.clone(), RetryPolicy::disabled()).await;

    let run_id = RunId::new();
    client
        .wait_for_run(run_id, WaitOptions::default().poll_every(Duration::from_millis(10)))
        .await
        .expect_err("the first failure reaches the caller");

    assert_eq!(stub.calls_to(&format!("/runs/{run_id}")), 1);
}

/// A connection that dies without a response — the case a status-based test
/// cannot reach, and the one a load balancer recycling connections produces.
///
/// Served by hand rather than by axum: the point is to accept the connection
/// and then drop it mid-request, which a working HTTP server will not do.
#[tokio::test]
async fn a_dropped_connection_is_retried() {
    const DROP_FIRST: usize = 2;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));

    let seen = accepted.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            if seen.fetch_add(1, Ordering::SeqCst) < DROP_FIRST {
                // Hang up without answering, which is what a replica going
                // away in the middle of a request looks like from here.
                drop(socket);
                continue;
            }
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let mut run = Run::new(AgentName::new("stub").unwrap(), None);
                run.status = RunStatus::Completed;
                let body = serde_json::to_string(&run).unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    let client = AcpClient::builder(format!("http://{addr}")).retry(prompt(3)).build().unwrap();
    let run = client.get_run(RunId::new()).await.expect("a later attempt connects");

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(accepted.load(Ordering::SeqCst), DROP_FIRST + 1);
}

/// Submitting a run over a dead connection is *not* retried, for the same
/// reason a 503 on submission is not: the server may have started it.
#[tokio::test]
async fn a_dropped_connection_does_not_retry_a_submission() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));

    let seen = accepted.clone();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            seen.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });

    let client = AcpClient::builder(format!("http://{addr}")).retry(prompt(3)).build().unwrap();
    let request = RunCreateRequest::new(AgentName::new("stub").unwrap(), [Message::user("go")]);
    client.create_run(request).await.expect_err("nothing is listening properly");

    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

/// One SSE frame: the `id` the server tags for resumption, the event name, and
/// the JSON body.
fn frame(id: u64, name: &str, event: &Event) -> String {
    format!("id: {id}\nevent: {name}\ndata: {}\n\n", serde_json::to_string(event).unwrap())
}

/// An SSE response, written by hand so a test can control exactly where the
/// body ends — which is the whole subject here.
fn sse(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

/// How a real stream opens, and the reason it opens that way: the client learns
/// which run it is watching from the first `run.*` event, and cannot resume
/// before it knows.
fn opening_frames(run: &Run) -> String {
    frame(0, "run.created", &Event::RunCreated { run: Box::new(run.clone()) })
        + &frame(1, "message.part", &Event::MessagePart { part: MessagePart::text("first") })
}

fn completed(run: &Run) -> Run {
    Run { status: RunStatus::Completed, ..run.clone() }
}

/// Serve `router` on an ephemeral port and point a client at it, with retrying
/// off so failures reach the resumption logic rather than being absorbed a
/// layer below it.
async fn serve_streaming(router: axum::Router) -> AcpClient {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    AcpClient::builder(format!("http://{addr}"))
        .retry(RetryPolicy::disabled())
        .reconnect(ReconnectPolicy {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(10),
        })
        .build()
        .unwrap()
}

/// A stream whose *reconnection* meets a blip.
///
/// This is a different layer from the retry loop above. The initial response
/// ends without a terminal event, so the client goes to resume — and meets a
/// 503 on the way back in. That should cost one of the stream's attempts rather
/// than the stream itself: a replica going away is precisely what resumption
/// exists for, so a balancer's 503 while it happens is not a reason to give up.
#[tokio::test]
async fn a_reconnection_that_fails_transiently_costs_an_attempt_not_the_stream() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let run = Run::new(AgentName::new("stub").unwrap(), None);
    let (started, resumed) = (run.clone(), run);

    let router = axum::Router::new()
        .route(
            "/runs",
            axum::routing::post(move || async move {
                // Then the body simply ends: no terminal event, so the client
                // treats it as a drop rather than an ending.
                sse(opening_frames(&started))
            }),
        )
        .route(
            "/runs/{run_id}/events",
            axum::routing::get(move || {
                let attempt = seen.fetch_add(1, Ordering::SeqCst);
                let run = resumed.clone();
                async move {
                    if attempt == 0 {
                        return (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response();
                    }
                    sse(frame(
                        2,
                        "run.completed",
                        &Event::RunCompleted { run: Box::new(completed(&run)) },
                    ))
                }
            }),
        );

    let stream = serve_streaming(router).await.stream("stub", [Message::user("go")]).await.unwrap();
    let events: Vec<Event> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .expect("no error reaches the caller");

    assert!(
        events.last().is_some_and(Event::is_terminal),
        "the stream stopped at the blip: {events:?}"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2, "one refused attempt, then one that worked");
}

/// A reconnection refused for a reason another attempt cannot fix — the run has
/// been swept — still reaches the caller rather than being quietly retried.
#[tokio::test]
async fn a_reconnection_refused_outright_reaches_the_caller() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let run = Run::new(AgentName::new("stub").unwrap(), None);

    let router = axum::Router::new()
        .route("/runs", axum::routing::post(move || async move { sse(opening_frames(&run)) }))
        .route(
            "/runs/{run_id}/events",
            axum::routing::get(move || {
                seen.fetch_add(1, Ordering::SeqCst);
                async { (StatusCode::NOT_FOUND, "gone").into_response() }
            }),
        );

    let stream = serve_streaming(router).await.stream("stub", [Message::user("go")]).await.unwrap();
    let collected: Vec<_> = stream.collect::<Vec<_>>().await;

    assert!(collected.last().is_some_and(Result::is_err), "the 404 was swallowed: {collected:?}");
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "a 404 is not worth asking again");
}
