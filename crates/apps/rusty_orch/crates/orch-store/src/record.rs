//! The on-disk shape of a goal snapshot, and the two conversions.
//!
//! Every type here is encoded by the engine's bincode codec: field order
//! and enum variant order are the format. Adding a variant at the end or a
//! field at the end of a struct keeps old snapshots readable; reordering
//! does not. The `SCHEMA_TAG` is checked before any byte is decoded.

use std::num::NonZeroU32;

use orch_core::board::{Author, Board, Confidence, EntryKind, NewEntry, Verdict};
use orch_core::task::{Agent, Plan, Role, TaskSpec, TaskState};
use orch_core::{EntryId, GoalId, Ref, TaskId, Text};
use orch_dispatch::Ledger;
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use serde::{Deserialize, Serialize};

use crate::{GoalState, StoreError};

/// The equality index: the goal id, so the record is found by key.
pub struct ById;
/// The mmap slot: the save count, a cheap monotonic field the engine
/// requires one of.
pub struct Revision;

/// Engine key for a goal. Orch ids are small 1-based counters, so the
/// `u64 -> i64` cast is lossless.
pub fn key(goal: GoalId) -> i64 {
    goal.get() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalRecord {
    id: i64,
    pub revision: i64,
    fingerprint: u64,
    tasks: Vec<TaskRow>,
    entries: Vec<EntryRow>,
    goal_calls: u32,
    task_calls: Vec<(u64, u32)>,
}

impl Record for GoalRecord {
    type Id = i64;
    fn id(&self) -> i64 {
        self.id
    }
}

impl SchemaTag for GoalRecord {
    const SCHEMA_TAG: &'static str = "rusty_orch::GoalRecord";
}

impl IndexedField<ById> for GoalRecord {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.id
    }
}

impl ScannableField<Revision> for GoalRecord {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.revision
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.revision = value;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskRow {
    id: u64,
    role: RoleRow,
    instruction: String,
    acceptance: Vec<String>,
    refs: Vec<RefRow>,
    depends_on: Vec<u64>,
    max_calls: u32,
    state: StateRow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum RoleRow {
    Research,
    Design,
    Implement,
    Triage,
    Review { target: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AgentRow {
    Claude,
    Codex,
    Gemini,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum StateRow {
    Pending,
    Running { agent: AgentRow },
    Blocked { agent: AgentRow, question: u64 },
    Done { agent: AgentRow, outputs: Vec<u64> },
    Failed { agent: AgentRow, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum RefRow {
    Path(String),
    Commit(String),
    Url(String),
    Entry(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntryRow {
    id: u64,
    task: Option<u64>,
    author: AuthorRow,
    kind: KindRow,
    body: String,
    refs: Vec<RefRow>,
    supersedes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AuthorRow {
    Human,
    Agent(AgentRow),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum ConfidenceRow {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum VerdictRow {
    Approve,
    ChangesRequested,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum KindRow {
    Finding { confidence: ConfidenceRow },
    Decision,
    Assumption,
    Question,
    Answer { to: u64 },
    Artifact,
    Review { of: u64, verdict: VerdictRow },
}

// ---- domain -> row ----------------------------------------------------

impl GoalRecord {
    pub fn from_state(state: &GoalState, revision: u64) -> Self {
        let plan = &state.plan;
        let tasks = plan.tasks().iter().map(task_row).collect();
        let entries = state.board.entries().iter().map(entry_row).collect();
        let task_calls = plan
            .tasks()
            .iter()
            .map(|t| (t.id().get(), state.ledger.task_calls(t.id())))
            .filter(|(_, n)| *n > 0)
            .collect();
        Self {
            id: key(plan.goal()),
            revision: revision as i64,
            fingerprint: state.fingerprint,
            tasks,
            entries,
            goal_calls: state.ledger.calls(),
            task_calls,
        }
    }
}

fn task_row(t: &orch_core::task::Task) -> TaskRow {
    let spec = t.spec();
    TaskRow {
        id: t.id().get(),
        role: match spec.role {
            Role::Research => RoleRow::Research,
            Role::Design => RoleRow::Design,
            Role::Implement => RoleRow::Implement,
            Role::Triage => RoleRow::Triage,
            Role::Review { target } => RoleRow::Review {
                target: target.get(),
            },
        },
        instruction: spec.instruction.as_str().to_owned(),
        acceptance: spec.acceptance.iter().map(text).collect(),
        refs: spec.refs.iter().map(ref_row).collect(),
        depends_on: spec.depends_on.iter().map(|d| d.get()).collect(),
        max_calls: spec.max_calls.get(),
        state: match t.state() {
            TaskState::Pending => StateRow::Pending,
            TaskState::Running { agent } => StateRow::Running {
                agent: agent_row(*agent),
            },
            TaskState::Blocked { agent, question } => StateRow::Blocked {
                agent: agent_row(*agent),
                question: question.get(),
            },
            TaskState::Done { agent, outputs } => StateRow::Done {
                agent: agent_row(*agent),
                outputs: outputs.iter().map(|o| o.get()).collect(),
            },
            TaskState::Failed { agent, reason } => StateRow::Failed {
                agent: agent_row(*agent),
                reason: text(reason),
            },
        },
    }
}

fn entry_row(e: &orch_core::board::Entry) -> EntryRow {
    let c = e.content();
    EntryRow {
        id: e.id().get(),
        task: c.task.map(|t| t.get()),
        author: match c.author {
            Author::Human => AuthorRow::Human,
            Author::Agent(a) => AuthorRow::Agent(agent_row(a)),
        },
        kind: match c.kind {
            EntryKind::Finding { confidence } => KindRow::Finding {
                confidence: match confidence {
                    Confidence::Low => ConfidenceRow::Low,
                    Confidence::Medium => ConfidenceRow::Medium,
                    Confidence::High => ConfidenceRow::High,
                },
            },
            EntryKind::Decision => KindRow::Decision,
            EntryKind::Assumption => KindRow::Assumption,
            EntryKind::Question => KindRow::Question,
            EntryKind::Answer { to } => KindRow::Answer { to: to.get() },
            EntryKind::Artifact => KindRow::Artifact,
            EntryKind::Review { of, verdict } => KindRow::Review {
                of: of.get(),
                verdict: match verdict {
                    Verdict::Approve => VerdictRow::Approve,
                    Verdict::ChangesRequested => VerdictRow::ChangesRequested,
                },
            },
        },
        body: text(&c.body),
        refs: c.refs.iter().map(ref_row).collect(),
        supersedes: c.supersedes.map(|s| s.get()),
    }
}

fn agent_row(a: Agent) -> AgentRow {
    match a {
        Agent::Claude => AgentRow::Claude,
        Agent::Codex => AgentRow::Codex,
        Agent::Gemini => AgentRow::Gemini,
        Agent::Local => AgentRow::Local,
    }
}

fn ref_row(r: &Ref) -> RefRow {
    match r {
        Ref::Path(t) => RefRow::Path(text(t)),
        Ref::Commit(t) => RefRow::Commit(text(t)),
        Ref::Url(t) => RefRow::Url(text(t)),
        Ref::Entry(id) => RefRow::Entry(id.get()),
    }
}

fn text(t: &Text) -> String {
    t.as_str().to_owned()
}

// ---- row -> domain ----------------------------------------------------

impl GoalRecord {
    /// Rebuild the domain state through its own constructors, so the
    /// snapshot is re-validated rather than trusted.
    pub fn into_state(self) -> Result<GoalState, StoreError> {
        let goal = GoalId::from_raw(self.id as u64);
        let plan = plan(goal, &self.tasks)?;
        let board = board(goal, &self.entries)?;
        let ledger = Ledger::from_counts(
            self.goal_calls,
            self.task_calls
                .iter()
                .map(|(id, n)| (TaskId::from_raw(*id), *n)),
        );
        Ok(GoalState {
            fingerprint: self.fingerprint,
            plan,
            board,
            ledger,
        })
    }
}

/// Add every card in id order, then replay each card's lifecycle. Cards
/// depend only on earlier ids, so by the time a card starts its
/// prerequisites already hold their final state.
fn plan(goal: GoalId, rows: &[TaskRow]) -> Result<Plan, StoreError> {
    let mut plan = Plan::new(goal);
    for row in rows {
        let id = plan.add(task_spec(row)?).map_err(corrupt)?;
        if id.get() != row.id {
            return Err(StoreError::Corrupt(format!(
                "task {} was saved as T-{}",
                id, row.id
            )));
        }
    }
    for row in rows {
        let id = TaskId::from_raw(row.id);
        match &row.state {
            StateRow::Pending => {}
            StateRow::Running { agent } => plan.start(id, agent_of(*agent)).map_err(corrupt)?,
            StateRow::Blocked { agent, question } => {
                plan.start(id, agent_of(*agent)).map_err(corrupt)?;
                plan.block(id, EntryId::from_raw(*question))
                    .map_err(corrupt)?;
            }
            StateRow::Done { agent, outputs } => {
                plan.start(id, agent_of(*agent)).map_err(corrupt)?;
                let outputs = outputs.iter().map(|o| EntryId::from_raw(*o)).collect();
                plan.complete(id, outputs).map_err(corrupt)?;
            }
            StateRow::Failed { agent, reason } => {
                plan.start(id, agent_of(*agent)).map_err(corrupt)?;
                plan.fail(id, text_of(reason)?).map_err(corrupt)?;
            }
        }
    }
    Ok(plan)
}

fn task_spec(row: &TaskRow) -> Result<TaskSpec, StoreError> {
    Ok(TaskSpec {
        role: match row.role {
            RoleRow::Research => Role::Research,
            RoleRow::Design => Role::Design,
            RoleRow::Implement => Role::Implement,
            RoleRow::Triage => Role::Triage,
            RoleRow::Review { target } => Role::Review {
                target: TaskId::from_raw(target),
            },
        },
        instruction: text_of(&row.instruction)?,
        acceptance: row
            .acceptance
            .iter()
            .map(|s| text_of(s))
            .collect::<Result<_, _>>()?,
        refs: row.refs.iter().map(ref_of).collect::<Result<_, _>>()?,
        depends_on: row
            .depends_on
            .iter()
            .map(|d| TaskId::from_raw(*d))
            .collect(),
        max_calls: NonZeroU32::new(row.max_calls)
            .ok_or_else(|| StoreError::Corrupt(format!("T-{} has max_calls 0", row.id)))?,
    })
}

/// Append every entry in id order; the board re-checks each one and
/// assigns the same id it did the first time.
fn board(goal: GoalId, rows: &[EntryRow]) -> Result<Board, StoreError> {
    let mut board = Board::new(goal);
    for row in rows {
        let id = board.append(new_entry(row)?).map_err(corrupt)?;
        if id.get() != row.id {
            return Err(StoreError::Corrupt(format!(
                "entry {} was saved as E-{}",
                id, row.id
            )));
        }
    }
    Ok(board)
}

fn new_entry(row: &EntryRow) -> Result<NewEntry, StoreError> {
    Ok(NewEntry {
        task: row.task.map(TaskId::from_raw),
        author: match row.author {
            AuthorRow::Human => Author::Human,
            AuthorRow::Agent(a) => Author::Agent(agent_of(a)),
        },
        kind: match row.kind {
            KindRow::Finding { confidence } => EntryKind::Finding {
                confidence: match confidence {
                    ConfidenceRow::Low => Confidence::Low,
                    ConfidenceRow::Medium => Confidence::Medium,
                    ConfidenceRow::High => Confidence::High,
                },
            },
            KindRow::Decision => EntryKind::Decision,
            KindRow::Assumption => EntryKind::Assumption,
            KindRow::Question => EntryKind::Question,
            KindRow::Answer { to } => EntryKind::Answer {
                to: EntryId::from_raw(to),
            },
            KindRow::Artifact => EntryKind::Artifact,
            KindRow::Review { of, verdict } => EntryKind::Review {
                of: TaskId::from_raw(of),
                verdict: match verdict {
                    VerdictRow::Approve => Verdict::Approve,
                    VerdictRow::ChangesRequested => Verdict::ChangesRequested,
                },
            },
        },
        body: text_of(&row.body)?,
        refs: row.refs.iter().map(ref_of).collect::<Result<_, _>>()?,
        supersedes: row.supersedes.map(EntryId::from_raw),
    })
}

fn agent_of(a: AgentRow) -> Agent {
    match a {
        AgentRow::Claude => Agent::Claude,
        AgentRow::Codex => Agent::Codex,
        AgentRow::Gemini => Agent::Gemini,
        AgentRow::Local => Agent::Local,
    }
}

fn ref_of(r: &RefRow) -> Result<Ref, StoreError> {
    Ok(match r {
        RefRow::Path(s) => Ref::Path(text_of(s)?),
        RefRow::Commit(s) => Ref::Commit(text_of(s)?),
        RefRow::Url(s) => Ref::Url(text_of(s)?),
        RefRow::Entry(id) => Ref::Entry(EntryId::from_raw(*id)),
    })
}

fn text_of(s: &str) -> Result<Text, StoreError> {
    Text::new(s).ok_or_else(|| StoreError::Corrupt("blank text".to_owned()))
}

fn corrupt<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Corrupt(e.to_string())
}
