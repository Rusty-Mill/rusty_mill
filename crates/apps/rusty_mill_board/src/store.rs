//! The storage port and an in-memory adapter.

use std::collections::BTreeMap;
use std::sync::RwLock;

use uuid::Uuid;

use crate::domain::{BoardError, Card, Event};

/// Persistence for cards and their event log.
///
/// A write persists the card and appends its event together. `replace` is a
/// compare-and-swap on the revision: it succeeds only if the stored card is
/// at `card.rev - 1`, else [`BoardError::Conflict`].
pub trait Store: Send + Sync {
    /// One card.
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError>;
    /// All cards, oldest first.
    fn list(&self) -> Result<Vec<Card>, BoardError>;
    /// Store a new card (rev 0) and its event.
    fn insert(&self, card: &Card, event: &Event) -> Result<(), BoardError>;
    /// Swap in the next revision of a card, and append its event.
    fn replace(&self, card: &Card, event: &Event) -> Result<(), BoardError>;
    /// Events with id greater than `after`, oldest first, at most `limit`.
    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError>;
}

/// In-memory store for tests and ephemeral boards.
#[derive(Default)]
pub struct MemStore {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    cards: BTreeMap<Uuid, Card>,
    events: BTreeMap<Uuid, Event>,
}

fn poisoned<T>(_: T) -> BoardError {
    BoardError::Storage("lock poisoned".into())
}

impl Store for MemStore {
    fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
        Ok(self.inner.read().map_err(poisoned)?.cards.get(&id).cloned())
    }

    fn list(&self) -> Result<Vec<Card>, BoardError> {
        let mut cards: Vec<_> = self.inner.read().map_err(poisoned)?.cards.values().cloned().collect();
        cards.sort_by_key(|c| (c.created_ms, c.id));
        Ok(cards)
    }

    fn insert(&self, card: &Card, event: &Event) -> Result<(), BoardError> {
        let mut inner = self.inner.write().map_err(poisoned)?;
        if inner.cards.contains_key(&card.id) {
            return Err(BoardError::Conflict);
        }
        inner.cards.insert(card.id, card.clone());
        inner.events.insert(event.id, event.clone());
        Ok(())
    }

    fn replace(&self, card: &Card, event: &Event) -> Result<(), BoardError> {
        let mut inner = self.inner.write().map_err(poisoned)?;
        let current = inner.cards.get(&card.id).ok_or(BoardError::NotFound(card.id))?;
        if current.rev + 1 != card.rev {
            return Err(BoardError::Conflict);
        }
        inner.cards.insert(card.id, card.clone());
        inner.events.insert(event.id, event.clone());
        Ok(())
    }

    fn events_since(&self, after: Option<Uuid>, limit: usize) -> Result<Vec<Event>, BoardError> {
        let inner = self.inner.read().map_err(poisoned)?;
        Ok(inner
            .events
            .values()
            .filter(|e| after.is_none_or(|a| e.id > a))
            .take(limit)
            .cloned()
            .collect())
    }
}
