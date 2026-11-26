//! Separation of duties on the factory workflow (#447): a card from Backlog to the merge gate,
//! made by one provider, reviewed by another, its gates crossed by the approver proxy, and its
//! merge confirmed by the owner.

use std::collections::BTreeMap;

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Actor, BoardError, Capability, Card, Column, text_hash};
use rusty_mill_board::duties::{Approval, Duty};
use rusty_mill_board::store::MemStore;
use rusty_mill_board::workflow::Workflow;

const FACTORY: &str = include_str!("../deploy/factory.workflow.json");

fn col(id: &str) -> Column {
    Column::new(id).expect("id")
}

fn board() -> Board<MemStore> {
    use Capability::*;
    let mut roles = Roles::default().0;
    roles.insert("approver".into(), [Approve].into());
    roles.insert(
        "maker".into(),
        [Triage, Plan, Architect, Build, Test, Review, Close].into(),
    );
    let flow: Workflow = serde_json::from_str(FACTORY).expect("factory workflow");
    Board::new(MemStore::default(), Roles(roles)).with_workflow(flow)
}

fn who(b: &Board<MemStore>, name: &str, role: &str, provider: &str) -> Actor {
    let mut a = b.roles().actor(name, role).expect("role");
    a.provider = Some(provider.to_owned());
    a
}

fn approval<'a>(card: &Card, to: &'a Column, pr_head: Option<&'a str>) -> Approval<'a> {
    Approval {
        to,
        digest: text_hash(&card.title, &card.body),
        verbatim: "Approve card #1, the gate? Yes.",
        source: "cmsg_test",
        pr_head,
    }
}

fn refused(r: Result<Card, BoardError>) -> bool {
    matches!(r, Err(BoardError::Refused(_)))
}

#[test]
fn a_card_crosses_the_factory_with_duties_split() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let sibling = who(&b, "cloud#2", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");

    let card = b.add(&nano, "feature", "do it", vec![]).expect("add");
    let ready = col("ready");
    let card = b
        .approve(card.id, &proxy, &approval(&card, &ready, None))
        .expect("the design gate into Ready moves at once");
    assert_eq!(card.column, ready);

    let card = b.take(&maker, None).expect("take").expect("a card");
    assert_eq!(card.column, col("planning"));
    let arch = col("architecting");
    assert!(
        refused(b.move_card(card.id, &arch, &maker)),
        "an author may not review"
    );
    b.release(card.id, &maker).expect("release");
    b.claim(card.id, &sibling).expect("claim");
    assert!(
        refused(b.move_card(card.id, &arch, &sibling)),
        "nor its provider"
    );
    b.release(card.id, &sibling).expect("release");
    b.claim(card.id, &dots).expect("claim");
    b.move_card(card.id, &arch, &dots)
        .expect("a foreign review moves it");

    b.claim(card.id, &maker).expect("design");
    let building = col("building");
    let card = b.get(card.id).expect("get");
    assert!(
        refused(b.approve(card.id, &proxy, &approval(&card, &building, None))),
        "no review of the design yet"
    );
    assert!(
        refused(b.review(card.id, &maker, true, "mine")),
        "self review"
    );
    b.review(card.id, &dots, true, "sound").expect("review");
    let card = b.get(card.id).expect("get");
    let card = b
        .approve(card.id, &proxy, &approval(&card, &building, None))
        .expect("the design gate");
    assert_eq!(card.column, building);
    assert_eq!(card.claim, None);

    b.claim(card.id, &maker).expect("build");
    b.release(card.id, &maker).expect("release");
    b.claim(card.id, &dots).expect("claim");
    b.move_card(card.id, &col("v_and_ving"), &dots)
        .expect("code review");

    let card = b.get(card.id).expect("get");
    let done = Column::DONE;
    let filed = b
        .approve(card.id, &proxy, &approval(&card, &done, Some("abc123")))
        .expect("a merge approval is filed");
    assert_eq!(filed.column, col("v_and_ving"), "never moves");
    assert!(matches!(
        filed.duties.last(),
        Some(Duty::Approval { pending: true, .. })
    ));
    let confirmed = b
        .approve(card.id, &nano, &approval(&filed, &done, Some("abc123")))
        .expect("the owner's tap");
    assert_eq!(confirmed.column, col("v_and_ving"));
    assert!(matches!(
        confirmed.duties.last(),
        Some(Duty::Approval { pending: false, confirmed_by: Some(n), .. }) if n == "nano"
    ));
    let kinds: BTreeMap<&str, usize> = confirmed.duties.iter().fold(BTreeMap::new(), |mut m, d| {
        let k = match d {
            Duty::Author { .. } => "author",
            Duty::Review { .. } => "review",
            Duty::Approval { .. } => "approval",
        };
        *m.entry(k).or_default() += 1;
        m
    });
    assert_eq!(kinds["approval"], 3, "{:?}", confirmed.duties);
    assert_eq!(kinds["review"], 3);
}

#[test]
fn the_proxy_cannot_move_a_gate_with_card_move() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let card = b.add(&nano, "x", "", vec![]).expect("add");
    assert!(matches!(
        b.move_card(card.id, &col("ready"), &proxy),
        Err(BoardError::Forbidden(_))
    ));
}

#[test]
fn the_owner_moving_a_gate_is_recorded() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let card = b.add(&nano, "x", "", vec![]).expect("add");
    let card = b.move_card(card.id, &col("ready"), &nano).expect("move");
    assert!(matches!(
        card.duties.as_slice(),
        [Duty::Approval { actor, verbatim, .. }] if actor == "nano" && verbatim.is_empty()
    ));
}

fn approvals(c: &Card) -> Vec<&Duty> {
    c.duties
        .iter()
        .filter(|d| matches!(d, Duty::Approval { .. }))
        .collect()
}

/// A card through Building to V&Ving, built by `maker` and reviewed by `dots`.
fn at_v_and_ving(b: &Board<MemStore>, nano: &Actor, maker: &Actor, dots: &Actor) -> Card {
    let card = b.add(nano, "feature", "do it", vec![]).expect("add");
    let card = b.move_card(card.id, &col("ready"), nano).expect("ready");
    let card = b
        .move_card(card.id, &col("planning"), nano)
        .expect("planning");
    let card = b
        .move_card(card.id, &col("architecting"), nano)
        .expect("architecting");
    let card = b
        .move_card(card.id, &col("building"), nano)
        .expect("building");
    b.claim(card.id, maker).expect("build");
    b.release(card.id, maker).expect("release");
    b.claim(card.id, dots).expect("claim");
    b.move_card(card.id, &col("v_and_ving"), dots)
        .expect("review")
}

#[test]
fn a_close_into_done_is_not_recorded_as_an_approval() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let before = approvals(&card).len();
    b.release(card.id, &dots).expect("release");
    b.claim(card.id, &maker).expect("claim");
    let done = b.move_card(card.id, &Column::DONE, &maker).expect("close");
    assert_eq!(approvals(&done).len(), before, "{:?}", done.duties);
}

#[test]
fn an_unblock_filed_on_old_text_expires_and_the_card_stays() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let card = b.add(&nano, "x", "body A", vec![]).expect("add");
    let card = b.move_card(card.id, &col("blocked"), &nano).expect("block");
    let ready = col("ready");
    b.approve(card.id, &proxy, &approval(&card, &ready, None))
        .expect("filed");
    let edited = b
        .edit(card.id, &nano, None, Some("body B"), None)
        .expect("edit");
    let tapped = b
        .approve(card.id, &nano, &approval(&edited, &ready, None))
        .expect("the stale filing does not lock the owner out");
    assert_eq!(
        tapped.column,
        col("blocked"),
        "no gate crossed on unread text"
    );
    assert!(
        matches!(
            approvals(&tapped).as_slice(),
            [Duty::Approval {
                expired: true,
                pending: false,
                confirmed_by: None,
                ..
            }]
        ),
        "the filing expired and nothing was approved: {:?}",
        tapped.duties
    );
    let moved = b
        .approve(card.id, &nano, &approval(&tapped, &ready, None))
        .expect("the owner approves on a fresh readback");
    assert_eq!(moved.column, ready);
}

#[test]
fn refiling_a_gate_does_not_grow_the_ledger() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let card = b.add(&nano, "x", "", vec![]).expect("add");
    let card = b.move_card(card.id, &col("blocked"), &nano).expect("block");
    let ready = col("ready");
    let first = b
        .approve(card.id, &proxy, &approval(&card, &ready, None))
        .expect("filed");
    for _ in 0..20 {
        b.approve(card.id, &proxy, &approval(&card, &ready, None))
            .expect("refiled");
    }
    let last = b.get(card.id).expect("get");
    assert_eq!(last.duties.len(), first.duties.len());
}

#[test]
fn labelled_instances_are_one_hand() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let maker = who(&b, "cli/a0", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let card = b.move_card(card.id, &col("building"), &nano).expect("back");
    let before = card.duties.len();
    for i in 1..30 {
        let instance = who(&b, &format!("cli/a{i}"), "maker", "anthropic");
        b.claim(card.id, &instance).expect("claim");
        b.release(card.id, &instance).expect("release");
    }
    assert_eq!(b.get(card.id).expect("get").duties.len(), before);
    let sibling = who(&b, "cli/z", "maker", "local");
    b.claim(card.id, &sibling).expect("claim");
    assert!(
        refused(b.move_card(card.id, &col("v_and_ving"), &sibling)),
        "another instance of the author's credential cannot review"
    );
}

#[test]
fn a_recorders_close_is_not_recorded_as_an_approval() {
    use Capability::*;
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let recorder = Actor {
        name: "rec".into(),
        caps: [Build, Record].into(),
        projects: None,
        provider: Some("other".into()),
    };
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let before = approvals(&card).len();
    b.release(card.id, &dots).expect("release");
    b.add_evidence(
        card.id,
        &["https://github.com/o/r/pull/1".to_owned()],
        &nano,
    )
    .expect("evidence");
    b.claim(card.id, &recorder).expect("claim");
    let done = b
        .move_card(card.id, &Column::DONE, &recorder)
        .expect("record merged work");
    assert_eq!(approvals(&done).len(), before, "{:?}", done.duties);
}

#[test]
fn a_v_and_v_verdict_holds_the_merge_gate() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let done = Column::DONE;
    let card = b.review(card.id, &dots, false, "tests fail").expect("fail");
    assert!(refused(b.approve(
        card.id,
        &proxy,
        &approval(&card, &done, Some("abc"))
    )));
    let card = b.review(card.id, &dots, true, "fixed").expect("pass");
    let filed = b
        .approve(card.id, &proxy, &approval(&card, &done, Some("abc")))
        .expect("filed");
    assert!(
        approvals(&filed)
            .iter()
            .any(|d| matches!(d, Duty::Approval { pending: true, .. }))
    );
}

#[test]
fn claim_cycles_do_not_grow_the_ledger() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let card = b.move_card(card.id, &col("building"), &nano).expect("back");
    let len = |b: &Board<MemStore>| b.get(card.id).expect("get").duties.len();
    b.claim(card.id, &maker).expect("claim");
    b.release(card.id, &maker).expect("release");
    let once = len(&b);
    for _ in 0..50 {
        b.claim(card.id, &dots).expect("other");
        b.release(card.id, &dots).expect("release");
        b.claim(card.id, &maker).expect("claim");
        b.release(card.id, &maker).expect("release");
    }
    assert_eq!(len(&b), once);
}

#[test]
fn a_pending_approval_expires_when_the_card_leaves_its_gate() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let card = b.add(&nano, "x", "", vec![]).expect("add");
    let card = b.move_card(card.id, &col("blocked"), &nano).expect("block");
    let ready = col("ready");
    b.approve(card.id, &proxy, &approval(&card, &ready, None))
        .expect("filed");
    b.move_card(card.id, &ready, &nano)
        .expect("owner unblocks by hand");
    let back = b
        .move_card(card.id, &col("blocked"), &nano)
        .expect("blocked again");
    assert!(back.duties.iter().any(|d| matches!(
        d,
        Duty::Approval {
            expired: true,
            pending: false,
            ..
        }
    )));
    let mine = b
        .approve(card.id, &nano, &approval(&back, &ready, None))
        .expect("owner's own");
    assert!(
        !mine.duties.iter().any(|d| matches!(
            d,
            Duty::Approval {
                confirmed_by: Some(_),
                ..
            }
        )),
        "the expired filing is not confirmed: {:?}",
        mine.duties
    );
}

#[test]
fn reviews_cannot_fill_the_ledger() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = at_v_and_ving(&b, &nano, &maker, &dots);
    let before = card.duties.len();
    for i in 0..300 {
        b.review(card.id, &dots, i % 2 == 0, "again")
            .expect("review");
    }
    assert_eq!(b.get(card.id).expect("get").duties.len(), before + 1);
}

#[test]
fn a_planning_review_does_not_carry_the_design_gate() {
    let b = board();
    let nano = who(&b, "nano", "human", "human");
    let proxy = who(&b, "nano-proxy", "approver", "human");
    let maker = who(&b, "cloud#1", "maker", "anthropic");
    let dots = who(&b, "dots", "reviewer", "openai");
    let card = b.add(&nano, "x", "", vec![]).expect("add");
    b.move_card(card.id, &col("ready"), &nano).expect("ready");
    let card = b.take(&maker, None).expect("take").expect("a card");
    b.release(card.id, &maker).expect("release");
    b.claim(card.id, &dots).expect("claim");
    b.review(card.id, &dots, true, "plan is sound")
        .expect("planning review");
    let card = b
        .move_card(card.id, &col("architecting"), &dots)
        .expect("review move");
    assert!(
        refused(b.approve(card.id, &proxy, &approval(&card, &col("building"), None))),
        "nobody reviewed Architecting"
    );
}
