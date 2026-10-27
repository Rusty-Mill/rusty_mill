//! The storage port and an in-memory adapter.

use std::collections::BTreeMap;
use std::sync::Mutex;

use uuid::Uuid;

use crate::domain::{BoardError, Card, Event, next_event_id};

/// Persistence for cards and their event log.
///
/// **A commit is a compare-and-swap**: `replace` succeeds only if the stored card
/// is at `card.rev - 1`, else [`BoardError::Conflict`], and the card and its event
/// become visible together. The store assigns the event's id inside the commit, so
/// event ids are in commit order.
///
/// Adapters differ in two ways the board allows for. Events may live only as long
/// as the process ([`crate::tick::TickStore`]: rusty_tick keeps no change log).
/// A commit may take more than one fenced step (the tick adapter's `card_plan`),
/// so the revision it returns can be ahead of `card.rev`.
pub trait Store: Send + Sync {
    /// One card.
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError>;
    /// All cards, oldest first.
    fn list(&self) -> Result<Vec<Card>, BoardError>;
    /// Commit a new card (rev 0) and its event. The event's `id` is ignored
    /// and assigned here. Returns the revision committed, which a later `replace`
    /// must follow: `card.rev`, or more when the adapter commits in several steps.
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError>;
    /// Commit the next revision of a card and its event (id assigned here).
    /// Returns the revision committed, as for `insert`.
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError>;
    /// Events with id greater than `after`, in commit order, at most `limit`.
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError>;
    /// The id of the newest committed event: a cursor that has seen everything so far.
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError>;
    /// `Err` once the store can no longer commit and must be reopened.
    ///
    /// # Errors
    /// [`BoardError::Storage`] naming why.
    fn health(&self) -> Result<(), BoardError> {
        Ok(())
    }
}

/// The committed state [`MemStore`] keeps: cards by id and the event log in
/// commit (= id) order. Mutated only under its lock.
#[derive(Default)]
struct Log {
    cards: BTreeMap<Uuid, Card>,
    events: Vec<Event>,
}

impl Log {
    /// Check a commit could apply: the card is new (`insert`) or exactly one revision ahead.
    ///
    /// # Errors
    /// [`BoardError::Conflict`] for a duplicate id or a stale revision, [`BoardError::NotFound`] for a missing card.
    fn check(&self, card: &Card, is_insert: bool) -> Result<(), BoardError> {
        match (self.cards.get(&card.id), is_insert) {
            (Some(_), true) => Err(BoardError::Conflict),
            (None, true) => Ok(()),
            (None, false) => Err(BoardError::NotFound(card.id)),
            (Some(current), false) if current.rev + 1 == card.rev => Ok(()),
            (Some(_), false) => Err(BoardError::Conflict),
        }
    }

    /// The event as it will be committed: its id above every earlier one.
    fn stamp(&self, mut event: Event) -> Event {
        event.id = next_event_id(self.events.last().map(|e| e.id));
        event
    }

    /// Apply a checked commit.
    fn apply(&mut self, card: &Card, event: Event) {
        self.cards.insert(card.id, card.clone());
        self.events.push(event);
    }

    /// One card.
    fn get(&self, id: Uuid) -> Option<Card> {
        self.cards.get(&id).cloned()
    }

    /// All cards, oldest first.
    fn list(&self) -> Vec<Card> {
        let mut cards: Vec<_> = self.cards.values().cloned().collect();
        cards.sort_by_key(|c| (c.created_ms, c.id));
        cards
    }

    /// Events after `after`, up to `limit`. Binary search: the log is sorted by id.
    fn since(&self, after: Option<Uuid>, limit: usize) -> Vec<Event> {
        let start = after.map_or(0, |a| self.events.partition_point(|e| e.id <= a));
        self.events[start..].iter().take(limit).cloned().collect()
    }

    /// The newest event id.
    fn last_id(&self) -> Option<Uuid> {
        self.events.last().map(|e| e.id)
    }
}

/// In-memory store for tests and ephemeral boards.
#[derive(Default)]
pub struct MemStore {
    log: Mutex<Log>,
}

fn poisoned<T>(_: T) -> BoardError {
    BoardError::Storage("lock poisoned".into())
}

impl MemStore {
    fn commit(&self, card: &Card, event: Event, is_insert: bool) -> Result<(), BoardError> {
        let mut log = self.log.lock().map_err(poisoned)?;
        log.check(card, is_insert)?;
        let event = log.stamp(event);
        log.apply(card, event);
        Ok(())
    }
}

impl Store for MemStore {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.get(id))
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.list())
    }

    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, true).map(|()| card.rev)
    }

    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        self.commit(card, event, false).map(|()| card.rev)
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.since(after, limit))
    }

    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        Ok(self.log.lock().map_err(poisoned)?.last_id())
    }
}

impl<T: Store + ?Sized> Store for Box<T> {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        (**self).get(id)
    }
    fn list(&self) -> Result<Vec<Card>, BoardError> {
        (**self).list()
    }
    fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        (**self).insert(card, event)
    }
    fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
        (**self).replace(card, event)
    }
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        (**self).events_since(after, limit)
    }
    fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
        (**self).last_event_id()
    }
    fn health(&self) -> Result<(), BoardError> {
        (**self).health()
    }
}
