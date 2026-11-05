//! Orchestrator persistence (ADR-0010): the plan, board, and ledger of one
//! goal saved as a single snapshot record in the embedded
//! `rusty_multimodal_db` engine, so a run that stops blocked in one process
//! resumes in the next.
//!
//! One record prevents plan, board, and ledger payloads from splitting across
//! versions. A failed replacement can nevertheless be recovered with its new
//! payload and the preceding scannable revision. The revision is the persisted
//! scannable checkpoint generation; it may lag a recovered replacement and does
//! not prove how many saves returned successfully. The domain types are rebuilt
//! on load through their own
//! public API (`Plan::add` and the lifecycle transitions, `Board::append`,
//! `Ledger::from_counts`), so every invariant `orch-core` enforces is
//! re-checked and a snapshot that no longer satisfies them is refused as
//! corrupt rather than trusted.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod assignments;
mod hunts;
mod lens_records;
mod lens_states;
mod meters;
mod probes;
mod record;
mod taints;
mod watch;

use std::fmt;
use std::path::{Path, PathBuf};

use orch_core::board::Board;
use orch_core::factory::Factory;
use orch_core::task::Plan;
use orch_core::GoalId;
use orch_dispatch::Ledger;
use orch_lens::meter::Snapshot;
use orch_lens::saved::Saved;
use orch_lens::taint::Taints;
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use rusty_multimodal_db_engine::durability::record_blob::Fnv1a64;
use rusty_multimodal_db_engine::durability::DurabilityError;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::GenericMmapStore;

use assignments::{AssignmentRecord, LegacyAssignmentRecord};
use hunts::HuntRecord;
use lens_records::LensRecordRecord;
use lens_states::LensStateRecord;
use meters::MeterRecord;
use orch_lens::alien::{HuntRow, ProbeRow};
use orch_lens::record::Review;
use probes::ProbeRecord;
use record::{ById, GoalRecord, Revision};
use taints::TaintRecord;
use watch::WatchRecord;
pub use watch::{PendingPost, WatchFinding, WatchSlot};

/// The store's files inside its directory.
const LOCK_FILE: &str = "lock";
const GOALS_FILE: &str = "goals.mmap";
/// v1 assignment records, read for goals not saved since leases.
const LEGACY_ASSIGNMENTS_FILE: &str = "assignments.mmap";
const ASSIGNMENTS_FILE: &str = "assignments-v2.mmap";
/// Lens experiment meters (#594).
const METERS_FILE: &str = "meters.mmap";
/// BBP tasks' taint tables (#598).
const TAINTS_FILE: &str = "taints.mmap";
/// Alien-lens probe rows (#708).
const PROBES_FILE: &str = "probes.mmap";
/// Completed alien-lens hunts, with or without probes (#708).
const HUNTS_FILE: &str = "hunts.mmap";
/// Lens scheduler memory, one record per BBP board (#594).
const LENS_STATES_FILE: &str = "lens-states.mmap";
/// Watched findings (#599, `watch_finding`).
const WATCH_FILE: &str = "watch.mmap";
/// Blind review jobs for lens track records (#599, `lens_record`).
const LENS_RECORDS_FILE: &str = "lens-records.mmap";

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
    /// A read-only reader found no such store file; nothing was made.
    Missing(PathBuf),
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
            Self::Missing(path) => write!(f, "{} does not exist", path.display()),
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

/// One data directory, locked for this process, holding goal snapshots and
/// each goal's assignment record.
pub struct Store {
    goals: GenericMmapStore<GoalRecord, ById, Revision>,
    assignments: GenericMmapStore<AssignmentRecord, ById, Revision>,
    legacy: Option<GenericMmapStore<LegacyAssignmentRecord, ById, Revision>>,
    meters: GenericMmapStore<MeterRecord, ById, Revision>,
    taints: GenericMmapStore<TaintRecord, ById, Revision>,
    probes: GenericMmapStore<ProbeRecord, ById, Revision>,
    hunts: GenericMmapStore<HuntRecord, ById, Revision>,
    lens_states: GenericMmapStore<LensStateRecord, ById, Revision>,
    watch: GenericMmapStore<WatchRecord, ById, Revision>,
    lens_records: GenericMmapStore<LensRecordRecord, ById, Revision>,
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
        let path = dir.join(ASSIGNMENTS_FILE);
        let assignments = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(METERS_FILE);
        let meters = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(TAINTS_FILE);
        let taints = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(PROBES_FILE);
        let probes = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(HUNTS_FILE);
        let hunts = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(LENS_STATES_FILE);
        let lens_states = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(WATCH_FILE);
        let watch = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(LENS_RECORDS_FILE);
        let lens_records = if path.exists() {
            GenericMmapStore::open_portable(&path)?
        } else {
            GenericMmapStore::create(Vec::new(), &path)?
        };
        let path = dir.join(LEGACY_ASSIGNMENTS_FILE);
        let legacy = if path.exists() {
            Some(GenericMmapStore::open_portable(&path)?)
        } else {
            None
        };
        Ok(Self {
            goals,
            assignments,
            legacy,
            meters,
            taints,
            probes,
            hunts,
            lens_states,
            watch,
            lens_records,
            _lock: lock,
        })
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

    /// The persisted scannable checkpoint generation, or zero.
    ///
    /// Recovery after a failed replacement may expose the replacement's
    /// aggregate payload with the preceding generation, while recovery after a
    /// failed initial insert can synthesize generation one from the durable log.
    /// It therefore does not prove how many calls to [`Store::save`] returned
    /// successfully.
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

impl Store {
    /// The assignment record of `goal`, or an empty factory when none was
    /// saved.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the record does not rebuild through
    /// [`Factory::restore`].
    pub fn load_factory(&self, goal: GoalId) -> Result<Factory, StoreError> {
        self.assignment_record(goal)
            .map_or_else(|| Ok(Factory::default()), AssignmentRecord::into_factory)
    }

    /// The v2 record of `goal`, or its v1 record read as unleased.
    fn assignment_record(&self, goal: GoalId) -> Option<AssignmentRecord> {
        let key = record::key(goal);
        self.assignments.get(key).or_else(|| {
            self.legacy
                .as_ref()
                .and_then(|legacy| legacy.get(key))
                .map(AssignmentRecord::from)
        })
    }

    /// Save `factory` as the assignment record of `goal`. A successful return
    /// means it is durable, so a generation taken over in it may be used.
    ///
    /// # Errors
    ///
    /// [`StoreError::Engine`] when the write fails.
    pub fn save_factory(&mut self, goal: GoalId, factory: &Factory) -> Result<(), StoreError> {
        let first = self.assignments.get(record::key(goal)).is_none();
        let revision = self
            .assignment_record(goal)
            .map_or(0, |r| r.revision as u64)
            + 1;
        let record = AssignmentRecord::from_factory(goal, factory, revision);
        if first {
            self.assignments
                .insert(record)
                .map_err(|e| StoreError::Engine(engine_error(e)))
        } else {
            self.assignments
                .replace(record)
                .map_err(|e| StoreError::Engine(engine_error(e)))
        }
    }
}

impl Store {
    /// The saved meter of lens experiment `experiment`, or `None` when
    /// none was saved. Restore it with `orch_lens::meter::Meter::restore`,
    /// which charges every reservation still held.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the record under its key names another
    /// experiment or lists a task twice.
    pub fn load_meter(&self, experiment: &str) -> Result<Option<Snapshot>, StoreError> {
        self.meters
            .get(meters::key(experiment))
            .map(|r| r.into_snapshot(experiment))
            .transpose()
    }

    /// Save `snapshot` as the meter of `experiment`. A successful return
    /// means it is durable, so a reservation in it may be spent.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another
    /// experiment's meter; [`StoreError::Engine`] when the write fails.
    pub fn save_meter(&mut self, experiment: &str, snapshot: &Snapshot) -> Result<(), StoreError> {
        let existing = self.meters.get(meters::key(experiment));
        if let Some(r) = &existing {
            if r.experiment != experiment {
                return Err(StoreError::Corrupt(format!(
                    "meter key for {experiment:?} holds {:?}",
                    r.experiment
                )));
            }
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = MeterRecord::new(experiment, snapshot, revision);
        let written = if existing.is_none() {
            self.meters.insert(record).map_err(engine_error)
        } else {
            self.meters.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }

    /// The slot `slot` of `app`; empty when nothing was saved for it.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the record under its key names another
    /// app or slot.
    pub fn watch_slot(&self, app: &str, slot: &str) -> Result<WatchSlot, StoreError> {
        self.watch
            .get(watch::key(app, slot))
            .map_or(Ok(WatchSlot::default()), |r| r.into_slot(app, slot))
    }

    /// Save `row` as the slot `slot` of `app`. A successful return means it
    /// is durable.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key holds another app or slot;
    /// [`StoreError::Engine`] when the write fails.
    pub fn save_watch_slot(
        &mut self,
        app: &str,
        slot: &str,
        row: &WatchSlot,
    ) -> Result<(), StoreError> {
        let existing = self.watch.get(watch::key(app, slot));
        if let Some(r) = &existing {
            r.check(app, slot)?;
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = WatchRecord::new(app, slot, row, revision);
        let written = if existing.is_none() {
            self.watch.insert(record).map_err(engine_error)
        } else {
            self.watch.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }
}

impl Store {
    /// Save one alien-lens probe row, replacing an earlier save of the
    /// same probe (task, claim, candidate, site, mechanism).
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another probe;
    /// [`StoreError::Engine`] when the write fails.
    pub fn save_probe(&mut self, row: &ProbeRow) -> Result<(), StoreError> {
        let existing = self.probes.get(probes::key(row));
        if let Some(r) = &existing {
            if !r.is(row) {
                return Err(StoreError::Corrupt(
                    "probe key holds another probe".to_owned(),
                ));
            }
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = ProbeRecord::new(row, revision);
        let written = if existing.is_none() {
            self.probes.insert(record).map_err(engine_error)
        } else {
            self.probes.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }

    /// Save one completed hunt, replacing an earlier save of the same
    /// (task, claim, candidate). Save it even when it wrote no probe, so
    /// the §8 bar counts it.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another hunt;
    /// [`StoreError::Engine`] when the write fails.
    pub fn save_hunt(&mut self, row: &HuntRow) -> Result<(), StoreError> {
        let existing = self.hunts.get(hunts::key(row));
        if let Some(r) = &existing {
            if !r.is(row) {
                return Err(StoreError::Corrupt(
                    "hunt key holds another hunt".to_owned(),
                ));
            }
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = HuntRecord::new(row, revision);
        let written = if existing.is_none() {
            self.hunts.insert(record).map_err(engine_error)
        } else {
            self.hunts.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }

    /// Every saved hunt, of `task` when given, sorted by task, claim and
    /// candidate.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when a record does not check out.
    pub fn hunts(&self, task: Option<&str>) -> Result<Vec<HuntRow>, StoreError> {
        let mut rows = Vec::new();
        for id in self.hunts.all_ids() {
            let Some(record) = self.hunts.get(id) else {
                continue;
            };
            let row = record.into_row()?;
            if task.is_none_or(|t| t == row.task) {
                rows.push(row);
            }
        }
        rows.sort_by(|a, b| (&a.task, a.claim, a.candidate).cmp(&(&b.task, b.claim, b.candidate)));
        Ok(rows)
    }

    /// Every saved probe row, of `task` when given, sorted by task, claim,
    /// candidate, site and mechanism.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when a record does not check out.
    pub fn probes(&self, task: Option<&str>) -> Result<Vec<ProbeRow>, StoreError> {
        let mut rows = Vec::new();
        for id in self.probes.all_ids() {
            let Some(record) = self.probes.get(id) else {
                continue;
            };
            let row = record.into_row()?;
            if task.is_none_or(|t| t == row.task) {
                rows.push(row);
            }
        }
        rows.sort_by(|a, b| {
            (&a.task, a.claim, a.candidate, &a.site, &a.mechanism).cmp(&(
                &b.task,
                b.claim,
                b.candidate,
                &b.site,
                &b.mechanism,
            ))
        });
        Ok(rows)
    }
    /// The saved lens scheduler memory of BBP board `board`, or `None` when
    /// none was saved. A board names one task in one BBP store directory;
    /// the caller builds it (`rusty_orch` uses the canonical directory and
    /// the task id). Rebuild it with `orch_lens::schedule::State::restore`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the record under its key names another
    /// board.
    pub fn load_lens_state(&self, board: &str) -> Result<Option<Saved>, StoreError> {
        self.lens_states
            .get(lens_states::key(board))
            .map(|r| r.into_saved(board))
            .transpose()
    }

    /// Save `saved` as the lens scheduler memory of `board`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another board's
    /// memory; [`StoreError::Engine`] when the write fails.
    pub fn save_lens_state(&mut self, board: &str, saved: &Saved) -> Result<(), StoreError> {
        let existing = self.lens_states.get(lens_states::key(board));
        if let Some(r) = &existing {
            if r.board != board {
                return Err(StoreError::Corrupt(format!(
                    "lens state key for {board:?} holds {:?}",
                    r.board
                )));
            }
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = LensStateRecord::new(board, saved, revision);
        let written = if existing.is_none() {
            self.lens_states.insert(record).map_err(engine_error)
        } else {
            self.lens_states.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }
}

impl Store {
    /// Save one blind review job of `board` for its lens's track record. A
    /// job saved again (re-asked after a refusal, or rerun after a crash)
    /// keeps the union of its items, so a restart never drops or doubles one.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another job;
    /// [`StoreError::Engine`] when the write fails.
    pub fn save_lens_review(&mut self, board: &str, row: &Review) -> Result<(), StoreError> {
        let existing = self.lens_records.get(lens_records::key(board, row));
        if existing.as_ref().is_some_and(|r| !r.is(board, row)) {
            return Err(StoreError::Corrupt(
                "lens record key holds another job".to_owned(),
            ));
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let merged = match existing.clone() {
            Some(r) => merge(r.into_row()?, row),
            None => row.clone(),
        };
        let record = LensRecordRecord::new(board, &merged, revision);
        let written = if existing.is_none() {
            self.lens_records.insert(record).map_err(engine_error)
        } else {
            self.lens_records.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }

    /// Every blind review job saved for `board`, sorted by lens, claim,
    /// epoch and kind.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when a record does not check out.
    pub fn lens_reviews(&self, board: &str) -> Result<Vec<Review>, StoreError> {
        let ids = self.lens_records.all_ids();
        reviews(
            ids.into_iter().filter_map(|id| self.lens_records.get(id)),
            board,
        )
    }

    /// [`Store::lens_reviews`] for the store in `dir`, read without changing
    /// anything: it takes the directory's existing lock, making no file, and
    /// reads the lens record table's saved records and insert log as
    /// [`Store::open`] would recover them, writing nothing back.
    ///
    /// # Errors
    ///
    /// [`StoreError::Missing`] when `dir` has no lock or lens record file;
    /// [`StoreError::Locked`] when another process holds `dir`;
    /// [`StoreError::Lock`] when the lock cannot be opened;
    /// [`StoreError::Engine`] or [`StoreError::Corrupt`] when the table
    /// cannot be read.
    pub fn read_lens_reviews(dir: &Path, board: &str) -> Result<Vec<Review>, StoreError> {
        let _lock = DirLock::acquire_existing(dir, LOCK_FILE).map_err(|e| match e {
            DirLockError::Io(path, e) if e.kind() == std::io::ErrorKind::NotFound => {
                StoreError::Missing(path)
            }
            e => e.into(),
        })?;
        let path = dir.join(LENS_RECORDS_FILE);
        if !path.is_file() {
            return Err(StoreError::Missing(path));
        }
        let records =
            GenericMmapStore::<LensRecordRecord, ById, Revision>::read_portable_records(&path)?;
        reviews(records, board)
    }

    /// BBP task `task`'s taint table; empty when none was saved.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the record under its key names another
    /// task or an unknown class.
    pub fn load_taints(&self, task: &str) -> Result<Taints, StoreError> {
        self.taints
            .get(taints::key(task))
            .map_or(Ok(Taints::default()), |r| r.into_taints(task))
    }

    /// Save `taints` as task `task`'s table. Only ever grows: the caller
    /// loads, raises and saves under the store's lock.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the key already holds another task's
    /// table; [`StoreError::Engine`] when the write fails.
    pub fn save_taints(&mut self, task: &str, taints: &Taints) -> Result<(), StoreError> {
        let existing = self.taints.get(taints::key(task));
        if let Some(r) = &existing {
            if r.task != task {
                return Err(StoreError::Corrupt(format!(
                    "taint key for {task:?} holds {:?}",
                    r.task
                )));
            }
        }
        let revision = existing.as_ref().map_or(0, |r| r.revision as u64) + 1;
        let record = TaintRecord::new(task, taints, revision);
        let written = if existing.is_none() {
            self.taints.insert(record).map_err(engine_error)
        } else {
            self.taints.replace(record).map_err(engine_error)
        };
        written.map_err(StoreError::Engine)
    }
}

/// Fold the engine's typed write errors into its one durability error;
/// the id-shaped variants cannot happen after `revision` chose the path.
fn engine_error<E: fmt::Display>(e: E) -> DurabilityError {
    DurabilityError::Engine(e.to_string())
}

/// `row` saved over the job's stored row `old` (D92 write-ahead):
///
/// - A job rerun after a crash posts again: every item it posted is kept.
/// - A lost post stays unknown: `unsure` never clears.
/// - A write-ahead over a row still `pending` follows an attempt that
///   crashed or failed its save after it may have posted: unsure.
/// - The save after the posts clears `pending`.
fn merge(mut old: Review, row: &Review) -> Review {
    for m in &row.items {
        if !old.items.contains(m) {
            old.items.push(*m);
        }
    }
    old.unsure |= row.unsure || (row.pending && old.pending);
    old.pending = row.pending;
    old
}

/// Every blind review job in `records` saved for `board`, sorted by lens,
/// claim, epoch and kind.
fn reviews(
    records: impl IntoIterator<Item = LensRecordRecord>,
    board: &str,
) -> Result<Vec<Review>, StoreError> {
    let mut rows = Vec::new();
    for record in records {
        if record.board() == board {
            rows.push(record.into_row()?);
        }
    }
    rows.sort_by(|a, b| {
        (&a.lens, a.claim, a.epoch, a.look_harder).cmp(&(&b.lens, b.claim, b.epoch, b.look_harder))
    });
    Ok(rows)
}
