//! The append-only blackboard agents read and write instead of messaging.
//!
//! Entries are never edited. To change one, append a new entry of the same
//! kind that `supersedes` it; [`Board::live`] hides the old one. Every
//! entry-to-entry pointer is checked on append, so the board has no
//! dangling references.

use std::collections::HashSet;
use std::fmt;
use std::mem::discriminant;

use crate::task::Agent;
use crate::{EntryId, GoalId, Ref, TaskId, Text};

/// Who wrote an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    Human,
    Agent(Agent),
}

/// How well-supported a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// Outcome of a cross-model review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    ChangesRequested,
}

/// What an entry is. Kind-specific data lives in the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Something learned; sources go in `refs`.
    Finding { confidence: Confidence },
    /// A settled choice. Supersede to change it.
    Decision,
    /// Taken as true without verification (best-effort runs).
    Assumption,
    /// Open question; a task may block on it.
    Question,
    /// Resolves a `Question`.
    Answer { to: EntryId },
    /// Pointer to a work product; `refs` must be non-empty.
    Artifact,
    /// Cross-model review of a task's output.
    Review { of: TaskId, verdict: Verdict },
}

/// An entry as submitted, before the board assigns its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    pub task: Option<TaskId>,
    pub author: Author,
    pub kind: EntryKind,
    /// Short statement. Detail belongs behind `refs`.
    pub body: Text,
    pub refs: Vec<Ref>,
    pub supersedes: Option<EntryId>,
}

/// A stored entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    id: EntryId,
    content: NewEntry,
}

impl Entry {
    /// Id, unique within its board.
    pub fn id(&self) -> EntryId {
        self.id
    }

    /// What was written.
    pub fn content(&self) -> &NewEntry {
        &self.content
    }
}

/// Why an append was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardError {
    UnknownEntry(EntryId),
    NotAQuestion(EntryId),
    KindMismatch(EntryId),
    AlreadySuperseded(EntryId),
    ArtifactWithoutRefs,
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntry(id) => write!(f, "{id} does not exist"),
            Self::NotAQuestion(id) => write!(f, "{id} is not a question"),
            Self::KindMismatch(id) => write!(
                f,
                "{id} can only be superseded by an entry of the same kind"
            ),
            Self::AlreadySuperseded(id) => write!(f, "{id} is already superseded"),
            Self::ArtifactWithoutRefs => f.write_str("an artifact must point at something"),
        }
    }
}

impl std::error::Error for BoardError {}

/// The shared, append-only log for one goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    goal: GoalId,
    entries: Vec<Entry>,
}

impl Board {
    /// An empty board for `goal`.
    pub fn new(goal: GoalId) -> Self {
        Self {
            goal,
            entries: Vec::new(),
        }
    }

    /// The goal this board serves.
    pub fn goal(&self) -> GoalId {
        self.goal
    }

    /// Full history, including superseded entries.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Look up an entry.
    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        let index = usize::try_from(id.get().checked_sub(1)?).ok()?;
        self.entries.get(index)
    }

    /// Validate and append; returns the new id.
    pub fn append(&mut self, new: NewEntry) -> Result<EntryId, BoardError> {
        self.check(&new)?;
        let id = EntryId::from_raw(self.entries.len() as u64 + 1);
        self.entries.push(Entry { id, content: new });
        Ok(id)
    }

    /// True if a later entry supersedes `id`.
    pub fn is_superseded(&self, id: EntryId) -> bool {
        self.entries
            .iter()
            .any(|e| e.content.supersedes == Some(id))
    }

    /// Current view: every entry not superseded. This is what agents read.
    pub fn live(&self) -> impl Iterator<Item = &Entry> {
        let dead: HashSet<EntryId> = self
            .entries
            .iter()
            .filter_map(|e| e.content.supersedes)
            .collect();
        self.entries.iter().filter(move |e| !dead.contains(&e.id))
    }

    /// Live questions with no live answer.
    pub fn open_questions(&self) -> Vec<&Entry> {
        let answered: HashSet<EntryId> = self
            .live()
            .filter_map(|e| match e.content.kind {
                EntryKind::Answer { to } => Some(to),
                _ => None,
            })
            .collect();
        self.live()
            .filter(|e| e.content.kind == EntryKind::Question && !answered.contains(&e.id))
            .collect()
    }

    fn check(&self, new: &NewEntry) -> Result<(), BoardError> {
        if new.kind == EntryKind::Artifact && new.refs.is_empty() {
            return Err(BoardError::ArtifactWithoutRefs);
        }
        for r in &new.refs {
            if let Ref::Entry(id) = r {
                self.entry(*id)?;
            }
        }
        if let EntryKind::Answer { to } = new.kind {
            if self.entry(to)?.content.kind != EntryKind::Question {
                return Err(BoardError::NotAQuestion(to));
            }
        }
        if let Some(old) = new.supersedes {
            self.check_supersede(old, new.kind)?;
        }
        Ok(())
    }

    fn check_supersede(&self, old: EntryId, kind: EntryKind) -> Result<(), BoardError> {
        if discriminant(&self.entry(old)?.content.kind) != discriminant(&kind) {
            return Err(BoardError::KindMismatch(old));
        }
        if self.is_superseded(old) {
            return Err(BoardError::AlreadySuperseded(old));
        }
        Ok(())
    }

    fn entry(&self, id: EntryId) -> Result<&Entry, BoardError> {
        self.get(id).ok_or(BoardError::UnknownEntry(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: EntryKind) -> NewEntry {
        NewEntry {
            task: Some(TaskId::from_raw(1)),
            author: Author::Agent(Agent::Gemini),
            kind,
            body: Text::new("note").expect("non-blank"),
            refs: vec![],
            supersedes: None,
        }
    }

    fn board() -> Board {
        Board::new(GoalId::from_raw(1))
    }

    #[test]
    fn append_assigns_sequential_ids() {
        let mut b = board();
        assert_eq!(
            b.append(entry(EntryKind::Decision)),
            Ok(EntryId::from_raw(1))
        );
        assert_eq!(
            b.append(entry(EntryKind::Assumption)),
            Ok(EntryId::from_raw(2))
        );
    }

    #[test]
    fn answer_must_target_a_question() {
        let mut b = board();
        let decision = b.append(entry(EntryKind::Decision)).expect("append");
        assert_eq!(
            b.append(entry(EntryKind::Answer { to: decision })),
            Err(BoardError::NotAQuestion(decision))
        );
        let ghost = EntryId::from_raw(9);
        assert_eq!(
            b.append(entry(EntryKind::Answer { to: ghost })),
            Err(BoardError::UnknownEntry(ghost))
        );
    }

    #[test]
    fn supersede_hides_old_entry_from_live_view() {
        let mut b = board();
        let old = b.append(entry(EntryKind::Decision)).expect("append");
        let new = b
            .append(NewEntry {
                supersedes: Some(old),
                ..entry(EntryKind::Decision)
            })
            .expect("append");
        assert_eq!(b.live().map(Entry::id).collect::<Vec<_>>(), vec![new]);
        assert_eq!(b.entries().len(), 2);
    }

    #[test]
    fn supersede_rejects_kind_change_and_forks() {
        let mut b = board();
        let old = b.append(entry(EntryKind::Decision)).expect("append");
        assert_eq!(
            b.append(NewEntry {
                supersedes: Some(old),
                ..entry(EntryKind::Assumption)
            }),
            Err(BoardError::KindMismatch(old))
        );
        b.append(NewEntry {
            supersedes: Some(old),
            ..entry(EntryKind::Decision)
        })
        .expect("append");
        assert_eq!(
            b.append(NewEntry {
                supersedes: Some(old),
                ..entry(EntryKind::Decision)
            }),
            Err(BoardError::AlreadySuperseded(old))
        );
    }

    #[test]
    fn finding_may_be_superseded_with_new_confidence() {
        let mut b = board();
        let old = b
            .append(entry(EntryKind::Finding {
                confidence: Confidence::Low,
            }))
            .expect("append");
        let revised = NewEntry {
            supersedes: Some(old),
            ..entry(EntryKind::Finding {
                confidence: Confidence::High,
            })
        };
        assert!(b.append(revised).is_ok());
    }

    #[test]
    fn dangling_entry_ref_rejected() {
        let ghost = EntryId::from_raw(3);
        let new = NewEntry {
            refs: vec![Ref::Entry(ghost)],
            ..entry(EntryKind::Decision)
        };
        assert_eq!(board().append(new), Err(BoardError::UnknownEntry(ghost)));
    }

    #[test]
    fn artifact_requires_refs() {
        let mut b = board();
        assert_eq!(
            b.append(entry(EntryKind::Artifact)),
            Err(BoardError::ArtifactWithoutRefs)
        );
        let commit = Ref::Commit(Text::new("a1b2c3d").expect("non-blank"));
        assert!(b
            .append(NewEntry {
                refs: vec![commit],
                ..entry(EntryKind::Artifact)
            })
            .is_ok());
    }

    #[test]
    fn open_questions_excludes_answered() {
        let mut b = board();
        let q1 = b.append(entry(EntryKind::Question)).expect("append");
        let q2 = b.append(entry(EntryKind::Question)).expect("append");
        b.append(entry(EntryKind::Answer { to: q1 }))
            .expect("append");
        assert_eq!(
            b.open_questions()
                .iter()
                .map(|e| e.id())
                .collect::<Vec<_>>(),
            vec![q2]
        );
    }
}
