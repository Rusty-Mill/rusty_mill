//! Dereferencing a session's history without paying for every turn in series.
//!
//! ACP models history as a list of *dereferenceable URLs* so a session can span
//! servers. `fetch_session_history` followed them one at a time, so a
//! two-hundred-turn conversation was two hundred sequential round trips, each
//! potentially to a different host — the total was the sum of every latency
//! rather than the largest of them. Retries compounded it: one slow server's
//! backoff stalled every turn queued behind it.
//!
//! # Why these tests do not race
//!
//! The gap between serial and concurrent is made **wide and fixed** rather than
//! measured at whatever speed the runner happens to manage. The stub history
//! server sleeps a fixed [`DELAY`] per request, so with [`TURNS`] URLs the two
//! shapes are seconds apart and the assertion sits in the middle of that gulf.
//! A test that merely observed "concurrent is faster" would pass by luck on a
//! quiet machine and fail on a loaded one.
//!
//! The ordering test has no timing in its assertion at all: the stub delays
//! each turn by *decreasing* amounts, so completion order is the exact reverse
//! of session order. It is a guard rather than a discriminator — the serial
//! loop was ordered too — but built so that a switch to `buffer_unordered`
//! could never pass it.

#![cfg(feature = "client")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::Path;
use axum::routing::get;
use axum::Json;
use rusty_acp::client::{AcpClient, HISTORY_CONCURRENCY};
use rusty_acp::types::{Message, Session, SessionId};

/// Long enough that the serial and concurrent shapes are seconds apart.
const DELAY: Duration = Duration::from_millis(200);

/// More turns than the concurrency limit, so the limit is actually exercised
/// rather than every fetch simply starting at once.
const TURNS: usize = 24;

/// What the stub server observed while serving a history.
#[derive(Debug, Default)]
struct Observed {
    /// Requests in flight right now.
    in_flight: AtomicUsize,
    /// The most that were ever in flight at once.
    peak: AtomicUsize,
}

/// A server that serves `TURNS` numbered messages, sleeping `delay(index)`
/// before each, and records how many requests overlapped.
async fn history_server(delay: fn(usize) -> Duration) -> (String, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let seen = Arc::clone(&observed);

    let app = axum::Router::new().route(
        "/turn/{index}",
        get(move |Path(index): Path<usize>| {
            let seen = Arc::clone(&seen);
            async move {
                let now = seen.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                seen.peak.fetch_max(now, Ordering::SeqCst);

                tokio::time::sleep(delay(index)).await;

                seen.in_flight.fetch_sub(1, Ordering::SeqCst);
                // The index travels in the body, so a misordered result is
                // visible as content rather than merely as a different length.
                Json(Message::user(index.to_string()))
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    (format!("http://{addr}"), observed)
}

fn session_over(base_url: &str) -> Session {
    let mut session = Session::with_id(SessionId::new());
    session.history = (0..TURNS).map(|index| format!("{base_url}/turn/{index}")).collect();
    session
}

/// The claim: the fetches overlap, so the wall clock is nothing like the sum.
///
/// Serial would be `TURNS * DELAY` — 4.8s. Concurrent at the limit is
/// `ceil(TURNS / HISTORY_CONCURRENCY) * DELAY` — 600ms. The bar is set at a
/// third of the serial figure, which is more than double the concurrent one:
/// wide enough that neither a slow runner nor a fast one decides the outcome.
#[tokio::test]
async fn the_turns_are_fetched_concurrently() {
    let (base_url, _observed) = history_server(|_| DELAY).await;
    let client = AcpClient::new(base_url.clone()).unwrap();
    let session = session_over(&base_url);

    let started = Instant::now();
    let messages = client.fetch_session_history(&session).await.unwrap();
    let elapsed = started.elapsed();

    assert_eq!(messages.len(), TURNS);
    let serial = DELAY * TURNS as u32;
    assert!(
        elapsed < serial / 3,
        "fetched in {elapsed:?}, which is not meaningfully better than the {serial:?} \
         a serial fetch would take"
    );
}

/// The answer is in session order even though the fetches finish backwards.
///
/// A **guard**, not a discriminator: the serial loop this replaced was ordered
/// too, so it passes either way. It exists because ordering stopped being free
/// the moment the fetches overlapped — `buffered` preserves it and
/// `buffer_unordered`, one word away and the more commonly reached for, does
/// not.
///
/// It is built so that swap could never pass. The stub delays turn `i` by
/// `(TURNS - i) * 20ms`, so the *last* URL completes first and the first
/// completes last: unordered collection returns the whole history reversed,
/// every time, on any machine. No timing appears in the assertion.
#[tokio::test]
async fn the_result_is_in_session_order_not_completion_order() {
    let (base_url, _observed) =
        history_server(|index| Duration::from_millis(20 * (TURNS - index) as u64)).await;
    let client = AcpClient::new(base_url.clone()).unwrap();
    let session = session_over(&base_url);

    let messages = client.fetch_session_history(&session).await.unwrap();

    let order: Vec<usize> = messages.iter().map(|m| m.text().parse().unwrap()).collect();
    assert_eq!(order, (0..TURNS).collect::<Vec<usize>>(), "history came back out of order");
}

/// The concurrency is bounded, and it is real.
///
/// Both halves matter. Unbounded would open a connection per turn and multiply
/// across sessions — the reason this is a `buffered` rather than a `join_all`.
/// And a peak of one would mean the limit had collapsed back to a serial loop
/// while every other assertion here still passed.
#[tokio::test]
async fn no_more_than_the_limit_are_in_flight() {
    let (base_url, observed) = history_server(|_| DELAY).await;
    let client = AcpClient::new(base_url.clone()).unwrap();
    let session = session_over(&base_url);

    client.fetch_session_history(&session).await.unwrap();

    let peak = observed.peak.load(Ordering::SeqCst);
    assert!(peak > 1, "the fetches never overlapped; peak in flight was {peak}");
    assert!(
        peak <= HISTORY_CONCURRENCY,
        "{peak} requests were in flight against a limit of {HISTORY_CONCURRENCY}"
    );
}

/// An empty history is not a special case.
#[tokio::test]
async fn an_empty_history_fetches_nothing() {
    let (base_url, observed) = history_server(|_| DELAY).await;
    let client = AcpClient::new(base_url).unwrap();
    let session = Session::with_id(SessionId::new());

    let messages = client.fetch_session_history(&session).await.unwrap();

    assert!(messages.is_empty());
    assert_eq!(observed.peak.load(Ordering::SeqCst), 0, "an empty history still made requests");
}

/// A failing turn abandons the fetch rather than returning a partial history.
///
/// Deliberate, and the same reasoning as the `410` on a trimmed event log: a
/// history with a hole in it that reads as complete is worse than no history,
/// because a caller cannot tell it from a short conversation — and here that
/// difference is what gets fed to an agent.
#[tokio::test]
async fn a_failing_turn_fails_the_whole_fetch() {
    let (base_url, _observed) = history_server(|_| Duration::from_millis(10)).await;
    let client = AcpClient::new(base_url.clone()).unwrap();

    let mut session = session_over(&base_url);
    // A URL the stub does not serve, in the middle of the conversation.
    session.history[TURNS / 2] = format!("{base_url}/missing");

    let result = client.fetch_session_history(&session).await;

    assert!(result.is_err(), "a partial history was returned as if it were whole");
}
