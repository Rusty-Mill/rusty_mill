//! Bounding the default store's sessions.
//!
//! Runs have always been bounded and sessions were not, which made "this
//! store's memory is bounded" true of one half only. The asymmetry hides in
//! exactly the deployment where it bites: a load test that reuses one session
//! shows stable memory, and a long-lived server with many short conversations —
//! the shape a hosted agent actually has — accumulates a record, a history and
//! a state document per conversation, forever.
//!
//! What matters as much as the bound is *which* session goes. Evicting by age
//! would drop the long conversation still in progress in favour of the one
//! nobody has opened since, so eviction is least-recently-used, and a read
//! counts as use.

#![cfg(feature = "server")]

use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::types::{Message, Session, SessionId};

/// A store bounded at `max_sessions`, with runs left unbounded so nothing
/// crosses over.
fn store(max_sessions: usize) -> InMemoryStore {
    InMemoryStore::with_limits(1024, max_sessions)
}

async fn converse(store: &InMemoryStore, session_id: SessionId, text: &str) {
    store
        .append_session_messages(session_id, "http://acp.example", vec![Message::user(text)])
        .await
        .unwrap();
}

#[tokio::test]
async fn sessions_are_bounded() {
    let store = store(3);

    for index in 0..10 {
        converse(&store, SessionId::new(), &format!("conversation {index}")).await;
    }

    assert_eq!(store.session_count(), 3, "sessions grew without bound");
}

/// The bound has to take the state document with it, or it holds back the half
/// that is usually larger and reports itself bounded anyway.
#[tokio::test]
async fn eviction_takes_the_state_document_too() {
    let store = store(2);

    let evicted = SessionId::new();
    store
        .put_session_state(evicted, "http://acp.example", serde_json::json!({ "big": "payload" }))
        .await
        .unwrap();
    assert!(store.get_session_state(evicted).await.unwrap().is_some());

    // Two newer sessions push it out.
    for index in 0..2 {
        converse(&store, SessionId::new(), &format!("newer {index}")).await;
    }

    assert!(store.get_session(evicted).await.unwrap().is_none(), "the record survived");
    assert!(
        store.get_session_state(evicted).await.unwrap().is_none(),
        "the state document outlived the session it belonged to"
    );
}

/// Least *recently used*, not oldest. The conversation still being read from is
/// the one to keep.
#[tokio::test]
async fn reading_a_session_keeps_it() {
    let store = store(2);

    let old_but_active = SessionId::new();
    let idle = SessionId::new();
    converse(&store, old_but_active, "first").await;
    converse(&store, idle, "second").await;

    // The older session is read, which is use — an agent handed its history
    // every turn does exactly this.
    store.get_session(old_but_active).await.unwrap();

    // A third session forces one out. It must be the idle one.
    converse(&store, SessionId::new(), "third").await;

    assert!(
        store.get_session(old_but_active).await.unwrap().is_some(),
        "evicted the session that was still being read from"
    );
    assert!(store.get_session(idle).await.unwrap().is_none(), "kept the idle session instead");
}

/// Reading *state* is use as well — a stateful agent may touch its state
/// without ever reading its history.
#[tokio::test]
async fn reading_session_state_keeps_it() {
    let store = store(2);

    let stateful = SessionId::new();
    let idle = SessionId::new();
    store
        .put_session_state(stateful, "http://acp.example", serde_json::json!({ "n": 1 }))
        .await
        .unwrap();
    converse(&store, idle, "second").await;

    store.get_session_state(stateful).await.unwrap();
    converse(&store, SessionId::new(), "third").await;

    assert!(
        store.get_session_state(stateful).await.unwrap().is_some(),
        "evicted a session whose state was just read"
    );
    assert!(store.get_session(idle).await.unwrap().is_none());
}

/// Appending is use too, which is what protects a run's own session while it
/// runs.
#[tokio::test]
async fn appending_to_a_session_keeps_it() {
    let store = store(2);

    let busy = SessionId::new();
    let idle = SessionId::new();
    converse(&store, busy, "first").await;
    converse(&store, idle, "second").await;
    converse(&store, busy, "still going").await;

    converse(&store, SessionId::new(), "third").await;

    let survivor = store.get_session(busy).await.unwrap().expect("the busy session survives");
    assert_eq!(survivor.messages.len(), 2);
    assert!(store.get_session(idle).await.unwrap().is_none());
}

/// `ensure_session` is how a run adopts a session, so it counts as use for the
/// same reason appending does.
#[tokio::test]
async fn ensuring_a_session_keeps_it() {
    let store = store(2);

    let adopted = SessionId::new();
    let idle = SessionId::new();
    converse(&store, adopted, "first").await;
    converse(&store, idle, "second").await;

    store.ensure_session(Session::with_id(adopted)).await.unwrap();
    converse(&store, SessionId::new(), "third").await;

    assert!(store.get_session(adopted).await.unwrap().is_some());
    assert!(store.get_session(idle).await.unwrap().is_none());
}

/// An evicted session is indistinguishable from one that never existed, and a
/// later append silently starts a fresh conversation. Asserted rather than left
/// implicit: it is the cost of the decision, and a future change that made
/// eviction louder should have to come here and say so.
#[tokio::test]
async fn an_evicted_session_comes_back_empty() {
    let store = store(1);

    let forgotten = SessionId::new();
    converse(&store, forgotten, "something memorable").await;
    converse(&store, SessionId::new(), "pushes it out").await;

    assert!(store.get_session(forgotten).await.unwrap().is_none());

    converse(&store, forgotten, "starting over").await;
    let restarted = store.get_session(forgotten).await.unwrap().expect("recreated");
    assert_eq!(restarted.messages.len(), 1, "the history came back from somewhere");
}

/// The runs bound and the sessions bound are separate numbers, so a store full
/// of sessions does not evict runs and vice versa.
#[tokio::test]
async fn the_two_bounds_are_independent() {
    let store = InMemoryStore::with_limits(1024, 2);

    use rusty_acp::types::{AgentName, Run, RunStatus};
    for _ in 0..5 {
        let mut run = Run::new(AgentName::new("agent").unwrap(), None);
        run.status = RunStatus::Completed;
        store.put_run(&run).await.unwrap();
    }
    for index in 0..5 {
        converse(&store, SessionId::new(), &format!("session {index}")).await;
    }

    assert_eq!(store.run_count(), 5, "runs were evicted by the session bound");
    assert_eq!(store.session_count(), 2);
}

/// A limit of zero would evict everything the moment it was written, so it is
/// clamped the same way `max_runs` is.
#[tokio::test]
async fn a_zero_limit_is_clamped() {
    let store = InMemoryStore::with_limits(0, 0);

    let session_id = SessionId::new();
    converse(&store, session_id, "hello").await;

    assert_eq!(store.session_count(), 1, "a zero limit ate the session it was just given");
    assert!(store.get_session(session_id).await.unwrap().is_some());
}
