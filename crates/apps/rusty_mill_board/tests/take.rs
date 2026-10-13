//! `Board::take`: pulling the next card for a role.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Actor, BoardError, Column};
use rusty_mill_board::store::MemStore;

fn board() -> Board<MemStore> {
    Board::new(MemStore::default(), Roles::default())
}

fn who(b: &Board<MemStore>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

/// A card put in Ready by a person.
fn ready(b: &Board<MemStore>, title: &str) -> uuid::Uuid {
    let h = who(b, "nano", "human");
    let c = b.add(&h, title, "", vec![]).expect("add");
    b.move_card(c.id, Column::Ready, &h).expect("ready");
    c.id
}

#[test]
fn take_enters_a_ready_card_and_claims_it() {
    let b = board();
    let id = ready(&b, "a");
    let dev = who(&b, "claude", "builder");
    let card = b.take(&dev, None).expect("take").expect("a card");
    assert_eq!(
        (card.id, card.column, card.claim.as_deref()),
        (id, Column::Building, Some("claude"))
    );
    assert!(
        b.take(&dev, None)
            .expect("take")
            .is_some_and(|c| c.id == id),
        "own claim is resumed"
    );
}

#[test]
fn take_is_role_aware_and_none_when_nothing_waits() {
    let b = board();
    ready(&b, "a");
    let qa = who(&b, "gemini", "qa");
    assert!(
        b.take(&qa, None).expect("take").is_none(),
        "a qa role has no work in Ready"
    );
    let rev = who(&b, "codex", "reviewer");
    assert!(b.take(&rev, None).expect("take").is_none());
}

#[test]
fn take_prefers_work_nearest_done_then_oldest() {
    let b = board();
    let first = ready(&b, "first");
    let second = ready(&b, "second");
    let dev = who(&b, "claude", "builder");
    // Move `second` on to Qa by hand: the qa role must see it, the builder takes `first`.
    let t = b.take(&dev, Some(second)).expect("take").expect("card");
    b.move_card(t.id, Column::Qa, &dev).expect("to qa");
    let got = b.take(&dev, None).expect("take").expect("card");
    assert_eq!(got.id, first, "builder has nothing in Qa");
    let qa = who(&b, "gemini", "qa");
    assert_eq!(b.take(&qa, None).expect("take").expect("card").id, second);
}

#[test]
fn two_workers_never_take_the_same_card() {
    let b = board();
    ready(&b, "only");
    let (a, c) = (who(&b, "claude", "builder"), who(&b, "codex", "builder"));
    let got = b.take(&a, None).expect("take").expect("card");
    assert!(
        b.take(&c, None).expect("take").is_none(),
        "the claimed card is skipped: {got:?}"
    );
}

#[test]
fn take_by_id_reports_why_it_cannot() {
    let b = board();
    let id = ready(&b, "a");
    let (a, c) = (who(&b, "claude", "builder"), who(&b, "codex", "builder"));
    b.take(&a, Some(id)).expect("take").expect("card");
    assert_eq!(
        b.take(&c, Some(id)),
        Err(BoardError::Claimed("claude".into()))
    );
    assert_eq!(
        b.take(&a, Some(uuid::Uuid::nil())),
        Err(BoardError::NotFound(uuid::Uuid::nil()))
    );
}

#[test]
fn a_claimed_ready_card_is_not_half_taken() {
    let b = board();
    let id = ready(&b, "a");
    let h = who(&b, "nano", "human");
    b.claim(id, &h).expect("a person parked a claim on it");
    let dev = who(&b, "claude", "builder");
    assert_eq!(
        b.take(&dev, Some(id)),
        Err(BoardError::Claimed("nano".into()))
    );
    assert_eq!(b.get(id).expect("card").column, Column::Ready, "not moved");
}
