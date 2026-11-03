//! Projects: one board, several applications. A credential limited to some projects can
//! neither see nor change cards of the others, and `take` never offers them.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{
    Actor, BoardError, Capability, Card, Column, DEFAULT_PROJECT, Event, EventKind,
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
    b.move_card(c.id, &Column::READY, &human).expect("ready");
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
        b.move_card(orch_card, &Column::BUILDING, &dev),
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
        (Column::READY, None, 0)
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
        b.assign(id, "orch", &actor(&b, "q", "qa", None)),
        Err(BoardError::Forbidden(Capability::Project))
    );
    assert_eq!(
        b.assign(id, "orch", &actor(&b, "gemini", "builder", None)),
        Err(BoardError::Claimed("claude".into())),
        "a builder does not take another's claimed card away"
    );
    let own = b
        .assign(id, "orch", &dev)
        .expect("a builder moves its own card");
    assert_eq!((own.project.as_str(), own.claim), ("orch", None));
    let unclaimed = ready_in(&b, "rl", "free");
    b.assign(unclaimed, "orch", &actor(&b, "gemini", "builder", None))
        .expect("or an unclaimed one");
    b.assign(id, "rl", &actor(&b, "nano", "human", None))
        .expect("back");
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

/// A Done card in `rl`, moved there by an unrestricted human.
fn done_in_rl(b: &Board<impl Store>) -> Uuid {
    let human = actor(b, "nano", "human", None);
    let id = ready_in(b, "rl", "x");
    for to in [Column::BUILDING, Column::QA, Column::REVIEW, Column::DONE] {
        b.move_card(id, &to, &human).expect("move");
    }
    id
}

#[test]
fn a_done_card_changes_project_and_stays_done() {
    let b = board();
    let id = done_in_rl(&b);
    let builder = actor(&b, "claude", "builder", None);
    let moved = b.assign(id, "orch", &builder).expect("re-file a Done card");
    assert_eq!(
        (moved.project.as_str(), moved.column),
        ("orch", Column::DONE)
    );
    let human = actor(&b, "nano", "human", None);
    assert_eq!(
        b.move_card(id, &Column::READY, &human),
        Err(BoardError::Terminal)
    );
    let events = b.events(None, 1000).expect("events");
    let last = events.last().expect("an event");
    assert_eq!((last.kind, last.card), (EventKind::Assigned, id));
}

#[test]
fn re_filing_a_done_card_needs_close_or_triage() {
    let b = board();
    let id = done_in_rl(&b);
    let filer = Actor {
        name: "filer".into(),
        caps: BTreeSet::from([Capability::Project]),
        projects: None,
    };
    assert_eq!(
        b.assign(id, "orch", &filer),
        Err(BoardError::Forbidden(Capability::Close))
    );
    assert_eq!(b.get(id).expect("card").project, "rl");
    b.assign(id, "orch", &actor(&b, "nano", "human", None))
        .expect("triage re-files it");
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
            let moved = seen.as_ref().expect("card").assign(&self.to, 1);
            self.inner
                .replace(&moved, Event::of(&moved, EventKind::Assigned, "nano"))?;
        }
        Ok(seen)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, c: &Card, e: Event) -> Result<u64, BoardError> {
        self.inner.insert(c, e)
    }
    fn replace(&self, c: &Card, e: Event) -> Result<u64, BoardError> {
        self.inner.replace(c, e)
    }
    fn events_since(&self, a: Option<Uuid>, l: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(a, l)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        self.inner.reserve_number(card, floor)
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
        ("orch", Column::READY, None)
    );
}

// ---- the registry and the cards that depend on it change under one lock

/// A store whose writes pause once armed: it says it has reached the write, then waits to be released.
struct Gated {
    inner: MemStore,
    armed: Arc<AtomicBool>,
    reached: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Gated {
    fn pause(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.reached.lock().expect("lock").send(()).expect("send");
            self.release.lock().expect("lock").recv().expect("release");
        }
    }
}

impl Store for Gated {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        self.inner.get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        self.inner.list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.pause();
        self.inner.insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.pause();
        self.inner.replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.inner.events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        self.inner.last_event_id()
    }
    fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
        self.inner.reserve_number(card, floor)
    }
}

/// A board over a gated store; `go` lets the paused write continue.
/// The `armed` flag pauses the next write; `at_write` says it has paused.
fn gated_board() -> (
    Arc<Board<Gated>>,
    Arc<AtomicBool>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let armed = Arc::new(AtomicBool::new(false));
    let (reached, at_write) = mpsc::channel();
    let (go, release) = mpsc::channel();
    let store = Gated {
        inner: MemStore::default(),
        armed: armed.clone(),
        reached: Mutex::new(reached),
        release: Mutex::new(release),
    };
    let board = Board::new(store, Roles::default()).with_projects(projects());
    (Arc::new(board), armed, at_write, go)
}

fn without_rl() -> Projects {
    Projects::parse(r#"{"orch": ["crates/apps/rusty_orch"]}"#).expect("projects")
}

const SOON: Duration = Duration::from_millis(200);

#[test]
fn a_project_cannot_be_removed_while_a_card_is_being_added_to_it() {
    let (b, armed, at_write, go) = gated_board();
    let human = actor(&b, "nano", "human", None);
    armed.store(true, Ordering::SeqCst);
    let adding = {
        let (b, human) = (b.clone(), human.clone());
        thread::spawn(move || b.add_in(&human, "rl", "late", "", vec![]))
    };
    at_write.recv().expect("the add reached its write");

    let (done, finished) = mpsc::channel();
    let removing = {
        let b = b.clone();
        thread::spawn(move || {
            let r = b.set_projects(without_rl(), |_| Ok(()));
            done.send(()).expect("send");
            r
        })
    };
    assert!(
        finished.recv_timeout(SOON).is_err(),
        "the removal waits for the add in flight"
    );
    go.send(()).expect("go");
    assert!(adding.join().expect("add").is_ok());
    let removed = removing.join().expect("remove");
    assert!(matches!(removed, Err(BoardError::Refused(m)) if m.contains("still has cards")));
    assert!(
        b.projects().contains("rl"),
        "the project and its card stay together"
    );
    assert_eq!(b.list(None).expect("list").len(), 1);
}

#[test]
fn a_project_cannot_be_removed_while_a_card_is_being_assigned_to_it() {
    let (b, armed, at_write, go) = gated_board();
    let human = actor(&b, "nano", "human", None);
    let card = b.add(&human, "x", "", vec![]).expect("add").id;
    armed.store(true, Ordering::SeqCst);
    let assigning = {
        let (b, human) = (b.clone(), human.clone());
        thread::spawn(move || b.assign(card, "rl", &human))
    };
    at_write.recv().expect("the assign reached its write");

    let (done, finished) = mpsc::channel();
    let removing = {
        let b = b.clone();
        thread::spawn(move || {
            let r = b.set_projects(without_rl(), |_| Ok(()));
            done.send(()).expect("send");
            r
        })
    };
    assert!(finished.recv_timeout(SOON).is_err(), "the removal waits");
    go.send(()).expect("go");
    assert!(assigning.join().expect("assign").is_ok());
    assert!(removing.join().expect("remove").is_err());
    assert_eq!(b.get(card).expect("card").project, "rl");
    assert!(b.projects().contains("rl"));
}

#[test]
fn a_failed_save_of_a_new_project_leaves_no_card_in_it_and_changes_nothing() {
    let (b, ..) = gated_board();
    let human = actor(&b, "nano", "human", None);
    let with_new = Projects::parse(
        r#"{"rl": ["crates/apps/rocket_league"], "orch": ["crates/apps/rusty_orch"], "fresh": ["x"]}"#,
    )
    .expect("projects");

    let (saving, at_save) = mpsc::channel();
    let (fail, failing) = mpsc::channel::<()>();
    let setting = {
        let b = b.clone();
        thread::spawn(move || {
            b.set_projects(with_new, |_| {
                saving.send(()).expect("send");
                failing.recv().expect("fail");
                Err("disk full".into())
            })
        })
    };
    at_save.recv().expect("the registry is being saved");

    let (done, finished) = mpsc::channel();
    let adding = {
        let (b, human) = (b.clone(), human.clone());
        thread::spawn(move || {
            let r = b.add_in(&human, "fresh", "early", "", vec![]);
            done.send(()).expect("send");
            r
        })
    };
    assert!(
        finished.recv_timeout(SOON).is_err(),
        "a project that is not durable yet takes no cards"
    );
    fail.send(()).expect("fail");
    assert_eq!(
        setting.join().expect("set"),
        Err(BoardError::Refused("disk full".into()))
    );
    assert_eq!(
        adding.join().expect("add").map(|c| c.title),
        Err(BoardError::Invalid("unknown project"))
    );
    assert!(
        !b.projects().contains("fresh"),
        "live registry is the old one"
    );
    assert!(b.list(None).expect("list").is_empty(), "no orphan card");
}

#[test]
fn a_builder_adds_a_project_but_never_changes_an_existing_one() {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&saved);
    let b = board().with_projects_sink(Box::new(move |p: &Projects| {
        sink.lock().expect("lock").push(p.file_json()?);
        Ok(())
    }));
    let dev = actor(&b, "claude", "builder", None);
    let paths = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

    let added = b
        .add_project(
            &dev,
            "rusty_mill",
            "term",
            &paths(&["crates/libs/ui/rusty_term"]),
        )
        .expect("add");
    assert_eq!(added.paths("term"), ["crates/libs/ui/rusty_term"]);
    assert!(
        b.projects().contains("rusty_mill"),
        "a new repository has its own project"
    );
    assert_eq!(saved.lock().expect("lock").len(), 1);
    b.add_in(&dev, "term", "a", "", vec![])
        .expect("cards can go there");

    for (repo, name, p, why) in [
        ("default", "rl", "crates/x", "already exists"),
        ("default", "rl2", "crates/apps/rocket_league", "overlaps"),
        (
            "default",
            "rl3",
            "crates/apps/rocket_league/sub",
            "overlaps",
        ),
        ("default", "apps", "crates/apps", "overlaps"),
        ("default", "up", "../escape", "inside the repository"),
    ] {
        match b.add_project(&dev, repo, name, &paths(&[p])) {
            Err(BoardError::Refused(m)) => assert!(m.contains(why), "{name}: {m}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(
        saved.lock().expect("lock").len(),
        1,
        "a refusal saves nothing"
    );

    let qa = actor(&b, "q", "qa", None);
    assert_eq!(
        b.add_project(&qa, "default", "x", &paths(&["x"]))
            .map(|_| ()),
        Err(BoardError::Forbidden(Capability::Project))
    );
    let scoped = actor(&b, "s", "builder", Some(&["rl"]));
    b.add_project(&scoped, "default", "y", &paths(&["y"]))
        .expect("a scoped builder may add one");
    assert!(
        matches!(
            b.add_in(&scoped, "y", "a", "", vec![]),
            Err(BoardError::OutOfScope(_))
        ),
        "but adding it does not widen its scope"
    );
}

#[test]
fn a_board_without_a_projects_file_refuses_a_new_project() {
    let b = board();
    let dev = actor(&b, "claude", "builder", None);
    assert!(matches!(
        b.add_project(&dev, "default", "x", &["x".to_owned()]),
        Err(BoardError::Refused(m)) if m.contains("no projects file")
    ));
}

#[test]
fn a_card_in_an_unregistered_project_does_not_block_an_unrelated_change() {
    let store = MemStore::default();
    let mut stray = Card::new("stray", "", vec![], 0).expect("card");
    "gone".clone_into(&mut stray.project);
    store
        .insert(&stray, Event::of(&stray, EventKind::Created, "x"))
        .expect("insert");
    let b = Board::new(store, Roles::default()).with_projects(projects());
    let more = projects()
        .with_project("default", "extra", &["extra".to_owned()])
        .expect("more");
    b.set_projects(more, |_| Ok(()))
        .expect("additive change is not blocked");
}

/// A persist step that signals `reached` and waits for `go` before it returns.
fn gate() -> (
    impl Fn(&Projects) -> Result<(), String> + Send + Sync + 'static,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (reached, at_persist) = mpsc::channel();
    let (go, release) = mpsc::channel::<()>();
    let (reached, release) = (Mutex::new(reached), Mutex::new(release));
    let persist = move |_: &Projects| {
        reached.lock().expect("lock").send(()).expect("send");
        release.lock().expect("lock").recv().expect("go");
        Ok(())
    };
    (persist, at_persist, go)
}

fn one(path: &str) -> Vec<String> {
    vec![path.to_owned()]
}

#[test]
fn concurrent_project_adds_both_survive() {
    let (persist, at_persist, go) = gate();
    let first = Arc::new(AtomicBool::new(true));
    let b = Arc::new(board().with_projects_sink(Box::new(move |p: &Projects| {
        if first.swap(false, Ordering::SeqCst) {
            persist(p)?;
        }
        Ok(())
    })));
    let dev = actor(&b, "claude", "builder", None);
    let a = {
        let (b, dev) = (b.clone(), dev.clone());
        thread::spawn(move || b.add_project(&dev, "default", "a", &one("a")).map(|_| ()))
    };
    at_persist.recv().expect("the first add is persisting");
    let second = {
        let (b, dev) = (b.clone(), dev.clone());
        thread::spawn(move || b.add_project(&dev, "default", "c", &one("c")).map(|_| ()))
    };
    thread::sleep(SOON);
    go.send(()).expect("go");
    a.join().expect("join").expect("first add");
    second.join().expect("join").expect("second add");
    let projects = b.projects();
    assert!(
        projects.contains("a") && projects.contains("c"),
        "{projects:?}"
    );
}

#[test]
fn a_project_add_keeps_an_administrators_concurrent_change() {
    let (persist, at_persist, go) = gate();
    let b = Arc::new(board().with_projects_sink(Box::new(|_: &Projects| Ok(()))));
    let admin = {
        let b = b.clone();
        let next = projects()
            .with_project("default", "admins", &one("admins"))
            .expect("next");
        thread::spawn(move || b.set_projects(next, persist))
    };
    at_persist.recv().expect("the admin change is persisting");
    let dev = actor(&b, "claude", "builder", None);
    let add = {
        let b = b.clone();
        thread::spawn(move || {
            b.add_project(&dev, "default", "mine", &one("mine"))
                .map(|_| ())
        })
    };
    thread::sleep(SOON);
    go.send(()).expect("go");
    admin.join().expect("join").expect("admin change");
    add.join().expect("join").expect("add");
    let projects = b.projects();
    assert!(
        projects.contains("admins") && projects.contains("mine"),
        "{projects:?}"
    );
}
