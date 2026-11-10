//! `TickStore` specifics: how a card lands on its rusty_tick task, and how the
//! owner's edits in rusty_tick reach the board (ADR-0012).

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{
    BoardError, Card, CardSummary, Column, Dates, Edit, Event, EventKind, Plan, Shelf, Span,
};
use rusty_mill_board::milestones::{MilestoneEdit, MilestoneStore};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::Store;
use rusty_mill_board::tick::{TickApi, TickRequest, TickStore};
use serde_json::json;
use std::sync::{Arc, Barrier};

/// An edit of the title only.
fn title(t: &str) -> Edit<'_> {
    Edit {
        title: Some(t),
        ..Edit::default()
    }
}

/// An edit of the prompt only.
fn prompt(p: &str) -> Edit<'_> {
    Edit {
        prompt: Some(p),
        ..Edit::default()
    }
}

/// A plan of a priority and dependencies only.
fn plan(priority: Option<u8>, deps: Option<Vec<uuid::Uuid>>) -> Plan {
    Plan {
        priority,
        deps,
        ..Plan::default()
    }
}

mod fake_tick;
use fake_tick::FakeTick;

/// Lets the first task patch mutate rusty_tick, then holds its response so a
/// later patch can finish recording first.
#[derive(Clone)]
struct ReverseFinishTick {
    inner: FakeTick,
    first_patch: Arc<std::sync::atomic::AtomicBool>,
    mutated: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl ReverseFinishTick {
    fn new(inner: FakeTick) -> Self {
        Self {
            inner,
            first_patch: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            mutated: Arc::new(Barrier::new(2)),
            release: Arc::new(Barrier::new(2)),
        }
    }
}

impl TickApi for ReverseFinishTick {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        let result = self.inner.send(request);
        if request.method == "PATCH"
            && request.path.starts_with("/tasks/")
            && self
                .first_patch
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.mutated.wait();
            self.release.wait();
        }
        result
    }
}

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
    assert_eq!(board.get(card.id).expect("get").column, Column::READY);

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
        .move_card(card.id, &Column::READY, &human)
        .expect("move");
    assert_eq!(
        (moved.title.as_str(), moved.rev),
        ("renamed on the phone", 2)
    );
}

#[test]
fn reversed_same_card_write_finishes_keep_the_newest_seen_version() {
    let tick = FakeTick::default();
    let gated = ReverseFinishTick::new(tick.clone());
    let store = Arc::new(TickStore::new(gated.clone()));
    let card = Card::new("a", "", vec![], 0).expect("card");
    store
        .insert(&card, Event::of(&card, EventKind::Created, "insert"))
        .expect("insert");
    let cursor = store.last_event_id().expect("prime observations");

    let first = card.edit(Some("first"), None, None, 1).expect("edit");
    let first_store = Arc::clone(&store);
    let first_write = std::thread::spawn(move || {
        first_store
            .replace(&first, Event::of(&first, EventKind::Edited, "first"))
            .expect("first write")
    });

    gated.mutated.wait();
    let after_first = store.get(card.id).expect("get").expect("card");
    assert_eq!(
        after_first.rev, 1,
        "the first mutation landed before it finished"
    );
    let second = after_first
        .edit(None, Some("second"), None, 2)
        .expect("edit");
    assert_eq!(
        store
            .replace(&second, Event::of(&second, EventKind::Edited, "second"),)
            .expect("second write"),
        2
    );
    gated.release.wait();
    assert_eq!(first_write.join().expect("first thread"), 1);

    let writes = store.events_since(cursor, 10).expect("events");
    assert_eq!(
        writes.len(),
        2,
        "no synthetic event follows the board writes"
    );
    assert_eq!(
        writes
            .iter()
            .map(|event| event.actor.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"],
        "event ids follow finish/recording order"
    );

    let outside_cursor = writes.last().map(|event| event.id);
    tick.owner_edit(card.id, |task| task["title"] = json!("outside"));
    let outside = store
        .events_since(outside_cursor, 10)
        .expect("outside event");
    assert_eq!(outside.len(), 1, "a later outside change is still observed");
    assert_eq!(
        (outside[0].kind, outside[0].actor.as_str()),
        (EventKind::Edited, "tick")
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
    for column in [Column::READY, Column::BUILDING, Column::QA, Column::REVIEW] {
        board.move_card(card.id, &column, &human).expect("move");
        assert_eq!(tick.task(card.id)["status"], "open");
    }
    board
        .move_card(card.id, &Column::DONE, &human)
        .expect("done");
    assert_eq!(tick.task(card.id)["status"], "done");
    let shipped = tick.task(card.id)["completedMs"].as_i64();
    assert!(shipped.is_some());
    assert_eq!(board.get(card.id).expect("get").done_ms, shipped);

    let other = board.add(&human, "b", "", vec![]).expect("add");
    tick.owner_edit(other.id, |t| t["status"] = json!("wontdo"));
    let closed = board.get(other.id).expect("get");
    assert_eq!(closed.column, Column::DONE);
    assert_eq!(closed.done_ms, None, "won't-do is closed, not shipped");
}

#[test]
fn a_card_recorded_in_done_closes_its_task() {
    let (board, tick) = board();
    let builder = board.roles().actor("claude", "builder").expect("role");
    let card = board
        .add_done_in(&builder, "default", "shipped in #1", "", vec![])
        .expect("add done");
    assert_eq!(tick.task(card.id)["status"], "done");
    assert_eq!(board.get(card.id).expect("get").column, Column::DONE);
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
    assert_eq!((read.project.as_str(), read.column), ("orch", Column::DONE));
}

#[test]
fn links_live_in_the_board_document_and_are_left_out_while_empty() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let old = board.add(&human, "old", "", vec![]).expect("add");
    let newer = board.add(&human, "newer", "", vec![]).expect("add");
    board
        .plan(old.id, &human, &plan(None, Some(vec![newer.id])))
        .expect("plan");
    let doc = tick.doc("board", old.id).expect("board doc");
    assert!(
        doc.get("supersededBy").is_none() && doc.get("related").is_none(),
        "documents without links stay as before: {doc}"
    );

    let links = Plan {
        deps: Some(vec![]),
        superseded_by: Some(vec![newer.id]),
        related: Some(vec![newer.id]),
        ..Plan::default()
    };
    board.plan(old.id, &human, &links).expect("link");
    let doc = tick.doc("board", old.id).expect("board doc");
    assert_eq!(
        (&doc["supersededBy"], &doc["related"]),
        (&json!([newer.id]), &json!([newer.id]))
    );
    let read = board.get(old.id).expect("get");
    assert_eq!(
        (read.superseded_by, read.related),
        (vec![newer.id], vec![newer.id])
    );
}

#[test]
fn parent_and_milestone_live_in_the_board_document_and_are_left_out_while_unset() {
    let (board, tick) = board();
    let board = board.with_milestones(Box::new(TickStore::new(tick.clone())));
    let human = board.roles().actor("nano", "human").expect("role");
    let arc = board.add(&human, "arc 2", "", vec![]).expect("add");
    let slice = board.add(&human, "S2", "", vec![]).expect("add");
    let doc = tick.doc("board", slice.id).unwrap_or_default();
    assert!(doc.get("parent").is_none() && doc.get("milestone").is_none());

    let beta = board
        .add_milestone(&human, "default", "Beta", 1_800_000_000_000)
        .expect("milestone");
    let links = Plan {
        parent: Some(Some(arc.id)),
        milestone: Some(Some(beta.id)),
        ..Plan::default()
    };
    board.plan(slice.id, &human, &links).expect("link");
    let doc = tick.doc("board", slice.id).expect("board doc");
    assert_eq!(
        (&doc["parent"], &doc["milestone"]),
        (&json!(arc.id), &json!(beta.id))
    );
    let read = board.get(slice.id).expect("get");
    assert_eq!((read.parent, read.milestone), (Some(arc.id), Some(beta.id)));
    let progress = board
        .milestone_progress(vec![beta])
        .expect("progress")
        .remove(0)
        .progress;
    assert_eq!(progress.total, 1);

    let clear = Plan {
        parent: Some(None),
        milestone: Some(None),
        ..Plan::default()
    };
    board.plan(slice.id, &human, &clear).expect("clear");
    let doc = tick.doc("board", slice.id).expect("board doc");
    assert!(doc.get("parent").is_none() && doc.get("milestone").is_none());
}

#[test]
fn a_target_date_lives_in_the_board_document_and_is_left_out_while_unset() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "dated", "", vec![]).expect("add");
    let target = |t: Option<i64>| Plan {
        target_ms: Some(t),
        ..Plan::default()
    };
    board
        .plan(card.id, &human, &target(Some(1_800_000_000_000)))
        .expect("set");
    assert_eq!(
        tick.doc("board", card.id).expect("doc")["targetMs"],
        json!(1_800_000_000_000_i64)
    );
    assert_eq!(
        board.get(card.id).expect("get").target_ms,
        Some(1_800_000_000_000)
    );
    board.plan(card.id, &human, &target(None)).expect("clear");
    let doc = tick.doc("board", card.id).expect("doc");
    assert!(
        doc.get("targetMs").is_none(),
        "an unset target is left out: {doc}"
    );
    assert_eq!(board.get(card.id).expect("get").target_ms, None);
}

#[test]
fn deps_and_comments_are_rusty_tick_documents() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let first = board.add(&human, "first", "", vec![]).expect("add");
    let second = board.add(&human, "second", "", vec![]).expect("add");
    let planned = board
        .plan(second.id, &human, &plan(Some(5), Some(vec![first.id])))
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
        .plan(&plan(None, Some(vec![uuid::Uuid::now_v7()])), 1)
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
    let cursor = board.cursor().expect("cursor");
    tick.fail_next("PUT", "/docs/board/");
    let failed = board.plan(card.id, &human, &plan(Some(5), Some(vec![dep.id])));
    let half = board.get(card.id).expect("get");
    assert!(
        matches!(failed, Err(BoardError::PartlySaved { rev, .. }) if rev == half.rev),
        "reported as partly saved, with the revision the task write left: {failed:?}"
    );
    assert_eq!((half.priority, half.deps.len()), (5, 0));
    let outside = board.events(cursor, 10).expect("events after failed write");
    assert_eq!(outside.len(), 1, "the partial task write is observed once");
    assert_eq!(
        (outside[0].card, outside[0].kind, outside[0].actor.as_str()),
        (card.id, EventKind::Edited, "tick"),
        "a failed board write removes its marker and remains a genuine change"
    );

    let done = board
        .plan(card.id, &human, &plan(Some(5), Some(vec![dep.id])))
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
        .edit_at(imported.id, &human, Some(imported.rev), &title("t"))
        .expect("edit at the returned revision");

    // So is a plan with a priority and dependencies.
    let dep = board.add(&human, "dep", "", vec![]).expect("add");
    let card = board.add(&human, "card", "", vec![]).expect("add");
    let planned = board
        .plan(card.id, &human, &plan(Some(5), Some(vec![dep.id])))
        .expect("plan");
    assert_eq!(planned.rev, board.get(card.id).expect("get").rev);
    let edited = board
        .edit_at(card.id, &human, Some(planned.rev), &title("t"))
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

#[test]
fn an_archived_card_is_one_tag_on_its_closed_task() {
    let (board, tick) = board();
    let builder = board.roles().actor("claude", "builder").expect("role");
    let card = board
        .add_done_in(&builder, "default", "shipped", "", vec![])
        .expect("add done");
    board.archive(card.id, &builder).expect("archive");
    assert!(
        tick.task(card.id)["tags"]
            .as_array()
            .expect("tags")
            .contains(&json!("archived:42"))
    );
    assert_eq!(tick.task(card.id)["status"], "done");
    let read = board.get(card.id).expect("get");
    assert_eq!((read.archived_ms, read.tags.len()), (Some(42), 0));

    board.restore(card.id, &builder).expect("restore");
    assert_eq!(board.get(card.id).expect("get").archived_ms, None);
    assert!(
        !tick.task(card.id)["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .any(|t| t.as_str().is_some_and(|t| t.starts_with("archived:")))
    );

    // Only a Done card is archived: the tag added by hand to an open task means nothing.
    let open = board.add(&builder, "open", "", vec![]).expect("add");
    tick.owner_edit(open.id, |t| {
        t["tags"] = json!(["agent", "board:backlog", "archived:7"]);
    });
    assert_eq!(board.get(open.id).expect("get").archived_ms, None);
}

#[test]
fn the_archived_tag_is_the_boards_own() {
    let (board, _tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    assert!(matches!(
        board.add(&human, "a", "", vec!["archived:1".into()]),
        Err(BoardError::Invalid(_))
    ));
}

#[test]
fn restoring_an_aged_out_card_persists_its_restore_time_in_rusty_tick() {
    let (board, tick) = board();
    let builder = board.roles().actor("claude", "builder").expect("role");
    let card = board
        .add_done_in(&builder, "default", "old", "", vec![])
        .expect("add done");
    // Shipped 30 days before the board's clock.
    tick.owner_edit(card.id, |t| {
        t["completedMs"] = json!(42 - 30 * 86_400_000_i64)
    });
    let listed = |shelf| board.list_shelf(None, shelf, 14).expect("list").len();
    assert_eq!(listed(Shelf::Exclude), 0);

    board.restore(card.id, &builder).expect("restore");
    assert!(
        tick.task(card.id)["tags"]
            .as_array()
            .expect("tags")
            .contains(&json!("restored:42"))
    );
    let read = board.get(card.id).expect("reload");
    assert_eq!(
        (read.restored_ms, read.done_ms),
        (Some(42), Some(42 - 30 * 86_400_000))
    );
    assert_eq!(
        listed(Shelf::Exclude),
        1,
        "back on the board after a reload"
    );

    // Archiving again drops the restore time.
    board.archive(card.id, &builder).expect("archive");
    let tags = tick.task(card.id)["tags"].clone();
    assert!(
        tags.as_array()
            .expect("tags")
            .contains(&json!("archived:42"))
    );
    assert!(
        !tags
            .as_array()
            .expect("tags")
            .contains(&json!("restored:42"))
    );
}

#[test]
fn a_prompt_is_kept_in_the_board_document_and_read_back() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board
        .add(&human, "a", "the description", vec![])
        .expect("add");
    assert!(
        tick.docs("board").is_empty(),
        "no prompt and no links: no document"
    );
    let edited = board
        .edit_at(card.id, &human, None, &prompt("Do the thing."))
        .expect("set prompt");
    assert_eq!(edited.prompt, "Do the thing.");
    assert_eq!(tick.docs("board")[0]["prompt"], "Do the thing.");
    let read = board.get(card.id).expect("get");
    assert_eq!(
        (read.prompt.as_str(), read.body.as_str()),
        ("Do the thing.", "the description"),
        "the prompt is apart from the body"
    );
    let cleared = board
        .edit_at(card.id, &human, None, &prompt(""))
        .expect("clear");
    assert!(cleared.prompt.is_empty());
    assert!(
        tick.docs("board")[0].get("prompt").is_none(),
        "an empty prompt is left out of the document"
    );
    let too_long = "x".repeat(32 * 1024 + 1);
    assert!(matches!(
        board.edit_at(card.id, &human, None, &prompt(&too_long)),
        Err(BoardError::Invalid(_))
    ));
}

#[test]
fn a_prompt_that_escapes_past_the_document_limit_is_refused_before_any_write() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    // Within 32 KiB as typed, but every quote doubles once stored as JSON.
    let quotes = "\"".repeat(32 * 1024);
    let edit = Edit {
        title: Some("b"),
        prompt: Some(&quotes),
        ..Edit::default()
    };
    assert!(matches!(
        board.edit_at(card.id, &human, None, &edit),
        Err(BoardError::Invalid(_))
    ));
    let read = board.get(card.id).expect("get");
    assert_eq!(
        (read.title.as_str(), read.rev),
        ("a", card.rev),
        "the title is not applied without the prompt"
    );
    assert!(tick.docs("board").is_empty());
}

#[test]
fn moved_in_reads_rusty_tick_move_history_across_pages() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let early = board.add(&human, "early", "", vec![]).expect("add");
    let late = board.add(&human, "late", "", vec![]).expect("add");
    tick.record_move(early.id, 100);
    // More than one page of moves, so the adapter has to follow `next`.
    for at in 0..1_500 {
        tick.record_move(late.id, 500 + at);
    }
    let store = TickStore::new(tick.clone());
    let span = |from, to| Span { from, to };
    assert_eq!(
        store.moved_in(span(Some(0), Some(200))).expect("moved"),
        [early.id].into()
    );
    assert_eq!(
        store.moved_in(span(Some(1_900), None)).expect("moved"),
        [late.id].into()
    );
    assert!(
        store
            .moved_in(span(Some(5_000), None))
            .expect("moved")
            .is_empty()
    );
    // The server runs over a boxed store: it must reach rusty_tick's history too, not the
    // default that reads the (here empty) event log.
    let boxed: Box<dyn Store> = Box::new(TickStore::new(tick.clone()));
    assert_eq!(
        boxed.moved_in(span(Some(0), Some(200))).expect("moved"),
        [early.id].into()
    );
}

/// A title and prompt edit: two writes to rusty_tick, the task and then its board document.
fn title_and_prompt<'a>(t: &'a str, p: &'a str) -> Edit<'a> {
    Edit {
        title: Some(t),
        prompt: Some(p),
        ..Edit::default()
    }
}

#[test]
fn links_that_take_a_document_near_the_limit_past_it_are_refused_before_any_write() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add(&human, "b", "", vec![]).expect("add");
    let card = board.add(&human, "card", "", vec![]).expect("add");
    // 32,700 quotes store as 65,400 bytes: the document fits, with no room for two links.
    let near = "\"".repeat(32_700);
    let card = board
        .edit_at(card.id, &human, None, &prompt(&near))
        .expect("fits");
    assert!(matches!(
        board.plan(card.id, &human, &plan(Some(5), Some(vec![a.id, b.id]))),
        Err(BoardError::Invalid(_))
    ));
    let read = board.get(card.id).expect("get");
    assert_eq!(
        (read.rev, read.priority, read.deps.len()),
        (card.rev, card.priority, 0),
        "neither the priority nor the links are written"
    );
    let doc = tick.doc("board", card.id).expect("doc");
    assert_eq!(doc["prompt"].as_str().map(str::len), Some(near.len()));
}

#[test]
fn a_prompt_edit_whose_document_write_fails_is_partly_saved_and_finished_on_the_new_revision() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    tick.fail_next("PUT", "/docs/board/");
    let failed = board.edit_at(
        card.id,
        &human,
        Some(card.rev),
        &title_and_prompt("b", "do it"),
    );
    let half = board.get(card.id).expect("get");
    assert!(
        matches!(&failed, Err(BoardError::PartlySaved { rev, .. }) if *rev == half.rev),
        "not reported as a refused edit: {failed:?}"
    );
    assert_eq!((half.title.as_str(), half.prompt.as_str()), ("b", ""));
    assert_eq!(
        board.edit_at(
            card.id,
            &human,
            Some(card.rev),
            &title_and_prompt("b", "do it")
        ),
        Err(BoardError::Conflict),
        "the revision the editor started from is gone"
    );
    let done = board
        .edit_at(
            card.id,
            &human,
            Some(half.rev),
            &title_and_prompt("b", "do it"),
        )
        .expect("the same edit on the saved revision");
    assert_eq!((done.title.as_str(), done.prompt.as_str()), ("b", "do it"));
}

#[test]
fn a_writer_between_the_task_and_document_writes_leaves_the_edit_partly_saved() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let raced = json!({ "v": 1, "deps": [], "prompt": "theirs" });
    tick.race_after("PATCH", "/tasks/", 1, card.id, raced);
    let failed = board.edit_at(
        card.id,
        &human,
        Some(card.rev),
        &title_and_prompt("b", "mine"),
    );
    let now = board.get(card.id).expect("get");
    assert!(
        matches!(&failed, Err(BoardError::PartlySaved { rev, .. }) if *rev + 1 == now.rev),
        "the revision is the one the task write left, before the other writer: {failed:?}"
    );
    assert_eq!(
        (now.title.as_str(), now.prompt.as_str()),
        ("b", "theirs"),
        "the other writer's document is not overwritten"
    );
}

#[test]
fn a_document_only_write_that_fails_is_an_ordinary_error() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    tick.fail_next("PUT", "/docs/board/");
    let failed = board.edit_at(card.id, &human, Some(card.rev), &prompt("do it"));
    assert!(matches!(failed, Err(BoardError::Storage(_))), "{failed:?}");
    assert_eq!(
        board.get(card.id).expect("get").rev,
        card.rev,
        "nothing was saved"
    );
}

#[test]
fn a_milestone_is_a_dated_task_tagged_milestone_and_never_a_card() {
    let (board, tick) = board();
    let board = board.with_milestones(Box::new(TickStore::new(tick.clone())));
    let human = board.roles().actor("nano", "human").expect("role");
    let beta = board
        .add_milestone(&human, "orch", "Beta", 1_800_000_000_000)
        .expect("add");
    let task = tick.task(beta.id);
    assert_eq!(
        (&task["tags"], &task["dueMs"], &task["status"]),
        (
            &json!(["milestone"]),
            &json!(1_800_000_000_000_i64),
            &json!("open")
        )
    );
    assert!(board.list(None).expect("list").is_empty(), "not a card");
    let listed = board.milestones(|_| true).expect("milestones");
    assert_eq!(listed, vec![beta.clone()]);

    let close = MilestoneEdit {
        done: Some(true),
        target_ms: Some(1_900_000_000_000),
        ..MilestoneEdit::default()
    };
    board
        .edit_milestone(&human, beta.id, &close, |_| true)
        .expect("close");
    let read = TickStore::new(tick.clone())
        .milestone(beta.id)
        .expect("read")
        .expect("there");
    assert_eq!(read.target_ms, 1_900_000_000_000);
    assert!(read.done_ms.is_some(), "closed in rusty_tick");

    // A card that happens to carry the tag is still a card, not a milestone.
    let card = board
        .add(&human, "tagged", "", vec!["milestone".into()])
        .expect("add");
    let ids: Vec<_> = board
        .milestones(|_| true)
        .expect("milestones")
        .iter()
        .map(|m| m.id)
        .collect();
    assert!(!ids.contains(&card.id));
}

#[test]
fn a_milestone_save_is_fenced_on_the_version_its_scope_was_checked_at() {
    let (board, tick) = board();
    let store = TickStore::new(tick.clone());
    let board = board.with_milestones(Box::new(TickStore::new(tick.clone())));
    let human = board.roles().actor("nano", "human").expect("role");
    let beta = board
        .add_milestone(&human, "orch", "Beta", 1_800_000_000_000)
        .expect("add");
    let read = store.milestone(beta.id).expect("read").expect("there");
    // The owner moves it to another list in rusty_tick after the read.
    tick.owner_edit(beta.id, |t| t["listId"] = json!(uuid::Uuid::now_v7()));
    let everything = MilestoneEdit {
        title: Some("Renamed".into()),
        target_ms: Some(1_900_000_000_000),
        done: Some(true),
    };
    let next = read.edited(&everything, 1).expect("edit");
    assert!(matches!(
        store.save_milestone(&read, &next),
        Err(BoardError::Conflict)
    ));
    let task = tick.task(beta.id);
    assert_eq!(
        (&task["title"], &task["dueMs"], &task["status"]),
        (
            &json!("Beta"),
            &json!(1_800_000_000_000_i64),
            &json!("open")
        ),
        "title, date and completion are all unchanged"
    );
    // A fresh read sees the move, so the scope check runs on where it is now.
    let again = store.milestone(beta.id).expect("read").expect("there");
    assert_eq!(
        again.project, "default",
        "an unknown list reads as the Inbox"
    );
    assert!(again.rev > read.rev);
}

#[test]
fn a_restarted_boxed_board_lists_moves_from_tick_history_not_its_fresh_event_log() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let moved = board.add(&human, "moved", "", vec![]).expect("add");
    let quiet = board.add(&human, "quiet", "", vec![]).expect("add");
    tick.record_move(moved.id, 10);
    // A restart: a new board over a boxed store, as the server runs, with an empty event log.
    let fresh = Board::with_clock(
        Box::new(TickStore::new(tick.clone())) as Box<dyn Store>,
        Roles::default(),
        clock,
    );
    let human = fresh.roles().actor("nano", "human").expect("role");
    fresh
        .comment(quiet.id, &human, "just a note")
        .expect("comment");
    let mut cards = fresh.list(None).expect("list");
    let dates = Dates {
        moved: Span {
            from: Some(0),
            to: None,
        },
        ..Dates::default()
    };
    fresh.retain_dates(&mut cards, &dates).expect("dates");
    let titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        ["moved"],
        "the recorded move counts; a comment after the restart is not a move"
    );
}

#[test]
fn bad_storage_pages_fail_instead_of_returning_partial_cards() {
    struct BadPages {
        tick: FakeTick,
        mode: u8,
    }
    impl TickApi for BadPages {
        fn send(&self, r: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
            if r.path.starts_with("/tags/agent/tasks?") {
                // Ignore after here to model a broken server repeating the first page.
                let (status, mut body) = self.tick.send(&TickRequest {
                    method: "GET",
                    path: "/tags/agent/tasks".into(),
                    ..Default::default()
                })?;
                body["next"] = match self.mode {
                    0 => json!("bad-cursor"),
                    _ => body["tasks"][0]["id"].clone(),
                };
                if self.mode == 2 {
                    body["tasks"] = json!([]);
                }
                return Ok((status, body));
            }
            self.tick.send(r)
        }
    }
    let (board, tick) = board();
    let actor = board.roles().actor("human", "human").expect("role");
    let card = board
        .add(&actor, "complete record", "", vec![])
        .expect("add");
    for mode in 0..3 {
        let store = TickStore::new(BadPages {
            tick: tick.clone(),
            mode,
        });
        assert!(matches!(store.list(), Err(BoardError::Storage(_))));
        assert_eq!(
            store.get(card.id).expect("get").expect("card").title,
            card.title
        );
    }
    assert_eq!(board.list(None).expect("recovery").len(), 1);
}

/// Records the path of every request it passes on.
#[derive(Clone, Default)]
struct Recording {
    inner: FakeTick,
    paths: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Recording {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.paths.lock().expect("paths"))
    }
}

impl TickApi for Recording {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        self.paths
            .lock()
            .expect("paths")
            .push(format!("{} {}", request.method, request.path));
        self.inner.send(request)
    }
}

#[test]
fn a_card_named_by_number_is_read_without_the_rest_of_the_board() {
    let tick = Recording::default();
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock);
    let human = board.roles().actor("nano", "human").expect("role");
    let cards: Vec<Card> = (0..40)
        .map(|i| {
            board
                .add(&human, &format!("card {i}"), "", vec![])
                .expect("add")
        })
        .collect();
    let wanted = &cards[17];
    let number = wanted.number.expect("numbered");
    tick.take();

    assert_eq!(board.get_number(number).expect("by number").id, wanted.id);
    let paths = tick.take();
    let whole = ["/tags/agent/tasks", "/docs/board?", "/docs/comment?limit"];
    assert!(
        !paths.iter().any(|p| whole.iter().any(|w| p.contains(w))),
        "a numbered read touched a whole collection: {paths:?}"
    );
    assert!(paths.len() <= 8, "{} requests: {paths:?}", paths.len());

    // A number nobody has, and one a card lost when it left the board.
    assert!(board.get_number(number + 1000).is_err());
    tick.inner.owner_edit(wanted.id, |t| {
        t["tags"] = json!(["board:ready", format!("number:{number}")])
    });
    assert!(board.get_number(number).is_err());
}

#[test]
fn a_number_edited_onto_two_cards_by_hand_is_refused_not_guessed() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add(&human, "b", "", vec![]).expect("add");
    let n = a.number.expect("numbered");
    tick.owner_edit(b.id, |t| {
        t["tags"] = json!(["agent", "board:backlog", format!("number:{n}")])
    });
    assert!(board.get_number(n).is_err());
}

/// A summary is the full card's task fields: the cheap read loses nothing a summary shows.
#[test]
fn summaries_match_the_full_read_on_every_summary_field() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let first = board
        .add(&human, "first", "body", vec!["x".into()])
        .expect("add");
    let second = board.add(&human, "second", "", vec![]).expect("add");
    board
        .plan(second.id, &human, &plan(Some(3), Some(vec![first.id])))
        .expect("plan");
    board.comment(second.id, &human, "note").expect("comment");
    board
        .edit(first.id, &human, Some("renamed"), None, None)
        .expect("edit");
    board.assign(second.id, "orch", &human).expect("assign");
    board
        .move_card(first.id, &Column::READY, &human)
        .expect("move");

    let store = TickStore::new(tick);
    let summarise =
        |cards: Vec<Card>| -> Vec<CardSummary> { cards.iter().map(CardSummary::from).collect() };
    let full = summarise(store.list().expect("list"));
    assert_eq!(full.len(), 2);
    assert_eq!(summarise(store.summaries().expect("summaries")), full);
}

/// A clock that moves on 1 ms per reading, so cards list in the order they were made.
fn ticking() -> i64 {
    static NOW: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
    NOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The MCP surface over a rusty_tick store, on [`ticking`] time.
fn tick_shared(tick: FakeTick) -> rusty_mill_board::api::Shared {
    let store: Box<dyn Store> = Box::new(TickStore::new(tick));
    Board::with_clock(store, Roles::default(), ticking)
}

/// Run the MCP operation `name` as nano, a human.
fn op(
    board: &rusty_mill_board::api::Shared,
    name: &str,
    mut args: serde_json::Value,
) -> serde_json::Value {
    args["actor"] = json!("nano");
    args["role"] = json!("human");
    rusty_mill_board::api::ops()
        .into_iter()
        .find(|o| o.name == name)
        .expect("op")
        .call(board, None, args)
        .expect(name)
}

/// Every card of a `board_list` scan of `args` in pages of `limit`.
fn scan_all(
    board: &rusty_mill_board::api::Shared,
    args: &serde_json::Value,
    limit: u32,
) -> Vec<serde_json::Value> {
    let mut page = args.clone();
    page["limit"] = json!(limit);
    let mut seen = Vec::new();
    loop {
        let out = op(board, "board_list", page.clone());
        seen.extend(out["cards"].as_array().expect("cards").iter().cloned());
        match out["next_cursor"].as_str() {
            Some(c) => page["cursor"] = json!(c),
            None => return seen,
        }
    }
}

/// The largest records the board holds come through a paged scan whole, from rusty_tick.
#[test]
fn a_paged_scan_over_rusty_tick_never_truncates_large_records() {
    let board = tick_shared(FakeTick::default());
    let body = "b".repeat(28 * 1024);
    let prompt = "p".repeat(rusty_mill_board::domain::MAX_PROMPT_BYTES);
    let comment = "c".repeat(31 * 1024);
    op(&board, "card_add", json!({"title": "before"}));
    let big = op(&board, "card_add", json!({"title": "big", "body": body}))["card"]["id"].clone();
    op(&board, "card_update", json!({"id": big, "prompt": prompt}));
    op(&board, "card_comment", json!({"id": big, "text": comment}));
    op(&board, "card_add", json!({"title": "after"}));

    let all = op(&board, "board_list", json!({}))["cards"].clone();
    let seen = scan_all(&board, &json!({}), 1);
    assert_eq!(json!(seen), all);
    let card = seen.iter().find(|c| c["id"] == big).expect("big card");
    assert_eq!(card["body"].as_str().map(str::len), Some(body.len()));
    assert_eq!(card["prompt"].as_str().map(str::len), Some(prompt.len()));
    assert_eq!(
        card["comments"][0]["text"].as_str().map(str::len),
        Some(comment.len())
    );
}

/// A scan over rusty_tick with fenced writes between pages: an unchanged card comes once, an
/// edited one once with its edit if it was still ahead, and a card added mid-scan at the end.
#[test]
fn writes_during_a_paged_scan_over_rusty_tick_never_repeat_or_drop_a_card() {
    let board = tick_shared(FakeTick::default());
    let ids: Vec<serde_json::Value> = (1..=6)
        .map(|i| {
            op(&board, "card_add", json!({"title": format!("card {i}")}))["card"]["id"].clone()
        })
        .collect();
    let first = op(&board, "board_list", json!({"limit": 3}));
    let rev =
        |id: &serde_json::Value| op(&board, "card_get", json!({"id": id}))["card"]["rev"].clone();
    for (id, title) in [(&ids[0], "edited behind"), (&ids[4], "edited ahead")] {
        let rev = rev(id);
        op(
            &board,
            "card_update",
            json!({"id": id, "title": title, "rev": rev}),
        );
    }
    let added = op(&board, "card_add", json!({"title": "added"}))["card"]["id"].clone();

    let mut seen: Vec<serde_json::Value> = first["cards"].as_array().expect("cards").clone();
    let mut page = json!({"limit": 3, "cursor": first["next_cursor"]});
    loop {
        let out = op(&board, "board_list", page.clone());
        seen.extend(out["cards"].as_array().expect("cards").iter().cloned());
        match out["next_cursor"].as_str() {
            Some(c) => page["cursor"] = json!(c),
            None => break,
        }
    }
    let seen_ids: Vec<_> = seen.iter().map(|c| c["id"].clone()).collect();
    let mut expected = ids.clone();
    expected.push(added);
    assert_eq!(seen_ids, expected);
    assert_eq!(seen[0]["title"], "card 1", "returned before the edit");
    assert_eq!(seen[4]["title"], "edited ahead");
}

/// `card_take` over rusty_tick keeps take's order and skips (priority first, then the oldest;
/// a card waiting on an open dependency or superseded is passed over) while reading only the
/// queue columns' tags and the candidates it tries: never a whole collection.
#[test]
fn take_over_rusty_tick_keeps_its_order_and_reads_only_the_queues() {
    let tick = Recording::default();
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), ticking);
    let human = board.roles().actor("nano", "human").expect("role");
    let ids: Vec<uuid::Uuid> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|t| {
            let id = board.add(&human, t, "", vec![]).expect("add").id;
            board.move_card(id, &Column::READY, &human).expect("ready");
            id
        })
        .collect();
    let (a, b, c, d, e) = (ids[0], ids[1], ids[2], ids[3], ids[4]);
    board
        .plan(b, &human, &plan(Some(5), None))
        .expect("priority");
    board
        .plan(e, &human, &plan(None, Some(vec![a])))
        .expect("deps");
    let superseded = Plan {
        superseded_by: Some(vec![d]),
        ..Plan::default()
    };
    board.plan(c, &human, &superseded).expect("supersede");
    // A card in Backlog is never a candidate, however old.
    board.add(&human, "backlog", "", vec![]).expect("add");
    tick.take();

    let taken: Vec<Option<uuid::Uuid>> = ["w1", "w2", "w3", "w4"]
        .iter()
        .map(|w| {
            let dev = board.roles().actor(w, "builder").expect("role");
            board.take(&dev, None).expect("take").map(|c| c.id)
        })
        .collect();
    assert_eq!(taken, [Some(b), Some(a), Some(d), None]);

    let paths = tick.take();
    let whole = ["/tags/agent/tasks", "/docs/board?", "/docs/comment?limit"];
    assert!(
        !paths.iter().any(|p| whole.iter().any(|w| p.contains(w))),
        "a take touched a whole collection: {paths:?}"
    );
    // Named, a waiting card says why; once its dependency is Done it is taken.
    let dev = board.roles().actor("w5", "builder").expect("role");
    assert!(matches!(
        board.take(&dev, Some(e)),
        Err(BoardError::Refused(_))
    ));
    // The owner closes a in rusty_tick: Done.
    tick.inner.owner_edit(a, |t| t["status"] = json!("done"));
    assert_eq!(board.take(&dev, None).expect("take").map(|c| c.id), Some(e));
}

/// Hides every task from `/search`, as a delta read that lost its answer would.
#[derive(Clone)]
struct BlindSearch(FakeTick);

impl TickApi for BlindSearch {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        if request.path.starts_with("/search") {
            return Ok((200, json!({ "tasks": [] })));
        }
        self.0.send(request)
    }
}

fn kinds(events: &[Event]) -> Vec<(EventKind, String)> {
    events.iter().map(|e| (e.kind, e.actor.clone())).collect()
}

#[test]
fn a_comment_added_in_rusty_tick_shows_as_an_event_once() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    board
        .comment(card.id, &human, "from the board")
        .expect("comment");
    let cursor = board.cursor().expect("cursor");

    tick.owner_comment(card.id, "from the phone");
    let events = board.events(cursor, 10).expect("events");
    assert_eq!(kinds(&events), [(EventKind::Commented, "tick".to_owned())]);
    assert_eq!(events[0].card, card.id);
    let after = events.last().map(|e| e.id);
    assert!(
        board.events(after, 10).expect("events").is_empty(),
        "seen once"
    );
}

#[test]
fn comments_over_more_than_one_page_are_all_logged() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");

    for n in 0..1001 {
        tick.owner_comment(card.id, &format!("note {n}"));
    }
    let events = board.events(cursor, 2000).expect("events");
    let commented = events
        .iter()
        .filter(|e| e.kind == EventKind::Commented)
        .count();
    assert_eq!(commented, 1001, "every page of comments is read");
    let after = events.last().map(|e| e.id);
    assert!(
        board.events(after, 10).expect("next poll").is_empty(),
        "the window moves on"
    );
}

#[test]
fn an_edit_in_a_millisecond_already_read_is_not_missed() {
    let (board, tick) = board();
    let human = board.roles().actor("nano", "human").expect("role");
    let a = board.add(&human, "a", "", vec![]).expect("add");
    let b = board.add(&human, "b", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");
    tick.owner_edit(a.id, |t| t["title"] = json!("a2"));
    let first = board.events(cursor, 10).expect("events");
    assert_eq!(first.len(), 1);

    // b changes in the very millisecond the last read already covered.
    let at = tick.task(a.id)["updatedMs"].as_i64().expect("ms");
    tick.owner_edit_at(b.id, at, |t| t["title"] = json!("b2"));
    let next = board
        .events(first.last().map(|e| e.id), 10)
        .expect("events");
    assert_eq!(next.len(), 1);
    assert_eq!((next[0].card, next[0].kind), (b.id, EventKind::Edited));
}

#[test]
fn a_change_a_delta_read_missed_is_caught_by_the_next_full_read() {
    let tick = FakeTick::default();
    let store = TickStore::new(BlindSearch(tick.clone()))
        .with_reconcile_every(std::time::Duration::from_millis(200));
    let board = Board::with_clock(store, Roles::default(), clock);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");

    tick.owner_edit(card.id, |t| t["title"] = json!("renamed"));
    assert!(
        board.events(cursor, 10).expect("events").is_empty(),
        "the delta lost it"
    );
    std::thread::sleep(std::time::Duration::from_millis(250));
    let events = board.events(cursor, 10).expect("events");
    assert_eq!(kinds(&events), [(EventKind::Edited, "tick".to_owned())]);
}

/// Fails every task read while `down` is set, as an outage between two reads would.
#[derive(Clone)]
struct FlakyTaskRead(FakeTick, Arc<std::sync::atomic::AtomicBool>);

impl TickApi for FlakyTaskRead {
    fn send(&self, request: &TickRequest<'_>) -> Result<(u16, serde_json::Value), BoardError> {
        let down = self.1.load(std::sync::atomic::Ordering::SeqCst);
        if down && request.method == "GET" && request.path.starts_with("/tasks/") {
            return Err(BoardError::Storage("tick unreachable".into()));
        }
        self.0.send(request)
    }
}

#[test]
fn a_comment_whose_card_read_failed_is_logged_by_the_next_poll() {
    let tick = FakeTick::default();
    let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let store = TickStore::new(FlakyTaskRead(tick.clone(), down.clone()));
    let board = Board::with_clock(store, Roles::default(), clock);
    let human = board.roles().actor("nano", "human").expect("role");
    let card = board.add(&human, "a", "", vec![]).expect("add");
    let cursor = board.cursor().expect("cursor");

    tick.owner_comment(card.id, "from the phone");
    down.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(board.events(cursor, 10).is_err(), "the outage is reported");
    down.store(false, std::sync::atomic::Ordering::SeqCst);
    let events = board.events(cursor, 10).expect("events");
    assert_eq!(kinds(&events), [(EventKind::Commented, "tick".to_owned())]);
}

/// After the first read, an idle poll asks only what changed: no whole collection.
#[test]
fn an_idle_poll_reads_only_what_changed() {
    let tick = Recording::default();
    let board = Board::with_clock(TickStore::new(tick.clone()), Roles::default(), clock);
    let human = board.roles().actor("nano", "human").expect("role");
    for i in 0..20 {
        board
            .add(&human, &format!("card {i}"), "", vec![])
            .expect("add");
    }
    let cursor = board.cursor().expect("cursor");
    tick.take();
    assert!(board.events(cursor, 10).expect("events").is_empty());
    let paths = tick.take();
    assert!(
        paths
            .iter()
            .all(|p| p.starts_with("GET /search?updatedFrom=")
                || p.starts_with("GET /docs/comment?after=")),
        "{paths:?}"
    );
}

/// A cursor from before a restart is answered with an explicit resync, never silence.
#[test]
fn a_cursor_from_before_a_restart_gets_a_resync() {
    let tick = FakeTick::default();
    let before = tick_shared(tick.clone());
    op(&before, "card_add", json!({"title": "a"}));
    let old = op(&before, "board_events", json!({}));
    assert_eq!(old["resync"], false);
    let cursor = old["cursor"].clone();
    drop(before);

    let after = tick_shared(tick.clone());
    let a = op(&after, "card_add", json!({"title": "b"}))["card"]["id"].clone();
    let polled = op(&after, "board_events", json!({ "after": cursor }));
    assert_eq!(polled["resync"], true);
    // The new log is read from its start, so the change after the restart is there too.
    let events = polled["events"].as_array().expect("events");
    assert!(events.iter().any(|e| e["card"] == a), "{events:?}");
    let next = op(&after, "board_events", json!({ "after": polled["cursor"] }));
    assert_eq!(next["resync"], false);
}
