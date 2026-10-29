//! `TickStore` against a real rusty_tick. Ignored by default; point it at a
//! scratch rusty_tick, since it creates tasks there:
//!
//! ```text
//! RUSTY_TICK_URL=http://127.0.0.1:8788 RUSTY_TICK_TOKEN=... \
//!     cargo test -p rusty_mill_board --test tick_live -- --ignored --nocapture
//! ```

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Column, ExecutionLink};
use rusty_mill_board::tick::{TickHttp, TickStore};

#[test]
#[ignore = "writes to a real rusty_tick"]
fn a_card_round_trips_through_a_real_rusty_tick() {
    let url = std::env::var("RUSTY_TICK_URL").expect("RUSTY_TICK_URL");
    let token = std::env::var("RUSTY_TICK_TOKEN").expect("RUSTY_TICK_TOKEN");
    let board = Board::new(
        TickStore::new(TickHttp::new(&url, &token).expect("client")),
        Roles::default(),
    );
    board.health().expect("health");
    let human = board.roles().actor("nano", "human").expect("role");
    let dev = board.roles().actor("dev", "builder").expect("role");

    let dep = board.add(&human, "live dep", "", vec![]).expect("add");
    let card = board
        .add(&human, "live card", "body", vec!["Live".into()])
        .expect("add");
    board
        .plan(card.id, &human, Some(3), Some(vec![dep.id]))
        .expect("plan");
    board
        .move_card(card.id, Column::Ready, &human)
        .expect("ready");
    board
        .move_card(dep.id, Column::Ready, &human)
        .expect("ready");
    // The card waits on its dependency, so the dependency is taken first.
    let taken = board.take(&dev, None).expect("take").expect("a card");
    assert_eq!(taken.id, dep.id);
    let link = ExecutionLink {
        execution: "e1".into(),
        goal: "g".into(),
        bbp_task: "b".into(),
        attempt: "a".into(),
        assignment: "s".into(),
        generation: 1,
    };
    board
        .bind_execution(dep.id, link.clone(), &dev)
        .expect("bind");
    let mut other = link;
    other.execution = "e2".into();
    assert!(matches!(
        board.bind_execution(dep.id, other, &dev),
        Err(BoardError::Refused(_))
    ));
    board
        .comment(dep.id, &dev, "live comment")
        .expect("comment");

    let read = board.get(dep.id).expect("get");
    assert_eq!(
        (read.column, read.claim.as_deref(), read.comments.len()),
        (Column::Building, Some("dev"), 1)
    );
    assert_eq!(
        read.execution.as_ref().map(|e| e.execution.as_str()),
        Some("e1")
    );
    let planned = board.get(card.id).expect("get");
    assert_eq!(
        (planned.priority, planned.deps.clone(), planned.tags.clone()),
        (3, vec![dep.id], vec!["live".to_owned()])
    );
    let (d, c) = (
        dep.number.expect("numbered"),
        card.number.expect("numbered"),
    );
    assert!(c > d, "numbers are reserved in creation order");
    assert_eq!(board.get_number(c).expect("by number").id, card.id);
    println!("cards {} and {} round-tripped", card.id, dep.id);
}
