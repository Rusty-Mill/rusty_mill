//! A conformance suite any [`Store`] can be run against.
//!
//! The [module docs](super) make promises the trait signature cannot enforce —
//! dense unique indices under concurrent append, exactly one winner per lease
//! claim, fan-out to every live subscriber — and each of them is a requirement
//! that a plausible implementation gets wrong. The obvious way to hand out an
//! index is to read the length and add one, which is wrong only under
//! contention; the obvious way to claim a lease is to read then write, which
//! loses to itself. Both shipped backends needed a specific mechanism, and
//! neither is the first thing you would write.
//!
//! Documentation is not a test. This is:
//!
//! ```no_run
//! # #[cfg(feature = "store-testkit")]
//! # async fn demo() {
//! # use std::sync::Arc;
//! # use rusty_acp::server::store::{InMemoryStore, Store, testkit};
//! let report = testkit::verify(|| async { Arc::new(InMemoryStore::default()) as Arc<dyn Store> })
//!     .await;
//! assert!(report.is_ok(), "{report}");
//! # }
//! ```
//!
//! # What this does *not* cover
//!
//! The server's behaviour on top of a store. `tests/multi_replica.rs` drives two
//! whole [`AcpServer`](crate::server::AcpServer)s over HTTP against one backend
//! and is a different claim — that the *server* is replica-agnostic. Making a
//! backend author stand up a server to check their store would drag the `client`
//! and `server` layers into something that only wanted the trait.
//!
//! So the two are separate on purpose: this talks to the trait directly, and
//! needs neither an HTTP listener nor a client.
//!
//! # The factory
//!
//! `verify` takes a closure rather than a store because several checks need two
//! independent handles to the same backend, and because a fresh namespace per
//! check keeps one failure from cascading. What "fresh" means is the caller's
//! decision — a new key prefix, a new table prefix, a new instance.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;

use crate::server::store::{Notification, RecoveryRecord, Store};
use crate::types::{AgentName, Event, Message, MessagePart, Run, Session, SessionId};

/// What [`verify`] found.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Checks the backend satisfied, in the order they ran.
    pub passed: Vec<&'static str>,
    /// Checks it did not, with what went wrong.
    pub failed: Vec<(&'static str, String)>,
}

impl Report {
    /// Whether every check passed.
    pub fn is_ok(&self) -> bool {
        self.failed.is_empty()
    }

    /// Turn the report into a `Result`, for a caller that wants `?`.
    pub fn into_result(self) -> Result<(), Report> {
        if self.is_ok() {
            Ok(())
        } else {
            Err(self)
        }
    }

    fn record(&mut self, name: &'static str, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.passed.push(name),
            Err(why) => self.failed.push((name, why)),
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} passed, {} failed", self.passed.len(), self.failed.len())?;
        for (name, why) in &self.failed {
            writeln!(f, "  FAILED {name}: {why}")?;
        }
        Ok(())
    }
}

/// Assert inside a check, yielding `Err(String)` rather than panicking.
///
/// A panic would take the whole suite down on the first failure, and a backend
/// author wants every answer at once — the failures usually cluster.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err(format!($($arg)+));
        }
    };
}

/// Run every check against stores produced by `new_store`.
///
/// Never panics and never short-circuits: each check gets a fresh store and its
/// own entry in the [`Report`], so one broken invariant does not hide the rest.
pub async fn verify<F, Fut>(new_store: F) -> Report
where
    F: Fn() -> Fut,
    Fut: Future<Output = Arc<dyn Store>>,
{
    let mut report = Report::default();

    report.record("a run round-trips", a_run_round_trips(new_store().await).await);
    report.record(
        "an unknown run reads as absent",
        an_unknown_run_is_absent(new_store().await).await,
    );
    report.record(
        "concurrent appends get dense unique indices",
        concurrent_appends_are_dense(new_store().await).await,
    );
    report.record("events_from seeks", events_from_seeks(new_store().await).await);
    report.record(
        "an offset past the end yields nothing",
        reading_past_the_end_is_empty(new_store().await).await,
    );
    report.record("exactly one lease claim wins", one_claim_wins(new_store().await).await);
    report.record("a live lease is not claimable", a_live_lease_holds(new_store().await).await);
    report.record("an unrenewed lease lapses", an_unrenewed_lease_lapses(new_store().await).await);
    report.record("renewal extends a lease", renewal_extends(new_store().await).await);
    report.record("a released lease is claimable", release_frees_a_lease(new_store().await).await);
    report.record("every subscriber sees an event", publish_fans_out(new_store().await).await);
    report.record(
        "publishing to nobody is not an error",
        publish_without_subscribers(new_store().await).await,
    );
    report.record(
        "concurrent session appends do not interleave",
        session_appends_are_dense(new_store().await).await,
    );
    report.record(
        "ensure_session is idempotent",
        ensure_session_is_idempotent(new_store().await).await,
    );
    report.record("session state round-trips", session_state_round_trips(new_store().await).await);
    report.record(
        "a recovery record round-trips and clears",
        recovery_records_round_trip(new_store().await).await,
    );

    report
}

type Check = Result<(), String>;

fn probe_run() -> Run {
    Run::new(AgentName::new("testkit").expect("a valid agent name"), None)
}

fn sized_event(bytes: usize) -> Event {
    Event::MessagePart { part: MessagePart::text("x".repeat(bytes)) }
}

async fn seeded(store: &Arc<dyn Store>) -> Result<Run, String> {
    let run = probe_run();
    store.put_run(&run).await.map_err(|error| format!("put_run failed: {error}"))?;
    Ok(run)
}

async fn a_run_round_trips(store: Arc<dyn Store>) -> Check {
    let mut run = seeded(&store).await?;
    let read = store.get_run(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(read.is_some(), "a run written was not readable");

    // An overwrite, which is what `put_run` is: the sole-writer invariant means
    // a backend never has to merge.
    run.status = crate::types::RunStatus::Completed;
    store.put_run(&run).await.map_err(|e| e.to_string())?;
    let read = store.get_run(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(
        read.map(|run| run.status) == Some(crate::types::RunStatus::Completed),
        "an overwrite did not replace the run"
    );
    Ok(())
}

async fn an_unknown_run_is_absent(store: Arc<dyn Store>) -> Check {
    let read = store.get_run(probe_run().run_id).await.map_err(|e| e.to_string())?;
    ensure!(read.is_none(), "a run that was never written read as present");
    Ok(())
}

/// The one a read-then-write implementation fails, and only under contention.
async fn concurrent_appends_are_dense(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;

    let appends = (0..16).map(|_| {
        let store = Arc::clone(&store);
        let run_id = run.run_id;
        async move { store.append_event(run_id, &sized_event(64)).await }
    });
    let mut indices: Vec<u64> = futures_util::future::join_all(appends)
        .await
        .into_iter()
        .collect::<Result<_, _>>()
        .map_err(|e| format!("append_event failed: {e}"))?;
    indices.sort_unstable();

    ensure!(
        indices == (0..16).collect::<Vec<u64>>(),
        "indices were not dense and unique: {indices:?}"
    );
    Ok(())
}

async fn events_from_seeks(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    for index in 0..8u64 {
        let part = MessagePart::text(format!("{index}"));
        store
            .append_event(run.run_id, &Event::MessagePart { part })
            .await
            .map_err(|e| e.to_string())?;
    }

    // Relative to what the backend still holds: a backend that bounds a log
    // reports a non-zero start, and reading from it must land on that event.
    let earliest = store.earliest_event(run.run_id).await.map_err(|e| e.to_string())?;
    let tail = store.events_from(run.run_id, earliest + 2).await.map_err(|e| e.to_string())?;
    ensure!(!tail.is_empty(), "events_from returned nothing for a retained index");

    let Some(Event::MessagePart { part }) = tail.first() else {
        return Err("events_from returned an unexpected event shape".into());
    };
    ensure!(
        part.content.as_deref() == Some((earliest + 2).to_string().as_str()),
        "events_from landed on the wrong event: {:?}",
        part.content
    );
    Ok(())
}

async fn reading_past_the_end_is_empty(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    store.append_event(run.run_id, &sized_event(16)).await.map_err(|e| e.to_string())?;

    let beyond = store.events_from(run.run_id, 500).await.map_err(|e| e.to_string())?;
    ensure!(beyond.is_empty(), "an offset past the end yielded {} events", beyond.len());
    Ok(())
}

/// What stops two replicas recovering the same run.
async fn one_claim_wins(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;

    let claims = (0..8).map(|index| {
        let store = Arc::clone(&store);
        let run_id = run.run_id;
        let owner = format!("replica-{index}");
        async move { store.try_claim_lease(run_id, &owner, Duration::from_secs(30)).await }
    });
    let won = futures_util::future::join_all(claims)
        .await
        .into_iter()
        .collect::<Result<Vec<bool>, _>>()
        .map_err(|e| format!("try_claim_lease failed: {e}"))?
        .into_iter()
        .filter(|won| *won)
        .count();

    ensure!(won == 1, "{won} claimants won a single lease; exactly one must");
    Ok(())
}

async fn a_live_lease_holds(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    let taken = store
        .try_claim_lease(run.run_id, "replica-a", Duration::from_secs(30))
        .await
        .map_err(|e| e.to_string())?;
    ensure!(taken, "an unheld lease could not be claimed");

    let stolen = store
        .try_claim_lease(run.run_id, "replica-b", Duration::from_secs(30))
        .await
        .map_err(|e| e.to_string())?;
    ensure!(!stolen, "a live lease was claimed out from under its holder");

    let owner = store.lease_owner(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(owner.as_deref() == Some("replica-a"), "lease_owner reported {owner:?}");
    Ok(())
}

/// The lease the timing checks use.
///
/// One second, and every wait around it is a second clear of a boundary. Two
/// reasons, and the second is the one that set the number.
///
/// A tighter lease would make the suite quicker and would be a race: a check
/// that expects a lease to still be held 100ms before it lapses fails on a
/// loaded runner for no reason the backend author can act on. Nothing here is
/// measuring latency, so there is no reason to be near an edge.
///
/// And a sub-second lease is not a thing every backend can express. Redis
/// expiry was second-granular until `PX`, and a store built on a KV service
/// with a seconds-only TTL is a reasonable backend that owes nobody
/// millisecond resolution. The trait asks that a lease expire, not that it
/// expire precisely.
const LEASE: Duration = Duration::from_secs(1);

/// What makes an abandoned run recognisable. A backend without native expiry
/// has to enforce this on read.
async fn an_unrenewed_lease_lapses(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    store.renew_lease(run.run_id, "replica-a", LEASE).await.map_err(|e| e.to_string())?;

    tokio::time::sleep(2 * LEASE).await;
    let owner = store.lease_owner(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(owner.is_none(), "a lease nobody renewed was still held by {owner:?}");
    Ok(())
}

async fn renewal_extends(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    store.renew_lease(run.run_id, "replica-a", 2 * LEASE).await.map_err(|e| e.to_string())?;
    tokio::time::sleep(LEASE).await;
    store.renew_lease(run.run_id, "replica-a", 2 * LEASE).await.map_err(|e| e.to_string())?;

    // Past the first expiry, and still a whole lease short of the second.
    tokio::time::sleep(3 * LEASE / 2).await;
    let owner = store.lease_owner(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(owner.is_some(), "renewal did not push the expiry out");
    Ok(())
}

async fn release_frees_a_lease(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    store
        .try_claim_lease(run.run_id, "replica-a", Duration::from_secs(30))
        .await
        .map_err(|e| e.to_string())?;
    store.release_lease(run.run_id).await.map_err(|e| e.to_string())?;

    let taken = store
        .try_claim_lease(run.run_id, "replica-b", Duration::from_secs(30))
        .await
        .map_err(|e| e.to_string())?;
    ensure!(taken, "a released lease could not be claimed");
    Ok(())
}

/// Fan-out, not hand-off: a streaming client on every replica has to see this.
///
/// The event is appended before it is published, which is the order
/// [`Store::publish`] requires and the order the server uses. Publishing an
/// index that is not in the log yet is allowed to deliver nothing —
/// [`PostgresStore`](crate::server::store::PostgresStore) sends only the index
/// and has each subscriber read the row, so an unwritten event resolves to
/// nothing at all. This check tests fan-out; it must not accidentally test that
/// precondition.
async fn publish_fans_out(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    let event = sized_event(16);
    let index = store.append_event(run.run_id, &event).await.map_err(|e| format!("append: {e}"))?;

    let mut first = store.subscribe(run.run_id).await.map_err(|e| e.to_string())?;
    let mut second = store.subscribe(run.run_id).await.map_err(|e| e.to_string())?;

    store
        .publish(run.run_id, Notification::event_at(index, event))
        .await
        .map_err(|e| e.to_string())?;

    for (which, subscriber) in [("first", &mut first), ("second", &mut second)] {
        let received = tokio::time::timeout(Duration::from_secs(5), subscriber.next()).await;
        match received {
            Ok(Some(notification)) => {
                ensure!(
                    notification.event().is_some(),
                    "the {which} subscriber received {notification:?}, not an event"
                );
            }
            Ok(None) => return Err(format!("the {which} subscriber's stream ended")),
            Err(_) => return Err(format!("the {which} subscriber received nothing in 5s")),
        }
    }
    Ok(())
}

async fn publish_without_subscribers(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    let event = sized_event(16);
    let index = store.append_event(run.run_id, &event).await.map_err(|e| format!("append: {e}"))?;
    store
        .publish(run.run_id, Notification::event_at(index, event))
        .await
        .map_err(|error| format!("publishing to nobody failed: {error}"))?;
    Ok(())
}

/// Two replicas appending to one session must not interleave or overwrite.
async fn session_appends_are_dense(store: Arc<dyn Store>) -> Check {
    let session_id = SessionId::new();
    let appends = (0..8).map(|index| {
        let store = Arc::clone(&store);
        async move {
            store
                .append_session_messages(
                    session_id,
                    "http://acp.example",
                    vec![Message::user(format!("message {index}"))],
                )
                .await
        }
    });
    futures_util::future::join_all(appends)
        .await
        .into_iter()
        .collect::<Result<Vec<()>, _>>()
        .map_err(|e| format!("append_session_messages failed: {e}"))?;

    let record = store
        .get_session(session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the session vanished after being appended to")?;
    ensure!(
        record.messages.len() == 8,
        "8 concurrent appends left {} messages",
        record.messages.len()
    );
    Ok(())
}

async fn ensure_session_is_idempotent(store: Arc<dyn Store>) -> Check {
    let session_id = SessionId::new();
    store
        .append_session_messages(session_id, "http://acp.example", vec![Message::user("hi")])
        .await
        .map_err(|e| e.to_string())?;

    // Whichever replica gets there first seeds it; the rest must read what is
    // stored rather than reset it.
    store.ensure_session(Session::with_id(session_id)).await.map_err(|e| e.to_string())?;

    let record = store
        .get_session(session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the session vanished after ensure_session")?;
    ensure!(
        record.messages.len() == 1,
        "ensure_session discarded history: {} messages left",
        record.messages.len()
    );
    Ok(())
}

async fn session_state_round_trips(store: Arc<dyn Store>) -> Check {
    let session_id = SessionId::new();
    let absent = store.get_session_state(session_id).await.map_err(|e| e.to_string())?;
    ensure!(absent.is_none(), "unwritten session state read as present");

    store
        .put_session_state(session_id, "http://acp.example", serde_json::json!({ "turns": 3 }))
        .await
        .map_err(|e| e.to_string())?;
    let read = store.get_session_state(session_id).await.map_err(|e| e.to_string())?;
    ensure!(
        read == Some(serde_json::json!({ "turns": 3 })),
        "session state did not round-trip: {read:?}"
    );
    Ok(())
}

async fn recovery_records_round_trip(store: Arc<dyn Store>) -> Check {
    let run = seeded(&store).await?;
    let absent = store.recovery_record(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(absent.is_none(), "an unwritten recovery record read as present");

    let record = RecoveryRecord { input: vec![Message::user("go")], attempt: 2, handed_off: true };
    store.put_recovery_record(run.run_id, Some(&record)).await.map_err(|e| e.to_string())?;
    let read = store.recovery_record(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(read.as_ref() == Some(&record), "a recovery record did not round-trip: {read:?}");

    // Its absence is what tells a reaper a run must not be replayed, so
    // clearing has to actually clear.
    store.put_recovery_record(run.run_id, None).await.map_err(|e| e.to_string())?;
    let cleared = store.recovery_record(run.run_id).await.map_err(|e| e.to_string())?;
    ensure!(cleared.is_none(), "clearing a recovery record left {cleared:?}");
    Ok(())
}
