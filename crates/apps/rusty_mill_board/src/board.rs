//! The service: resolves actors from roles, applies domain operations and
//! commits them through a [`Store`], retrying on revision conflicts.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::domain::{Actor, BoardError, Capability, Card, Column, Event, EventKind};
use crate::store::Store;

/// Attempts per operation when another writer wins the revision race.
const RETRIES: usize = 3;

/// Role name -> capabilities. The board enforces roles; defining them is
/// the caller's job (orch), so [`Roles::default`] is only a starting set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roles(pub BTreeMap<String, BTreeSet<Capability>>);

impl Default for Roles {
    fn default() -> Self {
        use Capability::*;
        let role = |caps: &[Capability]| caps.iter().copied().collect();
        Self(BTreeMap::from([
            ("human".into(), role(&[Triage, Build, Qa, Review, Merge])),
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
        })
    }
}

/// The board over a store.
pub struct Board<S> {
    store: S,
    roles: Roles,
    clock: fn() -> i64,
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
        Self {
            store,
            roles,
            clock: system_ms,
        }
    }

    /// A board with an injected clock (tests).
    pub fn with_clock(store: S, roles: Roles, clock: fn() -> i64) -> Self {
        Self {
            store,
            roles,
            clock,
        }
    }

    /// The role registry.
    pub fn roles(&self) -> &Roles {
        &self.roles
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

    /// Create a Backlog card.
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
        let card = Card::new(title, body, tags, (self.clock)())?;
        self.store
            .insert(&card, &Event::of(&card, EventKind::Created, &by.name))?;
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
        self.store
            .insert(&card, &Event::of(&card, EventKind::Created, &by.name))?;
        Ok(card)
    }

    /// Record the forge issue a card is synced with.
    ///
    /// # Errors
    /// If the card is missing or on storage failure.
    pub fn link_issue(&self, id: Uuid, issue: u64, by: &Actor) -> Result<Card, BoardError> {
        self.apply(id, by, EventKind::Linked, |c, now| Ok(c.link(issue, now)))
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
            let next = op(&self.get(id)?, (self.clock)())?;
            match self.store.replace(&next, &Event::of(&next, kind, &by.name)) {
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
        self.apply(id, by, EventKind::Edited, |c, now| {
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
