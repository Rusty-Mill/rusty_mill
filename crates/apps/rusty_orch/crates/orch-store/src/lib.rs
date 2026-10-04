//! Orchestrator persistence (ADR-0010): the plan, board, and ledger of one
//! goal saved as a single snapshot record in the embedded
//! `rusty_multimodal_db` engine, so a run that stops blocked in one process
//! resumes in the next.
//!
//! One record prevents plan, board, and ledger payloads from splitting across
//! versions. A failed replacement can nevertheless be recovered with its new
//! payload and the preceding scannable revision; only a successful return is
//! a durability guarantee. The domain types are rebuilt on load through their own
//! public API (`Plan::add` and the lifecycle transitions, `Board::append`,
//! `Ledger::from_counts`), so every invariant `orch-core` enforces is
//! re-checked and a snapshot that no longer satisfies them is refused as
//! corrupt rather than trusted.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod record;

use std::fmt;
use std::path::{Path, PathBuf};

use orch_core::board::Board;
use orch_core::task::Plan;
use orch_core::GoalId;
use orch_dispatch::Ledger;
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::generic::query::GetById;
use rusty_multimodal_db_engine::generic::GenericMmapStore;

use record::{ById, GoalRecord, Revision};

/// The store's files inside its directory.
const LOCK_FILE: &str = "lock";
const GOALS_FILE: &str = "goals.mmap";

/// Everything a later process needs to continue a goal. The goal contract
/// itself is not stored: `fingerprint` ties the snapshot to the goal file
/// it was built from, and the caller re-reads that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalState {
    /// Identifies the goal file's bytes; see [`fingerprint`].
    pub fingerprint: u64,
    pub plan: Plan,
    pub board: Board,
    pub ledger: Ledger,
}

impl GoalState {
    /// The goal both aggregates belong to.
    pub fn goal(&self) -> GoalId {
        self.plan.goal()
    }
}

/// Why the store could not do what was asked.
#[derive(Debug)]
pub enum StoreError {
    /// Another process has the directory open.
    Locked(PathBuf),
    /// The directory could not be locked or created.
    Lock(PathBuf, std::io::Error),
    /// The engine refused a read or write.
    Engine(DurabilityError),
    /// A saved snapshot does not rebuild into valid domain state.
    Corrupt(String),
    /// The plan and board handed to `save` belong to different goals.
    GoalMismatch { plan: GoalId, board: GoalId },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked(dir) => write!(f, "{} is in use by another process", dir.display()),
            Self::Lock(path, e) => write!(f, "cannot lock {}: {e}", path.display()),
            Self::Engine(e) => write!(f, "store: {e}"),
            Self::Corrupt(why) => write!(f, "saved state is corrupt: {why}"),
            Self::GoalMismatch { plan, board } => {
                write!(f, "plan is for {plan} but board is for {board}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<DurabilityError> for StoreError {
    fn from(e: DurabilityError) -> Self {
        Self::Engine(e)
    }
}

impl From<DirLockError> for StoreError {
    fn from(e: DirLockError) -> Self {
        match e {
            DirLockError::Held(dir) => Self::Locked(dir),
            DirLockError::Io(path, e) => Self::Lock(path, e),
        }
    }
}

/// A stable hash of the goal file's text, stored with the snapshot so a
/// changed file is refused instead of resumed against a plan built from
/// another.
pub fn fingerprint(goal_json: &str) -> u64 {
    let mut hash = Fnv1a64::new();
    hash.update(goal_json.as_bytes());
    hash.finish()
}

/// One data directory, locked for this process, holding goal snapshots.
pub struct Store {
    goals: GenericMmapStore<GoalRecord, ById, Revision>,
    _lock: DirLock,
}

impl Store {
    /// Open the store in `dir`, creating the directory and its files when
    /// they do not exist yet.
    ///
    /// # Errors
    ///
    /// [`StoreError::Locked`] when another process holds `dir`;
    /// [`StoreError::Lock`] or [`StoreError::Engine`] when the files
    /// cannot be made or read.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        let lock = DirLock::acquire(dir, LOCK_FILE)?;
        let path = dir.join(GOALS_FILE);
        let goals = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        Ok(Self { goals, _lock: lock })
    }

    /// The saved state of `goal`, or `None` when nothing was saved for it.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the snapshot does not rebuild through
    /// the domain's own constructors.
    pub fn load(&self, goal: GoalId) -> Result<Option<GoalState>, StoreError> {
        self.goals
            .get(record::key(goal))
            .map(GoalRecord::into_state)
            .transpose()
    }

    /// The last successfully completed checkpoint generation, or zero.
    ///
    /// Recovery after a failed replacement may expose the replacement's
    /// aggregate payload with the preceding generation, because the engine
    /// logs the record before rewriting its separate scannable slot.
    pub fn revision(&self, goal: GoalId) -> u64 {
        self.goals
            .get(record::key(goal))
            .map_or(0, |r| r.revision as u64)
    }

    /// Save `state`, replacing the previous snapshot of its goal. A successful
    /// return means the aggregate and its new revision are durable. After an
    /// error, reopening may recover either the prior snapshot or the new
    /// aggregate payload with the prior revision; callers must not infer
    /// durability from the error alone.
    ///
    /// # Errors
    ///
    /// [`StoreError::GoalMismatch`] when the plan and board disagree on
    /// their goal; [`StoreError::Engine`] when the write fails.
    pub fn save(&mut self, state: &GoalState) -> Result<(), StoreError> {
        let (plan, board) = (state.plan.goal(), state.board.goal());
        if plan != board {
            return Err(StoreError::GoalMismatch { plan, board });
        }
        let revision = self.revision(plan) + 1;
        let record = GoalRecord::from_state(state, revision);
        if revision == 1 {
            self.goals
                .insert(record)
                .map_err(|e| StoreError::Engine(engine_error(e)))
        } else {
            self.goals
                .replace(record)
                .map_err(|e| StoreError::Engine(engine_error(e)))
        }
    }
}

/// Fold the engine's typed write errors into its one durability error;
/// the id-shaped variants cannot happen after `revision` chose the path.
fn engine_error<E: fmt::Display>(e: E) -> DurabilityError {
    DurabilityError::Engine(e.to_string())
}
