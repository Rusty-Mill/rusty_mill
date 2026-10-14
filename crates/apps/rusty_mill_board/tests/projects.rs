//! Projects: one board, several applications. A credential limited to some projects can
//! neither see nor change cards of the others, and `take` never offers them.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{
    Actor, BoardError, Card, Column, DEFAULT_PROJECT, Event, EventKind,
};
use rusty_mill_board::projects::Projects;
use rusty_mill_board::store::{MemStore, Store};
use uuid::Uuid;

fn projects() -> Projects {
    Projects::parse(r#"{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"]}"#)
        .expect("projects")
}

fn board() -> Board<MemStore> {
    Board::new(MemStore::default(), Roles::default()).with_projects(projects())
}

fn actor(b: &Board<impl Store>, name: &str, role: &str, scope: Option<&[&str]>) -> Actor {
    let mut a = b.roles().actor(name, role).expect("role");
    a.projects = scope.map(|s| s.iter().map(|p| (*p).to_owned()).collect::<BTreeSet<_>>());
    a
}

/// A card in `project`, put in Ready by an unrestricted human.
fn ready_in(b: &Board<impl Store>, project: &str, title: &str) -> Uuid {
    let human = actor(b, "nano", "human", None);
    let c = b.add_in(&human, project, title, "", vec![]).expect("add");
    b.move_card(c.id, Column::Ready, &human).expect("ready");
    c.id
}

#[test]
fn cards_default_to_the_default_project_and_unknown_projects_are_refused() {
    let b = board();
    let human = actor(&b, "nano", "human", None);
    assert_eq!(
        b.add(&human, "a", "", vec![]).expect("add").project,
        DEFAULT_PROJECT
    );
    assert_eq!(
        b.add_in(&human, "rl", "b", "", vec![])
            .expect("add")
            .project,
        "rl"
    );
    assert_eq!(
        b.add_in(&human, "nope", "c", "", vec![]),
        Err(BoardError::Invalid("unknown project"))
    );
}

#[test]
fn a_card_stored_before_projects_existed_belongs_to_default() {
    let card = Card::new("old", "", vec![], 1).expect("card");
    let mut json = serde_json::to_value(&card).expect("json");
    json.as_object_mut().expect("object").remove("project");
    let read: Card = serde_json::from_value(json).expect("reads without the field");
    assert_eq!(read.project, DEFAULT_PROJECT);
}

#[test]
fn a_scoped_actor_cannot_change_cards_of_other_projects() {
    let b = board();
    let rl_card = ready_in(&b, "rl", "rl work");
    let orch_card = ready_in(&b, "orch", "orch work");
    let dev = actor(&b, "claude", "builder", Some(&["rl"]));
    let human = actor(&b, "nano", "human", None);

    assert_eq!(
        b.take(&dev, Some(orch_card)),
        Err(BoardError::OutOfScope("orch".into()))
    );
    assert_eq!(
        b.claim(orch_card, &dev),
        Err(BoardError::OutOfScope("orch".into()))
    );
    assert_eq!(
        b.comment(orch_card, &dev, "hi"),
        Err(BoardError::OutOfScope("orch".into()))
    );
    assert_eq!(
        b.move_card(orch_card, Column::Building, &dev),
        Err(BoardError::OutOfScope("orch".into()))
    );
    assert_eq!(
        b.add_in(&dev, "orch", "sneaky", "", vec![]),
        Err(BoardError::OutOfScope("orch".into()))
    );
    // Nothing happened to it, and the card in scope works as usual.
    let untouched = b.get(orch_card).expect("card");
    assert_eq!(
        (untouched.column, untouched.claim, untouched.comments.len()),
        (Column::Ready, None, 0)
    );
    assert_eq!(
        b.take(&dev, Some(rl_card))
            .expect("take")
            .map(|c| c.project),
        Some("rl".into())
    );
    // An unrestricted actor is not limited.
    assert!(b.comment(orch_card, &human, "ok").is_ok());
}

#[test]
fn take_never_offers_other_projects_even_when_theirs_is_nearer_done() {
    let b = board();
    let human = actor(&b, "nano", "human", None);
    let mine = ready_in(&b, "rl", "mine");
    let theirs = ready_in(&b, "orch", "theirs");
    // `theirs` is further along (Building, claimed by a colleague then released), so it ranks first.
    let colleague = actor(&b, "other", "builder", None);
    b.take(&colleague, Some(theirs)).expect("take");
    b.release(theirs, &colleague).expect("release");

    let dev = actor(&b, "claude", "builder", Some(&["rl"]));
    assert_eq!(b.take(&dev, None).expect("take").map(|c| c.id), Some(mine));
    assert_eq!(
        b.take(&dev, None).expect("take").map(|c| c.id),
        Some(mine),
        "own claim resumed"
    );
    let unrestricted = actor(&b, "free", "builder", None);
    assert_eq!(
        b.take(&unrestricted, None).expect("take").map(|c| c.id),
        Some(theirs),
        "without a scope, the nearest-Done card still wins"
    );
    let _ = human;
}

#[test]
fn take_can_be_limited_to_one_project() {
    let b = board();
    let rl = ready_in(&b, "rl", "rl");
    ready_in(&b, "orch", "orch");
    let free = actor(&b, "free", "builder", None);
    assert_eq!(
        b.take_in(&free, None, Some("rl"))
            .expect("take")
            .map(|c| c.id),
        Some(rl)
    );
    assert_eq!(
        b.take_in(&free, None, Some("nope")),
        Err(BoardError::Invalid("unknown project"))
    );
    let scoped = actor(&b, "claude", "builder", Some(&["rl"]));
    assert_eq!(
        b.take_in(&scoped, None, Some("orch")),
        Err(BoardError::OutOfScope("orch".into()))
    );
}

#[test]
fn assign_needs_triage_and_clears_the_claim() {
    let b = board();
    let id = ready_in(&b, "rl", "work");
    let dev = actor(&b, "claude", "builder", None);
    b.take(&dev, Some(id)).expect("take");
    assert_eq!(
        b.assign(id, "orch", &dev),
        Err(BoardError::Forbidden(
            rusty_mill_board::domain::Capability::Triage
        ))
    );
    let human = actor(&b, "nano", "human", None);
    assert_eq!(
        b.assign(id, "nope", &human),
        Err(BoardError::Invalid("unknown project"))
    );
    let moved = b.assign(id, "orch", &human).expect("assign");
    assert_eq!((moved.project.as_str(), moved.claim), ("orch", None));

    // A triager limited to rl can neither take cards out of rl nor push cards into orch.
    let rl_triager = actor(&b, "lead", "human", Some(&["rl"]));
    assert_eq!(
        b.assign(id, "rl", &rl_triager),
        Err(BoardError::OutOfScope("orch".into()))
    );
    let other = ready_in(&b, "rl", "other");
    assert_eq!(
        b.assign(other, "orch", &rl_triager),
        Err(BoardError::OutOfScope("orch".into()))
    );
    assert_eq!(b.get(other).expect("card").project, "rl");
}

#[test]
fn a_done_card_cannot_change_project() {
    let b = board();
    let human = actor(&b, "nano", "human", None);
    let id = ready_in(&b, "rl", "x");
    for to in [Column::Building, Column::Qa, Column::Review, Column::Done] {
        b.move_card(id, to, &human).expect("move");
    }
    assert_eq!(b.assign(id, "orch", &human), Err(BoardError::Terminal));
}

/// A store that moves the first card it is asked for to another project right after
/// returning it: the race between a caller's read and its write.
struct ReassignAfterGet {
    inner: MemStore,
    to: String,
    armed: Arc<AtomicBool>,
}

impl Store for ReassignAfterGet {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        let seen = self.inner.get(id)?;
        if self.armed.swap(false, Ordering::SeqCst) {
            let moved = seen.as_ref().expect("card").assign(&self.to, 1)?;
            self.inner
                .replace(&moved, Event::of(&moved, EventKind::Assigned, "nano"))?;
        }
        Ok(seen)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, c: &Card, e: Event) -> Result<(), BoardError> {
        self.inner.insert(c, e)
    }
    fn replace(&self, c: &Card, e: Event) -> Result<(), BoardError> {
        self.inner.replace(c, e)
    }
    fn events_since(&self, a: Option<Uuid>, l: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(a, l)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
}

#[test]
fn a_card_reassigned_after_the_read_is_refused_not_half_taken() {
    let armed = Arc::new(AtomicBool::new(false));
    let store = ReassignAfterGet {
        inner: MemStore::default(),
        to: "orch".into(),
        armed: Arc::clone(&armed),
    };
    let b = Board::new(store, Roles::default()).with_projects(projects());
    let id = ready_in(&b, "rl", "racy");
    let dev = actor(&b, "claude", "builder", Some(&["rl"]));
    armed.store(true, Ordering::SeqCst);
    // `take` read the card in rl; by the time it writes, the card belongs to orch.
    assert_eq!(
        b.take(&dev, Some(id)),
        Err(BoardError::OutOfScope("orch".into()))
    );
    let now = b.get(id).expect("card");
    assert_eq!(
        (now.project.as_str(), now.column, now.claim),
        ("orch", Column::Ready, None)
    );
}
