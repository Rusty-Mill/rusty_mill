//! `TickStore` specifics: how a card lands on its rusty_tick task, and how the
//! owner's edits in rusty_tick reach the board (ADR-0012).

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{BoardError, Card, Column, Event, EventKind};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::TickStore;
use serde_json::json;

mod fake_tick;
use fake_tick::FakeTick;

fn clock() -> i64 {
    42
}

fn board() -> (Board<TickStore<FakeTick>>, FakeTick) {
    let tick = FakeTick::default();
    let projects = Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#).expect("projects");
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock)
        .with_projects(projects);
    (board, tick)
}

#[test]
fn a_task_tagged_agent_in_rusty_tick_is_on_the_board_and_untagging_removes_it() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    tick.owner_edit(card.id, |t| t["tags"] = json!(["board:ready", "Agent"]));
    tick.owner_edit(card.id, |t| t["tags"] = json!(["agent", "board:ready"]));
    assert_eq!(board.get(card.id).expect("get").column, Column::Ready);

    tick.owner_edit(card.id, |t| t["tags"] = json!(["board:ready"]));
    assert_eq!(board.get(card.id), Err(BoardError::NotFound(card.id)));
    assert!(board.list(None).expect("list").is_empty());
}

#[test]
fn an_owner_edit_in_rusty_tick_is_read_next_and_shows_as_an_event() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");

    tick.owner_edit(card.id, |t| t["title"] = json!("renamed on the phone"));
    let read = board.get(card.id).expect("get");
    assert_eq!((read.title.as_str(), read.rev), ("renamed on the phone", 1));

    let events = board.events(cursor, 10).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].kind, events[0].actor.as_str()),
        (EventKind::Edited, "tick")
    );
    // The board writes on top of the owner's edit, not over it.
    let moved = board
        .move_card(card.id, Column::Ready, &human)
        .expect("move");
    assert_eq!(
        (moved.title.as_str(), moved.rev),
        ("renamed on the phone", 2)
    );
}

#[test]
fn a_write_from_a_stale_read_conflicts() {
    let tick = FakeTick::default();
    let store = TickStore::new(tick.clone());
    let card = Card::new("a", "", vec![], 0).expect("card");
    store
        .insert(&card, Event::of(&card, EventKind::Created, "x"))
        .expect("insert");
    tick.owner_edit(card.id, |t| t["notes"] = json!("owner"));

    let human = Roles::default().actor("nano", "human").expect("role");
    let stale = card.comment(&human, "late", 1).expect("comment");
    assert_eq!(
        store.replace(&stale, Event::of(&stale, EventKind::Commented, "x")),
        Err(BoardError::Conflict)
    );
    assert!(tick.docs("comment").is_empty());
}

#[test]
fn done_closes_the_task_and_a_task_closed_in_rusty_tick_is_done() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    for column in [Column::Ready, Column::Building, Column::Qa, Column::Review] {
        board.move_card(card.id, column, &human).expect("move");
        assert_eq!(tick.task(card.id)["status"], "open");
    }
    board
        .move_card(card.id, Column::Done, &human)
        .expect("done");
    assert_eq!(tick.task(card.id)["status"], "done");

    let other = board.add(&human, "b", "", vec![]).expect("add");
    tick.owner_edit(other.id, |t| t["status"] = json!("wontdo"));
    assert_eq!(board.get(other.id).expect("get").column, Column::Done);
}

#[test]
fn a_card_recorded_in_done_closes_its_task() {
    let (board, tick) = board();
    let builder = board.roles().actor("claude", "builder").expect("role");
    let card = board
        .add_done_in(&builder, "default", "shipped in #1", "", vec![])
        .expect("add done");
    assert_eq!(tick.task(card.id)["status"], "done");
    assert_eq!(board.get(card.id).expect("get").column, Column::Done);
    assert_eq!(tick.task(card.id)["etag"], "1", "one write: created closed");

    // A refused create leaves nothing behind, so a retry cannot duplicate it.
    tick.fail_next("POST", "/tasks");
    assert!(
        board
            .add_done_in(&builder, "default", "again", "", vec![])
            .is_err()
    );
    let titles: Vec<String> = board
        .list(None)
        .expect("list")
        .into_iter()
        .map(|c| c.title)
        .collect();
    assert_eq!(titles, ["shipped in #1"]);
}

#[test]
fn the_default_project_is_the_inbox_and_a_project_is_a_list() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let inbox = tick.task(card.id)["listId"].clone();
    assert_eq!(inbox, json!("00000000-0000-7000-8000-000000000001"));

    let moved = board.assign(card.id, "orch", &human).expect("assign");
    assert_eq!(moved.project, "orch");
    assert_ne!(tick.task(card.id)["listId"], inbox);
    assert_eq!(board.get(card.id).expect("get").project, "orch");
}

#[test]
fn a_done_card_moves_to_another_list_and_its_task_stays_closed() {
    let (board, tick) = board();
    let builder = board.roles().actor("claude", "builder").expect("role");
    let card = board
        .add_done_in(&builder, "default", "shipped", "", vec![])
        .expect("add done");
    let inbox = tick.task(card.id)["listId"].clone();
    board.assign(card.id, "orch", &builder).expect("assign");
    assert_ne!(tick.task(card.id)["listId"], inbox);
    assert_eq!(tick.task(card.id)["status"], "done");
    let read = board.get(card.id).expect("get");
    assert_eq!((read.project.as_str(), read.column), ("orch", Column::Done));
}

#[test]
fn deps_and_comments_are_rusty_tick_documents() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let first = board.add(&human, "first", "", vec![]).expect("add");
    let second = board.add(&human, "second", "", vec![]).expect("add");
    let planned = board
        .plan(second.id, &human, Some(5), Some(vec![first.id]))
        .expect("plan");
    assert_eq!(
        (planned.priority, planned.deps.clone()),
        (5, vec![first.id])
    );
    assert_eq!(tick.task(second.id)["priority"], 5);
    let doc = tick.doc("board", second.id).expect("board doc");
    assert_eq!(doc["deps"], json!([first.id]));

    board.comment(second.id, &human, "why").expect("comment");
    let comments = tick.docs("comment");
    assert_eq!(comments.len(), 1);
    assert_eq!(
        (
            &comments[0]["text"],
            &comments[0]["by"],
            &comments[0]["taskId"]
        ),
        (&json!("why"), &json!("nano"), &json!(second.id))
    );
    let read = board.get(second.id).expect("get");
    assert_eq!(
        (read.comments.len(), read.deps.clone()),
        (1, vec![first.id])
    );
}

#[test]
fn a_claim_rusty_tick_would_lowercase_is_refused() {
    let store = TickStore::new(FakeTick::default());
    let mut card = Card::new("a", "", vec![], 0).expect("card");
    card.claim = Some("Dev".into());
    assert!(matches!(
        store.insert(&card, Event::of(&card, EventKind::Created, "x")),
        Err(BoardError::Invalid(_))
    ));
}

fn link(execution: &str) -> rusty_mill_board::domain::ExecutionLink {
    rusty_mill_board::domain::ExecutionLink {
        execution: execution.into(),
        goal: "g".into(),
        bbp_task: "b".into(),
        attempt: "a".into(),
        assignment: "s".into(),
        generation: 1,
    }
}

/// Two boards (independent adapters over one rusty_tick) and a card both have read.
fn two_readers() -> (TickStore<FakeTick>, TickStore<FakeTick>, FakeTick, Card) {
    let tick = FakeTick::default();
    let (a, b) = (TickStore::new(tick.clone()), TickStore::new(tick.clone()));
    let card = Card::new("a", "", vec![], 0).expect("card");
    a.insert(&card, Event::of(&card, EventKind::Created, "x"))
        .expect("insert");
    let read = a.get(card.id).expect("get").expect("card");
    (a, b, tick, read)
}

#[test]
fn of_two_bindings_from_the_same_read_only_the_first_lands() {
    let (a, b, tick, read) = two_readers();
    let first = read.bind_execution(link("e1"), 1).expect("bind");
    let second = read.bind_execution(link("e2"), 1).expect("bind");
    a.replace(&first, Event::of(&first, EventKind::ExecutionLinked, "a"))
        .expect("first");
    assert_eq!(
        b.replace(&second, Event::of(&second, EventKind::ExecutionLinked, "b")),
        Err(BoardError::Conflict)
    );
    assert_eq!(
        tick.doc("board", read.id).expect("doc")["execution"]["execution"],
        "e1"
    );

    // Through the board, the loser re-reads and is refused by the domain rule.
    let board = Board::with_clock(b, Roles::default(), clock);
    let dev = board.roles().actor("dev", "builder").expect("role");
    assert!(matches!(
        board.bind_execution(read.id, link("e2"), &dev),
        Err(BoardError::Refused(_))
    ));
}

#[test]
fn a_dependency_write_and_a_binding_from_the_same_read_do_not_both_land() {
    let (a, b, tick, read) = two_readers();
    let planned = read
        .plan(None, Some(vec![uuid::Uuid::now_v7()]), 1)
        .expect("plan");
    let bound = read.bind_execution(link("e1"), 1).expect("bind");
    a.replace(&planned, Event::of(&planned, EventKind::Edited, "a"))
        .expect("plan");
    assert_eq!(
        b.replace(&bound, Event::of(&bound, EventKind::ExecutionLinked, "b")),
        Err(BoardError::Conflict)
    );
    let doc = tick.doc("board", read.id).expect("doc");
    assert_eq!(
        (doc["deps"].as_array().map(Vec::len), &doc["execution"]),
        (Some(1), &json!(null))
    );
}

#[test]
fn a_comment_the_owner_adds_meanwhile_is_kept() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let store = TickStore::new(tick.clone());
    let read = store.get(card.id).expect("get").expect("card");
    tick.owner_comment(card.id, "from the phone");
    let mine = read.comment(&human, "from the board", 1).expect("comment");
    store
        .replace(&mine, Event::of(&mine, EventKind::Commented, "nano"))
        .expect("append");
    let texts: Vec<String> = board
        .get(card.id)
        .expect("get")
        .comments
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(
        texts.contains(&"from the phone".to_owned())
            && texts.contains(&"from the board".to_owned())
    );
}

#[test]
fn overlapping_board_comments_both_land_one_after_the_other() {
    let (a, b, _tick, read) = two_readers();
    let human = Roles::default().actor("nano", "human").expect("role");
    let first = read.comment(&human, "one", 1).expect("comment");
    let second = read.comment(&human, "two", 1).expect("comment");
    a.replace(&first, Event::of(&first, EventKind::Commented, "a"))
        .expect("first");
    assert_eq!(
        b.replace(&second, Event::of(&second, EventKind::Commented, "b")),
        Err(BoardError::Conflict),
        "the second append from the same read is refused, not merged blindly"
    );
    let board = Board::with_clock(b, Roles::default(), clock);
    board.comment(read.id, &human, "two").expect("retried");
    assert_eq!(board.get(read.id).expect("get").comments.len(), 2);
}

#[test]
fn a_plan_interrupted_between_its_two_writes_is_finished_by_planning_again() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let dep = board.add(&human, "dep", "", vec![]).expect("add");
    let card = board.add(&human, "card", "", vec![]).expect("add");
    tick.fail_next("PUT", "/docs/board/");
    assert!(
        board
            .plan(card.id, &human, Some(5), Some(vec![dep.id]))
            .is_err()
    );
    let half = board.get(card.id).expect("get");
    assert_eq!((half.priority, half.deps.len()), (5, 0));

    let done = board
        .plan(card.id, &human, Some(5), Some(vec![dep.id]))
        .expect("plan again");
    assert_eq!((done.priority, done.deps.clone()), (5, vec![dep.id]));
}

#[test]
fn an_import_interrupted_before_its_link_keeps_the_id_that_names_its_issue() {
    let (board, tick) = board();
    let sync = board.roles().actor("github-sync", "human").expect("role");
    tick.fail_next("PUT", "/docs/board/");
    assert!(board.import(&sync, "from issue", "", vec![], 7).is_err());
    let card = board
        .get(rusty_mill_board::domain::import_id(7))
        .expect("the task is there");
    assert_eq!(card.issue, None, "the link is what the sync restores");
    assert_eq!(
        board.import(&sync, "from issue", "", vec![], 7),
        Err(BoardError::Conflict),
        "and the import is not repeated"
    );
}

#[test]
fn a_write_in_two_steps_returns_the_revision_a_read_sees_and_the_next_edit_names() {
    let (board, _tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let sync = board.roles().actor("github-sync", "human").expect("role");

    // An import is a task and then its board document: two version moves.
    let imported = board
        .import(&sync, "from issue", "", vec![], 7)
        .expect("import");
    assert_eq!(imported.rev, board.get(imported.id).expect("get").rev);
    board
        .edit_at(
            imported.id,
            &human,
            Some(imported.rev),
            Some("t"),
            None,
            None,
        )
        .expect("edit at the returned revision");

    // So is a plan with a priority and dependencies.
    let dep = board.add(&human, "dep", "", vec![]).expect("add");
    let card = board.add(&human, "card", "", vec![]).expect("add");
    let planned = board
        .plan(card.id, &human, Some(5), Some(vec![dep.id]))
        .expect("plan");
    assert_eq!(planned.rev, board.get(card.id).expect("get").rev);
    let edited = board
        .edit_at(card.id, &human, Some(planned.rev), Some("t"), None, None)
        .expect("edit at the returned revision");
    assert_eq!(edited.rev, board.get(card.id).expect("get").rev);
}

/// A write landing between a card read's task and its documents: the read never pairs the
/// newer document with the older revision; it reads again and returns both as they now stand.
#[test]
fn a_write_between_the_task_and_its_document_is_read_again_not_mixed() {
    let (a, _b, tick, read) = two_readers();
    let raced = json!({ "v": 1, "deps": [], "execution": link("raced") });
    tick.race_after("GET", "/docs/board", 1, read.id, raced);

    let card = a.get(read.id).expect("get").expect("card");
    assert_eq!(
        (card.rev, card.execution.map(|l| l.execution)),
        (read.rev + 1, Some("raced".to_owned())),
        "the document and the revision are from the same moment"
    );
}

/// A task that moves under every read is a conflict, not a mixed card or an endless loop.
#[test]
fn a_card_that_keeps_moving_under_its_reads_is_a_conflict() {
    let (a, _b, tick, read) = two_readers();
    let raced = json!({ "v": 1, "deps": [] });
    tick.race_after("GET", "/docs/board", 1_000, read.id, raced);

    assert_eq!(a.get(read.id), Err(BoardError::Conflict));
}
