//! Milestones: a project's dated goals, drawn on the roadmap ahead of today.
//!
//! A milestone is a title and a target date in one project, done once closed. A card may link to
//! one milestone of its project (`Card::milestone`), and the milestone's progress is rolled up from
//! those cards (see [`crate::trace`]); the roadmap shows both on one time line. Milestones are never deleted,
//! only closed, so a milestone once shown keeps its place in the project's history.
//!
//! Storage is its own small port, [`MilestoneStore`], so the card [`Store`](crate::store::Store)
//! stays card-only. In rusty_tick a milestone is a task tagged `milestone` (never `agent`, so it
//! is not a card) in the project's list, with `dueMs` as its target and closed when done.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::BoardError;

/// Longest milestone title, in characters.
pub const MAX_TITLE: usize = 200;

/// A project's dated goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Milestone {
    /// Identity.
    pub id: Uuid,
    /// The project it belongs to.
    pub project: String,
    /// What is reached.
    pub title: String,
    /// When it should be reached, Unix ms.
    pub target_ms: i64,
    /// When it was reached, Unix ms; `None` while open.
    pub done_ms: Option<i64>,
    /// The stored version this was read at: a save is refused once it has moved on, so a scope
    /// check made on this read still holds when the write lands.
    pub rev: u64,
}

/// A change to a milestone: each `None` field is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MilestoneEdit {
    /// A new title.
    pub title: Option<String>,
    /// A new target, Unix ms.
    pub target_ms: Option<i64>,
    /// `true` closes it, `false` reopens it.
    pub done: Option<bool>,
}

impl Milestone {
    /// A new open milestone.
    ///
    /// # Errors
    /// A blank or overlong title, or a target that is not a positive time.
    pub fn new(project: &str, title: &str, target_ms: i64) -> Result<Self, BoardError> {
        Ok(Self {
            id: Uuid::now_v7(),
            project: project.to_owned(),
            title: valid_title(title)?,
            target_ms: valid_target(target_ms)?,
            done_ms: None,
            rev: 0,
        })
    }

    /// This milestone with `edit` applied at `now_ms`.
    ///
    /// # Errors
    /// An empty edit, or a field [`Milestone::new`] would refuse.
    pub fn edited(&self, edit: &MilestoneEdit, now_ms: i64) -> Result<Self, BoardError> {
        if *edit == MilestoneEdit::default() {
            return Err(BoardError::Invalid("give a title, target_ms or done"));
        }
        let mut next = self.clone();
        if let Some(t) = &edit.title {
            next.title = valid_title(t)?;
        }
        if let Some(t) = edit.target_ms {
            next.target_ms = valid_target(t)?;
        }
        match edit.done {
            Some(true) if self.done_ms.is_none() => next.done_ms = Some(now_ms),
            Some(false) => next.done_ms = None,
            _ => {}
        }
        Ok(next)
    }
}

fn valid_title(title: &str) -> Result<String, BoardError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(BoardError::Invalid(
            "a milestone needs a title of 1 to 200 characters",
        ));
    }
    Ok(title.to_owned())
}

fn valid_target(ms: i64) -> Result<i64, BoardError> {
    if ms <= 0 {
        return Err(BoardError::Invalid(
            "target_ms must be a positive Unix time in ms",
        ));
    }
    Ok(ms)
}

/// Persistence for milestones.
pub trait MilestoneStore: Send + Sync {
    /// Every milestone, in no particular order.
    ///
    /// # Errors
    /// If the store fails.
    fn milestones(&self) -> Result<Vec<Milestone>, BoardError>;
    /// One milestone.
    ///
    /// # Errors
    /// If the store fails.
    fn milestone(&self, id: Uuid) -> Result<Option<Milestone>, BoardError>;
    /// Record a new milestone; returns the version stored.
    ///
    /// # Errors
    /// [`BoardError::Conflict`] if its id exists, or if the store fails.
    fn add_milestone(&self, milestone: &Milestone) -> Result<u64, BoardError>;
    /// Record a milestone's new state, from `current`, only if it is still at `current.rev`
    /// (a compare-and-swap); returns the version stored.
    ///
    /// # Errors
    /// [`BoardError::Conflict`] if it moved since `current` was read, [`BoardError::NotFound`]
    /// if it does not exist, or if the store fails.
    fn save_milestone(&self, current: &Milestone, next: &Milestone) -> Result<u64, BoardError>;
}

/// Milestones in memory, for a board without rusty_tick and for tests.
#[derive(Default)]
pub struct MemMilestones(Mutex<BTreeMap<Uuid, Milestone>>);

impl MemMilestones {
    fn with<R>(&self, f: impl FnOnce(&mut BTreeMap<Uuid, Milestone>) -> R) -> R {
        f(&mut self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl MilestoneStore for MemMilestones {
    fn milestones(&self) -> Result<Vec<Milestone>, BoardError> {
        Ok(self.with(|m| m.values().cloned().collect()))
    }

    fn milestone(&self, id: Uuid) -> Result<Option<Milestone>, BoardError> {
        Ok(self.with(|m| m.get(&id).cloned()))
    }

    fn add_milestone(&self, milestone: &Milestone) -> Result<u64, BoardError> {
        self.with(|m| match m.contains_key(&milestone.id) {
            true => Err(BoardError::Conflict),
            false => {
                m.insert(
                    milestone.id,
                    Milestone {
                        rev: 1,
                        ..milestone.clone()
                    },
                );
                Ok(1)
            }
        })
    }

    fn save_milestone(&self, current: &Milestone, next: &Milestone) -> Result<u64, BoardError> {
        self.with(|m| match m.get_mut(&current.id) {
            Some(slot) if slot.rev == current.rev => {
                let rev = slot.rev + 1;
                *slot = Milestone {
                    rev,
                    ..next.clone()
                };
                Ok(rev)
            }
            Some(_) => Err(BoardError::Conflict),
            None => Err(BoardError::NotFound(current.id)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_milestone_needs_a_title_and_a_positive_target() {
        assert!(Milestone::new("rl", "  Beta  ", 10).is_ok_and(|m| m.title == "Beta"));
        assert!(Milestone::new("rl", " ", 10).is_err());
        assert!(Milestone::new("rl", &"x".repeat(MAX_TITLE + 1), 10).is_err());
        assert!(Milestone::new("rl", "Beta", 0).is_err());
    }

    #[test]
    fn closing_stamps_the_time_once_and_reopening_clears_it() {
        let m = Milestone::new("rl", "Beta", 10).expect("new");
        let close = MilestoneEdit {
            done: Some(true),
            ..MilestoneEdit::default()
        };
        let closed = m.edited(&close, 50).expect("close");
        assert_eq!(closed.done_ms, Some(50));
        assert_eq!(
            closed.edited(&close, 90).expect("again").done_ms,
            Some(50),
            "closing twice keeps the first time"
        );
        let reopen = MilestoneEdit {
            done: Some(false),
            ..MilestoneEdit::default()
        };
        assert_eq!(closed.edited(&reopen, 99).expect("reopen").done_ms, None);
        assert!(m.edited(&MilestoneEdit::default(), 1).is_err());
    }

    #[test]
    fn a_save_from_a_stale_read_is_refused() {
        let store = MemMilestones::default();
        let m = Milestone::new("rl", "Beta", 10).expect("new");
        let read = Milestone {
            rev: store.add_milestone(&m).expect("add"),
            ..m
        };
        let rename = |t: &str| MilestoneEdit {
            title: Some(t.into()),
            ..MilestoneEdit::default()
        };
        let first = read.edited(&rename("One"), 1).expect("edit");
        assert_eq!(store.save_milestone(&read, &first).expect("save"), 2);
        let late = read.edited(&rename("Two"), 2).expect("edit");
        assert!(matches!(
            store.save_milestone(&read, &late),
            Err(BoardError::Conflict)
        ));
        assert_eq!(
            store.milestone(read.id).expect("get").map(|m| m.title),
            Some("One".into())
        );
    }
}
