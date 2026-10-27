//! Execution links outlive the process: binding and recovery are idempotent and fenced
//! across a restart, which reopens the board over the same rusty_tick.

use std::sync::Arc;

use rusty_mill_board::api::Shared;
use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::ExecutionLink;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::TickStore;

mod fake_tick;
use fake_tick::FakeTick;

/// A board process over `tick`; a second call is the same board after a restart.
fn board_on(tick: &FakeTick) -> Arc<Shared> {
    Arc::new(Board::new(
        Box::new(TickStore::new(tick.clone())) as Box<dyn Store>,
        Roles::default(),
    ))
}

#[test]
fn identical_execution_binding_is_concurrent_and_restart_idempotent() {
    let tick = FakeTick::default();
    let board = board_on(&tick);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "execute once", "", vec![]).expect("add");
    let link = ExecutionLink {
        execution: "execution-1".into(),
        goal: "goal-1".into(),
        bbp_task: "task-1".into(),
        attempt: "attempt-1".into(),
        assignment: "project-orch".into(),
        generation: 1,
    };
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let (board, barrier, link) = (Arc::clone(&board), Arc::clone(&barrier), link.clone());
        threads.push(std::thread::spawn(move || {
            let builder = board.roles().actor("nano", "human").expect("role");
            barrier.wait();
            board.bind_execution(card.id, link, &builder).expect("bind")
        }));
    }
    for thread in threads {
        let adopted = thread.join().expect("thread");
        assert_eq!(adopted.execution.as_ref(), Some(&link));
        assert_eq!(adopted.rev, 1);
    }
    assert_eq!(board.events(None, 100).expect("events").len(), 2);
    drop(board);

    let reopened = board_on(&tick);
    let builder = reopened.roles().actor("nano", "human").expect("role");
    let events = reopened.events(None, 100).expect("events").len();
    let adopted = reopened
        .bind_execution(card.id, link, &builder)
        .expect("adopt");
    assert_eq!(adopted.rev, 1);
    // Events live in the process; adopting the same link adds none.
    assert_eq!(reopened.events(None, 100).expect("events").len(), events);
    drop(reopened);
}

#[test]
fn execution_recovery_survives_reopen_and_fences_stale_actors() {
    let tick = FakeTick::default();
    let board = board_on(&tick);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "recover once", "", vec![]).expect("add");
    let link = |generation| ExecutionLink {
        execution: "execution-1".into(),
        goal: "goal-1".into(),
        bbp_task: "task-1".into(),
        attempt: "attempt-1".into(),
        assignment: "project-orch".into(),
        generation,
    };
    board
        .bind_execution(card.id, link(1), &human)
        .expect("bind");
    drop(board);

    // Two replacements race from the same expected generation: one wins.
    let reopened = board_on(&tick);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let racers: Vec<_> = [2_u64, 3]
        .into_iter()
        .map(|generation| {
            let (board, barrier) = (Arc::clone(&reopened), Arc::clone(&barrier));
            std::thread::spawn(move || {
                let builder = board.roles().actor("nano", "human").expect("role");
                barrier.wait();
                board.recover_execution(card.id, 1, link(generation), &builder)
            })
        })
        .collect();
    let won: Vec<_> = racers
        .into_iter()
        .filter_map(|racer| racer.join().expect("thread").ok())
        .collect();
    assert_eq!(won.len(), 1, "exactly one recovery wins the CAS");
    let winner = won[0].execution.clone().expect("link");
    drop(reopened);

    let reopened = board_on(&tick);
    let builder = reopened.roles().actor("nano", "human").expect("role");
    let stored = reopened.get(card.id).expect("card");
    assert_eq!(stored.execution.as_ref(), Some(&winner));
    // Replaying the winning recovery is a no-op, with no new event.
    let events = reopened.events(None, 100).expect("events").len();
    assert_eq!(
        reopened
            .recover_execution(card.id, 1, winner.clone(), &builder)
            .expect("replay"),
        stored
    );
    assert_eq!(reopened.events(None, 100).expect("events").len(), events);
    // The stale generation can neither recover again nor rebind.
    assert!(
        reopened
            .recover_execution(card.id, 1, link(9), &builder)
            .is_err()
    );
    assert!(reopened.bind_execution(card.id, link(1), &builder).is_err());
    assert_eq!(reopened.get(card.id).expect("card").execution, Some(winner));
    drop(reopened);
}
