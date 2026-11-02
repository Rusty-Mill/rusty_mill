//! What the runner needs from the outside: the board and the workspace.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use rusty_mill_board::domain::{Capability, Card, Column};
use rusty_mill_board::workflow::Workflow;
use uuid::Uuid;

use crate::config::Stage;

/// A refused or failed board call. `status` is the HTTP status, 0 for transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// HTTP status; 0 when the board was unreachable.
    pub status: u16,
    /// What the board said.
    pub message: String,
}

impl ApiError {
    /// Another actor got there first or the move does not exist: skip, don't fail.
    pub fn is_contention(&self) -> bool {
        matches!(self.status, 409 | 422)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "board {}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Role name -> capabilities, as the board reports it.
pub type Roles = BTreeMap<String, BTreeSet<Capability>>;

/// The board, as one actor acting in a role.
pub trait BoardApi: Sync {
    /// Cards in a column.
    fn list(&self, column: &Column) -> Result<Vec<Card>, ApiError>;
    /// One card.
    fn get(&self, id: Uuid) -> Result<Card, ApiError>;
    /// The board's columns and the moves between them.
    fn workflow(&self) -> Result<Workflow, ApiError>;
    /// The board's roles.
    fn roles(&self) -> Result<Roles, ApiError>;
    /// How many projects the board has (whatever this credential may see).
    fn project_count(&self) -> Result<usize, ApiError>;
    /// The `files` that `project` does not own, by the board's whole registry. Also the check
    /// that `project` exists and this credential may work in it.
    fn foreign(&self, project: &str, files: &[String]) -> Result<Vec<String>, ApiError>;
    /// Move a card as `role`.
    fn move_card(&self, role: &str, id: Uuid, to: &Column) -> Result<Card, ApiError>;
    /// Claim a card as `role`.
    fn claim(&self, role: &str, id: Uuid) -> Result<Card, ApiError>;
    /// Release a claim as `role`.
    fn release(&self, role: &str, id: Uuid) -> Result<Card, ApiError>;
    /// Comment as `role`.
    fn comment(&self, role: &str, id: Uuid, text: &str) -> Result<Card, ApiError>;
}

/// How one command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Exited 0.
    pub ok: bool,
    /// One line: how it ended and how long it took.
    pub summary: String,
    /// The end of its output.
    pub tail: String,
}

/// Where cards are worked: a directory per card, a command run in it.
pub trait Work: Sync {
    /// The card's working directory, created on first use.
    ///
    /// # Errors
    /// A message if it cannot be created.
    fn prepare(&self, card: &Card) -> Result<PathBuf, String>;
    /// Run `argv` for `stage` in `dir`, confined.
    ///
    /// # Errors
    /// A message if the command could not be started or confined at all;
    /// a command that runs and fails is an [`Outcome`].
    fn run(
        &self,
        stage: &Stage,
        argv: &[String],
        dir: &Path,
        env: &[(String, String)],
    ) -> Result<Outcome, String>;
    /// Every repository path with uncommitted changes in `dir` (`/`-separated, from the
    /// repository root; both sides of a rename).
    ///
    /// # Errors
    /// A message if the state could not be read.
    fn changed(&self, dir: &Path) -> Result<Vec<String>, String>;
    /// Throw away everything uncommitted in `dir`: the stage's work is not wanted.
    ///
    /// # Errors
    /// A message if the directory could not be restored.
    fn discard(&self, dir: &Path) -> Result<(), String>;
    /// Commit what the stage changed. `Ok(false)` when there was nothing to commit.
    ///
    /// # Errors
    /// A message if the commit failed.
    fn commit(&self, dir: &Path, message: &str) -> Result<bool, String>;
}
