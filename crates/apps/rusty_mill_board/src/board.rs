//! The service: resolves actors from roles, applies domain operations and
//! commits them through a [`Store`], retrying on revision conflicts.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{RwLock, RwLockReadGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{
    Actor, BoardError, Capability, Card, Column, DEFAULT_PROJECT, Event, EventKind, ExecutionLink,
    WORK_QUEUES, actionable, entry_column, text_hash,
};
use crate::projects::Projects;
use crate::store::Store;

/// Attempts per operation when another writer wins the revision race.
const RETRIES: usize = 3;

/// Role name -> capabilities. The board enforces roles; defining them is
/// the caller's job (orch), so [`Roles::default`] is only a starting set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Roles(pub BTreeMap<String, BTreeSet<Capability>>);

impl Default for Roles {
    fn default() -> Self {
        use Capability::*;
        let role = |caps: &[Capability]| caps.iter().copied().collect();
        Self(BTreeMap::from([
            (
                "human".into(),
                role(&[Triage, Build, Qa, Review, Merge, Admin]),
            ),
            ("builder".into(), role(&[Build])),
            ("qa".into(), role(&[Qa])),
            ("reviewer".into(), role(&[Review, Merge])),
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

/// The board over a store.
pub struct Board<S> {
    store: S,
    roles: RwLock<Roles>,
    projects: RwLock<Projects>,
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
            projects: RwLock::new(Projects::default()),
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
        let mut guard = self
            .projects
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(card) = self
            .store
            .list()?
            .iter()
            .find(|c| !projects.contains(&c.project))
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

    /// All cards, optionally one column only.
    ///
    /// # Errors
    /// On storage failure.
    pub fn list(&self, column: Option<Column>) -> Result<Vec<Card>, BoardError> {
        let mut cards = self.store.list()?;
        cards.retain(|c| column.is_none_or(|col| c.column == col));
        Ok(cards)
    }

    /// One card.
    ///
    /// # Errors
    /// If missing, or on storage failure.
    pub fn get(&self, id: Uuid) -> Result<Card, BoardError> {
        self.store.get(id)?.ok_or(BoardError::NotFound(id))
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

    /// Copy a consistent image of the store into the directory `dir`; returns the file name of
    /// the store's main file as copied.
    ///
    /// # Errors
    /// If the store is not durable or a file cannot be copied.
    pub fn backup(&self, dir: &std::path::Path) -> Result<std::ffi::OsString, BoardError> {
        self.store.backup(dir)
    }

    /// Fold the store's append log into its main files.
    ///
    /// # Errors
    /// On storage failure.
    pub fn compact(&self) -> Result<(), BoardError> {
        self.store.compact()
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
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        let mut card = Card::new(title, body, tags, (self.clock)())?;
        project.clone_into(&mut card.project);
        self.store
            .insert(&card, Event::of(&card, EventKind::Created, &by.name))?;
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
        card.issue = Some(issue);
        card.synced_text = Some(text_hash(&card.title, &card.body));
        self.store
            .insert(&card, Event::of(&card, EventKind::Created, &by.name))?;
        Ok(card)
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
        let rank = |c: &Card| WORK_QUEUES.iter().position(|q| *q == c.column);
        let mut candidates: Vec<Card> = self
            .store
            .list()?
            .into_iter()
            .filter(|c| id.is_none_or(|i| c.id == i))
            .filter(|c| project.is_none_or(|p| c.project == p))
            .filter(|c| rank(c).is_some() && actionable(c.column, &by.caps))
            .filter(|c| id.is_some() || c.claim.as_deref().is_none_or(|h| h == by.name))
            .collect();
        candidates.sort_by_key(|c| (rank(c), c.created_ms));
        for card in candidates {
            match self.enter_and_claim(card, by) {
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
                    | BoardError::NotFound(_),
                ) if id.is_none() => {}
                Err(e) => return Err(e),
            }
        }
        match id {
            Some(i) => Err(BoardError::NotFound(i)),
            None => Ok(None),
        }
    }

    /// Enter (if the card waits in Ready) and claim it. Every step re-checks the card
    /// *inside* its own compare-and-swap, so a card that another worker moved on or
    /// claimed since it was listed is refused instead of half-taken: a claim never
    /// lands on a card that is no longer in a queue this actor can work.
    fn enter_and_claim(&self, listed: Card, by: &Actor) -> Result<Card, BoardError> {
        let id = listed.id;
        let workable = |c: &Card| WORK_QUEUES.contains(&c.column) && actionable(c.column, &by.caps);
        let current = self.get(id)?;
        if !workable(&current) {
            return Err(BoardError::Conflict);
        }
        // Refuse before moving anything: a claimed card must not be half-taken.
        if let Some(holder) = current.claim.as_ref().filter(|h| **h != by.name) {
            return Err(BoardError::Claimed(holder.clone()));
        }
        if let Some(enter) = entry_column(current.column) {
            self.apply(id, by, EventKind::Moved, |c, now| {
                if entry_column(c.column) != Some(enter) {
                    return Err(BoardError::Conflict);
                }
                // Checked on the card being committed, not the earlier read.
                if let Some(holder) = c.claim.as_ref().filter(|h| **h != by.name) {
                    return Err(BoardError::Claimed(holder.clone()));
                }
                c.move_to(enter, by, now)
            })?;
        }
        self.apply(id, by, EventKind::Claimed, |c, now| {
            if !workable(c) {
                return Err(BoardError::Conflict);
            }
            c.claim(by, now)
        })
    }

    /// Apply `op` to the card and commit, re-reading on conflict.
    fn apply(
        &self,
        id: Uuid,
        by: &Actor,
        kind: EventKind,
        op: impl Fn(&Card, i64) -> Result<Card, BoardError>,
    ) -> Result<Card, BoardError> {
        for _ in 0..RETRIES {
            let current = self.get(id)?;
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
                result => return result.map(|()| next),
            }
        }
        Err(BoardError::Conflict)
    }

    /// Move a card.
    ///
    /// # Errors
    /// See [`Card::move_to`].
    pub fn move_card(&self, id: Uuid, to: Column, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Moved, |c, now| c.move_to(to, by, now))
    }

    /// Move a card to another project. Needs `Triage`, and both projects in scope.
    ///
    /// # Errors
    /// An unknown project, a missing capability, an out-of-scope project, or a Done card.
    pub fn assign(&self, id: Uuid, project: &str, by: &Actor) -> Result<Card, BoardError> {
        let registry = self.registry();
        if !registry.contains(project) {
            return Err(BoardError::Invalid("unknown project"));
        }
        if !by.caps.contains(&Capability::Triage) {
            return Err(BoardError::Forbidden(Capability::Triage));
        }
        if !by.may_work_in(project) {
            return Err(BoardError::OutOfScope(project.to_owned()));
        }
        let card = self.apply(id, by, EventKind::Assigned, |c, now| c.assign(project, now));
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
