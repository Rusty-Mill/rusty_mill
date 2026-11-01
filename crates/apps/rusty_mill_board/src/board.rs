//! The service: resolves actors from roles, applies domain operations and
//! commits them through a [`Store`], retrying on revision conflicts.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError, RwLock, RwLockReadGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{
    Actor, BoardError, Capability, Card, Column, DEFAULT_PROJECT, Event, EventKind, ExecutionLink,
    MoveRules, import_id, text_hash,
};
use crate::metrics::{Metrics, metrics};
use crate::projects::Projects;
use crate::roadmap::{Roadmap, roadmap};
use crate::store::Store;
use crate::workflow::Workflow;

/// Attempts per operation when another writer wins the revision race.
const RETRIES: usize = 3;

/// The actor named on the events that number cards.
const NUMBERER: &str = "board";

/// Role name -> capabilities. The board enforces roles; defining them is
/// the caller's job (orch), so [`Roles::default`] is only a starting set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Roles(pub BTreeMap<String, BTreeSet<Capability>>);

/// The capability, besides `Triage`, that re-files a card in `column`: closing work is what
/// files a Done card, so `Close` re-files one.
fn assign_capability(column: &Column) -> Capability {
    if *column == Column::DONE {
        Capability::Close
    } else {
        Capability::Project
    }
}

impl Default for Roles {
    fn default() -> Self {
        use Capability::*;
        let role = |caps: &[Capability]| caps.iter().copied().collect();
        Self(BTreeMap::from([
            (
                "human".into(),
                role(&[
                    Triage, Plan, Architect, Build, Test, Review, Merge, Close, Project, Admin,
                ]),
            ),
            ("builder".into(), role(&[Build, Close, Project])),
            ("planner".into(), role(&[Plan])),
            ("architect".into(), role(&[Architect])),
            ("tester".into(), role(&[Test])),
            ("qa".into(), role(&[Test])),
            // Merging is the human's gate, so the reviewer passes work on but never merges.
            ("reviewer".into(), role(&[Review])),
        ]))
    }
}

impl Roles {
    /// The actor `name` acting as `role`.
    ///
    /// # Errors
    /// If the role is unknown or the name blank.
    pub fn actor(&self, name: &str, role: &str) -> Result<Actor, BoardError> {
        let caps = self
            .0
            .get(role)
            .ok_or(BoardError::Invalid("unknown role"))?;
        if name.trim().is_empty() {
            return Err(BoardError::Invalid("actor name is empty"));
        }
        Ok(Actor {
            name: name.to_owned(),
            caps: caps.clone(),
            projects: None,
        })
    }
}

/// Makes a changed project registry durable (the projects file); see [`Board::set_projects`].
pub type ProjectsSink = Box<dyn Fn(&Projects) -> Result<(), String> + Send + Sync>;

/// The board over a store.
pub struct Board<S> {
    store: S,
    roles: RwLock<Roles>,
    rules: RwLock<MoveRules>,
    workflow: RwLock<Workflow>,
    projects: RwLock<Projects>,
    projects_sink: Option<ProjectsSink>,
    /// Held while numbers are handed out, so this process does not race itself for them.
    /// Uniqueness does not rest on it: the store reserves each number (`Store::reserve_number`).
    numbering: Mutex<()>,
    clock: fn() -> i64,
}

/// A read guard even if a writer panicked: the data is plain values, always consistent.
fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read()
        .map_or_else(|e| e.into_inner().clone(), |g| g.clone())
}

/// Wall-clock Unix milliseconds.
pub fn system_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl<S: Store> Board<S> {
    /// A board using the system clock.
    pub fn new(store: S, roles: Roles) -> Self {
        Self::with_clock(store, roles, system_ms)
    }

    /// A board with an injected clock (tests).
    pub fn with_clock(store: S, roles: Roles, clock: fn() -> i64) -> Self {
        Self {
            store,
            roles: RwLock::new(roles),
            rules: RwLock::new(MoveRules::default()),
            workflow: RwLock::new(Workflow::default()),
            projects: RwLock::new(Projects::default()),
            projects_sink: None,
            numbering: Mutex::new(()),
            clock,
        }
    }

    /// Serve these projects instead of only `default`.
    #[must_use]
    pub fn with_projects(self, projects: Projects) -> Self {
        *self
            .projects
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = projects;
        self
    }

    /// Start with these movement rules instead of [`MoveRules::Enforced`].
    #[must_use]
    pub fn with_move_rules(self, rules: MoveRules) -> Self {
        self.set_move_rules(rules);
        self
    }

    /// Use `workflow` instead of [`Workflow::default`].
    #[must_use]
    pub fn with_workflow(self, workflow: Workflow) -> Self {
        *self
            .workflow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = workflow;
        self
    }

    /// Let [`Board::add_project`] change the registry, persisting it through `sink`.
    #[must_use]
    pub fn with_projects_sink(mut self, sink: ProjectsSink) -> Self {
        self.projects_sink = Some(sink);
        self
    }

    /// Add project `name`, owning `paths`, to repository `repo` (created if new). Additive
    /// only (see [`Projects::with_project`]), so it widens no credential's reach: a credential
    /// limited to some projects does not gain the new one.
    ///
    /// # Errors
    /// If `by` lacks `Project`, the board has no projects file, or the project is refused.
    pub fn add_project(
        &self,
        by: &Actor,
        repo: &str,
        name: &str,
        paths: &[String],
    ) -> Result<Projects, BoardError> {
        if !by.caps.contains(&Capability::Project) {
            return Err(BoardError::Forbidden(Capability::Project));
        }
        let Some(sink) = &self.projects_sink else {
            return Err(BoardError::Refused(
                "this board has no projects file to add a project to".to_owned(),
            ));
        };
        let mut added = None;
        self.update_projects(
            |current| {
                let next = current
                    .with_project(repo, name, paths)
                    .map_err(BoardError::Refused)?;
                added = Some(next.clone());
                Ok(next)
            },
            |p| sink(p),
        )?;
        added.ok_or_else(|| BoardError::Storage("project registry not derived".to_owned()))
    }

    /// A snapshot of the project registry.
    pub fn projects(&self) -> Projects {
        read(&self.projects)
    }

    /// A snapshot of the role registry.
    pub fn roles(&self) -> Roles {
        read(&self.roles)
    }

    /// The registry, held shared: while a guard lives, no project can be removed.
    fn registry(&self) -> RwLockReadGuard<'_, Projects> {
        self.projects
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replace the project registry. Refused if a card still belongs to a project the new
    /// registry lacks (move or finish those cards first). `persist` makes the new registry
    /// durable and runs under the write lock, after validation and before publication: no
    /// card can be added to or assigned into a project while it is being removed, nor into one
    /// that is not yet durable, and if `persist` fails nothing has changed.
    ///
    /// # Errors
    /// [`BoardError::Refused`] naming the project or what `persist` reported, or a storage failure.
    pub fn set_projects(
        &self,
        projects: Projects,
        persist: impl FnOnce(&Projects) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        self.update_projects(|_| Ok(projects), persist)
    }

    /// [`Board::set_projects`] with the new registry derived from the current one under the
    /// same write lock, so a concurrent change is never lost.
    fn update_projects(
        &self,
        derive: impl FnOnce(&Projects) -> Result<Projects, BoardError>,
        persist: impl FnOnce(&Projects) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        let mut guard = self
            .projects
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let projects = derive(&guard)?;
        if let Some(card) = self
            .store
            .list()?
            .iter()
            .find(|c| guard.contains(&c.project) && !projects.contains(&c.project))
        {
            return Err(BoardError::Refused(format!(
                "project {:?} still has cards (for example {:?}); move them to another project first",
                card.project, card.title
            )));
        }
        persist(&projects).map_err(BoardError::Refused)?;
        *guard = projects;
        Ok(())
    }

    /// Replace the role registry. The caller checks that nobody is locked out.
    pub fn set_roles(&self, roles: Roles) {
        *self
            .roles
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = roles;
    }

    /// The movement rules in force.
    pub fn move_rules(&self) -> MoveRules {
        read(&self.rules)
    }

    /// Turn the movement rules on or off. The caller checks the actor may.
    pub fn set_move_rules(&self, rules: MoveRules) {
        *self
            .rules
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = rules;
    }

    /// The workflow in force: columns and the moves between them.
    pub fn workflow(&self) -> Workflow {
        self.flow().clone()
    }

    /// The workflow, held shared: while a guard lives, [`Board::set_workflow`] waits. Every
    /// write that depends on it (a move, a take) holds it through its commit, so a change to
    /// the workflow never lands between a check and the write it allowed. Never take it twice
    /// in one call: a waiting writer would block the second.
    fn flow(&self) -> RwLockReadGuard<'_, Workflow> {
        self.workflow
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replace the workflow. Refused if a card is still in a column the new workflow lacks
    /// (move those cards first). `persist` makes it durable and runs under the write lock,
    /// after validation and before publication; if it fails nothing has changed. The write
    /// lock waits for moves and takes in flight and holds new ones until it is published, so
    /// none commits under the workflow it replaces.
    ///
    /// # Errors
    /// [`BoardError::Refused`] naming the column or what `persist` reported, or a storage failure.
    pub fn set_workflow(
        &self,
        next: Workflow,
        persist: impl FnOnce(&Workflow) -> Result<(), String>,
    ) -> Result<(), BoardError> {
        let mut guard = self
            .workflow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let held = self
            .store
            .list()?
            .into_iter()
            .find(|c| guard.column(&c.column).is_some() && next.column(&c.column).is_none());
        if let Some(card) = held {
            return Err(BoardError::Refused(format!(
                "column {} still has cards (for example {:?}); move them to another column first",
                card.column, card.title
            )));
        }
        persist(&next).map_err(BoardError::Refused)?;
        *guard = next;
        Ok(())
    }

    /// Every card, each [`placed`] in the workflow.
    fn cards(&self) -> Result<Vec<Card>, BoardError> {
        self.cards_in(&self.flow())
    }

    fn cards_in(&self, flow: &Workflow) -> Result<Vec<Card>, BoardError> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|c| placed(flow, c))
            .collect())
    }

    /// All cards, optionally one column only.
    ///
    /// # Errors
    /// On storage failure.
    pub fn list(&self, column: Option<&Column>) -> Result<Vec<Card>, BoardError> {
        let mut cards = self.cards()?;
        cards.retain(|c| column.is_none_or(|col| c.column == *col));
        Ok(cards)
    }

    /// The roadmap of `project` as of now (see [`roadmap`](crate::roadmap::roadmap)).
    ///
    /// # Errors
    /// If the store fails.
    pub fn roadmap(&self, project: &str) -> Result<Roadmap, BoardError> {
        Ok(roadmap(
            &self.workflow(),
            &self.cards()?,
            project,
            (self.clock)(),
        ))
    }

    /// The metrics, as of now, of the cards `keep` selects (see
    /// [`metrics`](crate::metrics::metrics)).
    ///
    /// # Errors
    /// If the store fails.
    pub fn metrics(&self, keep: impl Fn(&Card) -> bool) -> Result<Metrics, BoardError> {
        let mut cards = self.cards()?;
        cards.retain(|c| keep(c));
        Ok(metrics(&self.workflow(), &cards, (self.clock)()))
    }

    /// One card.
    ///
    /// # Errors
    /// If missing, or on storage failure.
    pub fn get(&self, id: Uuid) -> Result<Card, BoardError> {
        self.get_in(&self.flow(), id)
    }

    fn get_in(&self, flow: &Workflow, id: Uuid) -> Result<Card, BoardError> {
        self.store
            .get(id)?
            .map(|c| placed(flow, c))
            .ok_or(BoardError::NotFound(id))
    }

    /// The card numbered `number`.
    ///
    /// # Errors
    /// If no card has that number, if more than one has (a number tag edited by hand in
    /// rusty_tick): never a guess at which was meant. Or on storage failure.
    pub fn get_number(&self, number: u64) -> Result<Card, BoardError> {
        let mut found = self
            .store
            .list()?
            .into_iter()
            .filter(|c| c.number == Some(number));
        match (found.next(), found.next()) {
            (Some(card), None) => Ok(card),
            (None, _) => Err(BoardError::Invalid("no card has that number")),
            (Some(_), Some(_)) => Err(BoardError::Invalid(
                "more than one card has that number; name it by id",
            )),
        }
    }

    /// Number every card that has none yet (made before numbering, or directly in rusty_tick),
    /// oldest first. Returns how many were numbered.
    ///
    /// # Errors
    /// On storage failure; cards numbered before it keep their numbers.
    pub fn number_cards(&self) -> Result<usize, BoardError> {
        let _numbering = self
            .numbering
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.number_strays().map(|(numbered, _)| numbered)
    }

    /// Number the cards that have none, each with a number the store reserves; returns how many
    /// it numbered and the highest number on the board. The caller holds `numbering`.
    fn number_strays(&self) -> Result<(usize, u64), BoardError> {
        let numberer = Actor {
            name: NUMBERER.into(),
            caps: BTreeSet::new(),
            projects: None,
        };
        let cards = self.store.list()?;
        let mut highest = cards.iter().filter_map(|c| c.number).max().unwrap_or(0);
        let strays: Vec<Uuid> = cards
            .iter()
            .filter(|c| c.number.is_none())
            .map(|c| c.id)
            .collect();
        for id in &strays {
            highest = self.store.reserve_number(*id, highest)?;
            // Another board may number the card first; then this reservation goes unused.
            self.apply(*id, &numberer, EventKind::Numbered, |c, _| {
                Ok(c.numbered(highest))
            })?;
        }
        Ok((strays.len(), highest))
    }

    /// Commit a new card under a newly reserved number.
    fn insert_numbered(&self, by: &Actor, mut card: Card) -> Result<Card, BoardError> {
        let _numbering = self
            .numbering
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let highest = self.number_strays()?.1;
        card.number = Some(self.store.reserve_number(card.id, highest)?);
        card.rev = self
            .store
            .insert(&card, Event::of(&card, EventKind::Created, &by.name))?;
        Ok(card)
    }

    /// Events after the cursor.
    ///
    /// # Errors
    /// On storage failure.
    pub fn events(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        self.store.events_since(after, limit)
    }

    /// The newest event id: a cursor that has seen everything committed so far.
    ///
    /// # Errors
    /// On storage failure.
    pub fn cursor(&self) -> Result<Option<Uuid>, BoardError> {
        self.store.last_event_id()
    }

    /// `Err` once the store can no longer commit and must be reopened.
    ///
    /// # Errors
    /// On storage failure.
    pub fn health(&self) -> Result<(), BoardError> {
        self.store.health()
    }

    /// Create a Backlog card in the `default` project.
    ///
    /// # Errors
    /// On a blank title or storage failure.
    pub fn add(
        &self,
        by: &Actor,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        self.add_in(by, DEFAULT_PROJECT, title, body, tags)
    }

    /// Create a Backlog card in `project`.
    ///
    /// # Errors
    /// On a blank title, an unknown project, a project outside `by`'s scope, or
    /// storage failure.
    pub fn add_in(
        &self,
        by: &Actor,
        project: &str,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        let card = Card::new(title, body, tags, (self.clock)())?;
        self.create(by, project, card)
    }

    /// Create a card straight in Done in `project`, recording work that
    /// already merged outside the board's flow. Needs `Close`.
    ///
    /// # Errors
    /// As [`Board::add_in`], or if `by` lacks `Close`.
    pub fn add_done_in(
        &self,
        by: &Actor,
        project: &str,
        title: &str,
        body: &str,
        tags: Vec<String>,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Close) {
            return Err(BoardError::Forbidden(Capability::Close));
        }
        let card = Card::new(title, body, tags, (self.clock)())?;
        self.create(
            by,
            project,
            Card {
                column: Column::DONE,
                done_ms: Some(card.created_ms),
                ..card
            },
        )
    }

    fn create(&self, by: &Actor, project: &str, mut card: Card) -> Result<Card, BoardError> {
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        project.clone_into(&mut card.project);
        let card = self.insert_numbered(by, card)?;
        drop(registry);
        Ok(card)
    }

    /// Create a Backlog card for an existing forge issue.
    ///
    /// # Errors
    /// On a blank title or storage failure.
    pub fn import(
        &self,
        by: &Actor,
        title: &str,
        body: &str,
        tags: Vec<String>,
        issue: u64,
    ) -> Result<Card, BoardError> {
        let mut card = Card::new(title, body, tags, (self.clock)())?;
        card.id = import_id(issue);
        card.issue = Some(issue);
        card.synced_text = Some(text_hash(&card.title, &card.body));
        self.insert_numbered(by, card)
    }

    /// Record the forge issue a card is synced with, and the text hash both
    /// sides agreed on at that moment (`None` when unknown).
    ///
    /// # Errors
    /// If the card is missing or on storage failure.
    pub fn link_issue(
        &self,
        id: Uuid,
        issue: u64,
        synced_text: Option<u64>,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Linked, |c, now| {
            Ok(c.link(issue, synced_text, now))
        })
    }

    /// Record that the card's title and body match its issue (hash `synced_text`).
    ///
    /// # Errors
    /// If the card is missing or on storage failure.
    pub fn mark_synced(&self, id: Uuid, synced_text: u64, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Synced, |c, now| {
            Ok(c.mark_synced(synced_text, now))
        })
    }

    /// Persist or adopt the one factory execution linked to this card. Builders
    /// may start work, but cannot replace an existing logical execution.
    pub fn bind_execution(
        &self,
        id: Uuid,
        link: ExecutionLink,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Build) {
            return Err(BoardError::Forbidden(Capability::Build));
        }
        self.apply(id, by, EventKind::ExecutionLinked, |c, now| {
            c.bind_execution(link.clone(), now)
        })
    }

    /// Move this card's execution to a replacement assignment generation, if
    /// the stored link still carries `expected_generation` (see
    /// [`Card::recover_execution`]). Needs Build, like binding.
    pub fn recover_execution(
        &self,
        id: Uuid,
        expected_generation: u64,
        link: ExecutionLink,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Build) {
            return Err(BoardError::Forbidden(Capability::Build));
        }
        self.apply(id, by, EventKind::ExecutionRecovered, |c, now| {
            c.recover_execution(expected_generation, link.clone(), now)
        })
    }

    /// Pull work: the next card this actor can act on, entered (Ready ->
    /// Building) and claimed. With `id`, that card or an error. Cards nearest
    /// Done come first, then oldest. Cards claimed by someone else are
    /// skipped; the actor's own claims are resumed. `Ok(None)` when nothing
    /// is waiting for them.
    ///
    /// # Errors
    /// On storage failure, or (with `id`) whatever refused that card.
    pub fn take(&self, by: &Actor, id: Option<Uuid>) -> Result<Option<Card>, BoardError> {
        self.take_in(by, id, None)
    }

    /// As [`Board::take`], restricted to cards of `project` when given. A card of a project
    /// outside `by`'s scope is refused by the write itself (see `apply`), so it is skipped
    /// when scanning and an error when named by `id`.
    ///
    /// # Errors
    /// As [`Board::take`], plus an unknown or out-of-scope `project`.
    pub fn take_in(
        &self,
        by: &Actor,
        id: Option<Uuid>,
        project: Option<&str>,
    ) -> Result<Option<Card>, BoardError> {
        if let Some(p) = project {
            if !self.registry().contains(p) {
                return Err(BoardError::Invalid("unknown project"));
            }
            if !by.may_work_in(p) {
                return Err(BoardError::OutOfScope(p.to_owned()));
            }
        }
        let flow = self.flow();
        let queues = flow.work_queues();
        let rank = |c: &Card| queues.iter().position(|q| *q == c.column);
        let cards = self.cards_in(&flow)?;
        let waiting = waiting_on_deps(&cards);
        if let Some(i) = id
            && waiting.contains(&i)
        {
            return Err(waits_on_deps());
        }
        let mut candidates: Vec<Card> = cards
            .into_iter()
            .filter(|c| id.is_none_or(|i| c.id == i))
            .filter(|c| project.is_none_or(|p| c.project == p))
            .filter(|c| rank(c).is_some() && flow.actionable(&c.column, &by.caps))
            .filter(|c| id.is_some() || c.claim.as_deref().is_none_or(|h| h == by.name))
            .filter(|c| !waiting.contains(&c.id))
            .collect();
        // Nearest Done first; within a queue the highest priority, then the manual order,
        // then the oldest.
        candidates.sort_by_key(|c| (rank(c), Reverse(c.priority), c.sort_order, c.created_ms));
        for card in candidates {
            match self.enter_and_claim(&flow, card, by) {
                Ok(card) => return Ok(Some(card)),
                // Another worker got there first and the card has moved on or been claimed:
                // try the next one. (With an explicit `id`, say why instead.)
                Err(
                    BoardError::Conflict
                    | BoardError::Claimed(_)
                    | BoardError::NoSuchMove { .. }
                    | BoardError::Forbidden(_)
                    | BoardError::NotClaimant
                    | BoardError::Terminal
                    | BoardError::OutOfScope(_)
                    | BoardError::NotFound(_)
                    | BoardError::Refused(_),
                ) if id.is_none() => {}
                Err(e) => return Err(e),
            }
        }
        match id {
            Some(i) => Err(BoardError::NotFound(i)),
            None => Ok(None),
        }
    }

    /// Enter (if the card waits in a queue) and claim it, in one commit, so a take that is
    /// refused leaves the card where it was. The commit re-checks the card *inside* its
    /// compare-and-swap, so a card another worker moved on or claimed since it was listed is
    /// refused instead of half-taken; and a card is entered only into a column whose work
    /// this actor does, so it is never moved and then left unclaimed.
    fn enter_and_claim(
        &self,
        flow: &Workflow,
        listed: Card,
        by: &Actor,
    ) -> Result<Card, BoardError> {
        let id = listed.id;
        let queues = flow.work_queues();
        let works = |col: &Column| queues.contains(col) && flow.actionable(col, &by.caps);
        let current = self.get_in(flow, id)?;
        if !works(&current.column) {
            return Err(BoardError::Conflict);
        }
        self.check_deps_done(&current)?;
        // Refuse before writing anything: a claimed card must not be half-taken.
        if let Some(holder) = current.claim.as_ref().filter(|h| **h != by.name) {
            return Err(BoardError::Claimed(holder.clone()));
        }
        let Some(enter) = flow.entry_column(&current.column) else {
            return self.apply_in(flow, id, by, EventKind::Claimed, |c, now| {
                if !works(&c.column) {
                    return Err(BoardError::Conflict);
                }
                self.check_deps_done(c)?;
                c.claim(by, now)
            });
        };
        if !works(enter) {
            return Err(BoardError::Refused(format!(
                "taking this card moves it to {enter}, whose work your role does not do"
            )));
        }
        self.apply_in(flow, id, by, EventKind::Moved, |c, now| {
            if flow.entry_column(&c.column) != Some(enter) {
                return Err(BoardError::Conflict);
            }
            self.check_deps_done(c)?;
            // Checked on the card being committed, not the earlier read.
            if let Some(holder) = c.claim.as_ref().filter(|h| **h != by.name) {
                return Err(BoardError::Claimed(holder.clone()));
            }
            let entered = c.move_to(flow, enter, by, now)?;
            Ok(Card {
                claim: Some(by.name.clone()),
                ..entered
            })
        })
    }

    /// Refuse `card` while any of its dependencies is not Done. Checked on the card being
    /// committed, so dependencies planned after `take` listed it still count; a Done
    /// dependency stays Done (it is terminal), so the check cannot go stale.
    fn check_deps_done(&self, card: &Card) -> Result<(), BoardError> {
        for dep in &card.deps {
            if self
                .store
                .get(*dep)?
                .is_some_and(|d| d.column != Column::DONE)
            {
                return Err(waits_on_deps());
            }
        }
        Ok(())
    }

    /// Apply `op` to the card and commit, re-reading on conflict.
    fn apply(
        &self,
        id: Uuid,
        by: &Actor,
        kind: EventKind,
        op: impl Fn(&Card, i64) -> Result<Card, BoardError>,
    ) -> Result<Card, BoardError> {
        self.apply_in(&self.flow(), id, by, kind, op)
    }

    /// [`Board::apply`] under `flow`, a guard the caller holds (see [`Board::flow`]).
    fn apply_in(
        &self,
        flow: &Workflow,
        id: Uuid,
        by: &Actor,
        kind: EventKind,
        op: impl Fn(&Card, i64) -> Result<Card, BoardError>,
    ) -> Result<Card, BoardError> {
        for _ in 0..RETRIES {
            let current = self.get_in(flow, id)?;
            // Checked on the card being committed, so a card re-assigned mid-call is refused too.
            if !by.may_work_in(&current.project) {
                return Err(BoardError::OutOfScope(current.project));
            }
            let next = op(&current, (self.clock)())?;
            // Domain operations may deliberately return the unchanged aggregate
            // for an idempotent replay.  Such a replay is already durable and
            // must not be sent through the store's rev + 1 CAS.
            if next == current {
                return Ok(current);
            }
            match self.store.replace(&next, Event::of(&next, kind, &by.name)) {
                Err(BoardError::Conflict) => continue,
                // The store's revision, not ours: an adapter that commits in two
                // steps moves it twice, and the caller's next edit names it.
                result => return result.map(|rev| Card { rev, ..next }),
            }
        }
        Err(BoardError::Conflict)
    }

    /// Move a card, under the movement rules in force. A move made with the rules off is
    /// logged, so what bypassed the table can be traced.
    ///
    /// # Errors
    /// See [`Card::move_to`], or [`Card::move_freely`] when the rules are off.
    pub fn move_card(&self, id: Uuid, to: &Column, by: &Actor) -> Result<Card, BoardError> {
        let flow = self.flow();
        match self.move_rules() {
            MoveRules::Enforced => self.apply_in(&flow, id, by, EventKind::Moved, |c, now| {
                c.move_to(&flow, to, by, now)
            }),
            MoveRules::Open => {
                let card = self.apply_in(&flow, id, by, EventKind::Moved, |c, now| {
                    c.move_freely(&flow, to, by, now)
                })?;
                rusty_log::info!(card = %id, actor = %by.name, to = %to, "card moved with movement rules off");
                Ok(card)
            }
        }
    }

    /// Move a card to another project. Needs `Triage`, or else `Project` (`Close` for a Done
    /// card, which stays Done), and both projects in scope; without `Triage`, not a card
    /// someone else has claimed.
    ///
    /// # Errors
    /// An unknown project, a missing capability, an out-of-scope project, or a card claimed
    /// by someone else (without `Triage`).
    pub fn assign(&self, id: Uuid, project: &str, by: &Actor) -> Result<Card, BoardError> {
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        let triage = by.caps.contains(&Capability::Triage);
        let card = self.apply(id, by, EventKind::Assigned, |c, now| {
            let needed = assign_capability(&c.column);
            if !triage && !by.caps.contains(&needed) {
                return Err(BoardError::Forbidden(needed));
            }
            match &c.claim {
                Some(holder) if !triage && *holder != by.name => {
                    Err(BoardError::Claimed(holder.clone()))
                }
                _ => Ok(c.assign(project, now)),
            }
        });
        drop(registry);
        card
    }

    /// Claim a card.
    ///
    /// # Errors
    /// See [`Card::claim`].
    pub fn claim(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Claimed, |c, now| c.claim(by, now))
    }

    /// Release a claim.
    ///
    /// # Errors
    /// See [`Card::release`].
    pub fn release(&self, id: Uuid, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Released, |c, now| c.release(by, now))
    }

    /// Edit title/body/tags.
    ///
    /// # Errors
    /// See [`Card::edit`].
    pub fn edit(
        &self,
        id: Uuid,
        by: &Actor,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Card, BoardError> {
        self.edit_at(id, by, None, title, body, tags)
    }

    /// As [`Board::edit`], but only if the card is still at revision `expect_rev`
    /// (what the editor last saw); otherwise [`BoardError::Conflict`], so an
    /// editor never silently overwrites changes it has not seen.
    ///
    /// # Errors
    /// See [`Card::edit`], and [`BoardError::Conflict`] on a stale revision.
    pub fn edit_at(
        &self,
        id: Uuid,
        by: &Actor,
        expect_rev: Option<u64>,
        title: Option<&str>,
        body: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Edited, |c, now| {
            if expect_rev.is_some_and(|rev| c.rev != rev) {
                return Err(BoardError::Conflict);
            }
            c.edit(title, body, tags.clone(), now)
        })
    }

    /// Set a card's priority and dependencies (each optional). Needs `Triage`:
    /// what is taken first, and what waits, is a planning decision.
    ///
    /// # Errors
    /// A missing capability, a dependency that is not a card, or see [`Card::plan`].
    pub fn plan(
        &self,
        id: Uuid,
        by: &Actor,
        priority: Option<u8>,
        deps: Option<Vec<Uuid>>,
    ) -> Result<Card, BoardError> {
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        for dep in deps.iter().flatten() {
            if *dep != id && self.store.get(*dep)?.is_none() {
                return Err(BoardError::NotFound(*dep));
            }
        }
        if priority.is_none() && deps.is_none() {
            return Err(BoardError::Invalid("give a priority, deps or both"));
        }
        // One commit: `Card::plan` validates the whole request before anything changes.
        self.apply(id, by, EventKind::Edited, |c, now| {
            c.plan(priority, deps.clone(), now)
        })
    }

    /// Comment on a card.
    ///
    /// # Errors
    /// See [`Card::comment`].
    pub fn comment(&self, id: Uuid, by: &Actor, text: &str) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Commented, |c, now| {
            c.comment(by, text, now)
        })
    }
}

/// `card` as `flow` sees it: in Backlog if its column is not one of the workflow's (removed
/// while the card was moving in, or a tag edited by hand in rusty_tick).
fn placed(flow: &Workflow, card: Card) -> Card {
    if flow.column(&card.column).is_some() {
        return card;
    }
    Card {
        column: Column::BACKLOG,
        ..card
    }
}

/// Ids of the cards that wait on a dependency that is not Done. A dependency
/// that is no longer on the board does not hold anything up.
fn waits_on_deps() -> BoardError {
    BoardError::Refused("card waits on cards that are not done".into())
}

fn waiting_on_deps(cards: &[Card]) -> BTreeSet<Uuid> {
    let open: BTreeSet<Uuid> = cards
        .iter()
        .filter(|c| c.column != Column::DONE)
        .map(|c| c.id)
        .collect();
    cards
        .iter()
        .filter(|c| c.deps.iter().any(|d| open.contains(d)))
        .map(|c| c.id)
        .collect()
}
