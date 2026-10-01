use crate::{EntryId, Text};

/// A pointer to context. Agents pass refs, never pasted content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    /// Repo-relative file path.
    Path(Text),
    /// Commit hash or branch name.
    Commit(Text),
    /// External source.
    Url(Text),
    /// Another blackboard entry on the same board.
    Entry(EntryId),
}
