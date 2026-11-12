//! External execution refs (ADR-0006): recorded through the board, stored with the card, and
//! never read to decide anything.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{
    Actor, BoardError, Capability, Card, Column, Event, EventKind, ExecutionLink,
};
use rusty_mill_board::external::{ExternalRef, Observation, Provider, RefState};
use rusty_mill_board::store::{MemStore, Store};

const NOW: i64 = 1_800_000_000_000;

fn board() -> Board<MemStore> {
    Board::with_clock(MemStore::default(), Roles::default(), || NOW)
}

fn who(b: &Board<MemStore>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

fn obs(reference: &str, state: RefState) -> Observation {
    Observation {
        provider: Provider::CodexCloud,
        reference: reference.into(),
        state,
        url: Some(format!("https://example.test/{reference}")),
        observed_at_ms: NOW - 1_000,
    }
}

#[test]
fn an_observation_is_recorded_with_the_actor_and_logged_once() {
    let b = board();
    let dev = who(&b, "codex", "builder");
    let card = b
        .add(&who(&b, "nano", "human"), "cloud work", "", vec![])
        .expect("add");
    let seen = b
        .observe_execution(card.id, obs("thread-1", RefState::Running), &dev)
        .expect("observe");
    assert_eq!(seen.external_refs[0].by, "codex");
    assert_eq!(b.get(card.id).expect("get"), seen);
    let observed = |b: &Board<MemStore>| {
        b.events(None, 100)
            .expect("events")
            .iter()
            .filter(|e| e.kind == EventKind::ExecutionObserved)
            .count()
    };
    assert_eq!(observed(&b), 1);
    // A replay changes nothing and logs nothing.
    let again = b
        .observe_execution(card.id, obs("thread-1", RefState::Running), &dev)
        .expect("replay");
    assert_eq!((again.rev, observed(&b)), (seen.rev, 1));
}

#[test]
fn observing_needs_build() {
    let b = board();
    let card = b
        .add(&who(&b, "nano", "human"), "t", "", vec![])
        .expect("add");
    let qa = who(&b, "gemini", "qa");
    assert_eq!(
        b.observe_execution(card.id, obs("t", RefState::Queued), &qa),
        Err(BoardError::Forbidden(Capability::Build))
    );
}

/// Three Ready cards, oldest first, with the same ids on every board that calls it.
fn cards(refs: &[ExternalRef]) -> Vec<Card> {
    (0..3)
        .map(|i| Card {
            id: uuid::Uuid::from_u128(i + 1),
            column: Column::READY,
            created_ms: NOW - 100 + i as i64,
            updated_ms: NOW - 100,
            external_refs: refs.to_vec(),
            ..Card::new(&format!("card {i}"), "", vec![], NOW - 100).expect("card")
        })
        .collect()
}

fn seeded(refs: &[ExternalRef]) -> Board<MemStore> {
    let store = MemStore::default();
    for c in cards(refs) {
        store
            .insert(&c, Event::of(&c, EventKind::Created, "nano"))
            .expect("insert");
    }
    Board::with_clock(store, Roles::default(), || NOW)
}

/// Every ref state, the live ones included.
fn every_state() -> Vec<ExternalRef> {
    use RefState::*;
    [Queued, Running, Succeeded, Failed, Cancelled, Unknown]
        .into_iter()
        .zip(0..)
        .map(|(state, i)| ExternalRef {
            provider: Provider::CodexCloud,
            reference: format!("thread-{i}"),
            url: None,
            state,
            observed_at_ms: NOW - 10,
            by: "codex".into(),
        })
        .collect()
}

fn link(generation: u64) -> ExecutionLink {
    ExecutionLink {
        execution: "x1".into(),
        goal: "g".into(),
        bbp_task: "T1".into(),
        attempt: "a".into(),
        assignment: "orch".into(),
        generation,
    }
}

/// What each operation returned, refs removed, and the events it logged, ids removed.
fn run(b: &Board<MemStore>) -> (Vec<Result<Option<Card>, BoardError>>, Vec<Event>) {
    let dev = who(b, "claude", "builder");
    let human = who(b, "nano", "human");
    let [first, second, third] = [1, 2, 3].map(uuid::Uuid::from_u128);
    let results = vec![
        b.take(&dev, None),
        b.move_card(first, &Column::QA, &dev).map(Some),
        b.claim(second, &human).map(Some),
        b.take(&dev, None),
        b.bind_execution(third, link(1), &dev).map(Some),
        b.recover_execution(third, 1, link(2), &dev).map(Some),
        b.take(&dev, None),
    ];
    let strip = |c: Card| Card {
        external_refs: Vec::new(),
        ..c
    };
    let results = results
        .into_iter()
        .map(|r| r.map(|c| c.map(strip)))
        .collect();
    let events = b
        .events(None, 100)
        .expect("events")
        .into_iter()
        .map(|e| Event {
            id: uuid::Uuid::nil(),
            ..e
        })
        .collect();
    (results, events)
}

#[test]
fn refs_in_any_state_change_no_take_move_claim_or_execution_outcome() {
    let plain = run(&seeded(&[]));
    let with_refs = run(&seeded(&every_state()));
    assert!(
        plain.0.iter().any(|r| matches!(r, Ok(Some(_)))),
        "{plain:?}"
    );
    assert_eq!(with_refs, plain);
}
