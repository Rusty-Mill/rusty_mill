//! The on-disk shape of a goal snapshot, and the two conversions.
//!
//! Every type here is encoded by the engine's positional bincode codec: the
//! complete v1 layout is the format. Any layout change is incompatible unless
//! an explicit old-layout reader migrates it. Otherwise it must use a new
//! schema tag and old files are rejected before record decoding.

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
    const SCHEMA_TAG: &'static str = "rusty_orch::GoalRecord::v1";
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

#[cfg(test)]
mod format_tests {
    use super::*;
    use rusty_multimodal_db_engine::codec;
    use rusty_multimodal_db_engine::generic::GenericMmapStore;

    fn empty_v1() -> GoalRecord {
        GoalRecord::from_state(
            &GoalState {
                fingerprint: 7,
                plan: Plan::new(GoalId::from_raw(1)),
                board: Board::new(GoalId::from_raw(1)),
                ledger: Ledger::new(),
            },
            1,
        )
    }

    fn populated_v1() -> GoalRecord {
        let task = |id, role, state| TaskRow {
            id,
            role,
            instruction: format!("instruction {id}"),
            acceptance: vec![format!("acceptance {id}")],
            refs: vec![],
            depends_on: vec![],
            max_calls: 3,
            state,
        };
        let entry = |id, task, author, kind, body: &str| EntryRow {
            id,
            task,
            author,
            kind,
            body: body.to_owned(),
            refs: vec![],
            supersedes: None,
        };

        let mut tasks = vec![
            task(
                1,
                RoleRow::Research,
                StateRow::Done {
                    agent: AgentRow::Codex,
                    outputs: vec![1],
                },
            ),
            task(
                2,
                RoleRow::Design,
                StateRow::Blocked {
                    agent: AgentRow::Claude,
                    question: 2,
                },
            ),
            task(
                3,
                RoleRow::Triage,
                StateRow::Running {
                    agent: AgentRow::Gemini,
                },
            ),
            task(
                4,
                RoleRow::Implement,
                StateRow::Failed {
                    agent: AgentRow::Codex,
                    reason: "failed".to_owned(),
                },
            ),
            task(
                5,
                RoleRow::Review { target: 1 },
                StateRow::Done {
                    agent: AgentRow::Local,
                    outputs: vec![4],
                },
            ),
            task(6, RoleRow::Research, StateRow::Pending),
        ];
        tasks[0].refs = vec![
            RefRow::Path("src/lib.rs".to_owned()),
            RefRow::Commit("abc123".to_owned()),
            RefRow::Url("https://example.invalid".to_owned()),
        ];
        tasks[1].depends_on = vec![1];

        let mut entries = vec![
            entry(
                1,
                Some(1),
                AuthorRow::Agent(AgentRow::Codex),
                KindRow::Finding {
                    confidence: ConfidenceRow::Medium,
                },
                "finding",
            ),
            entry(
                2,
                Some(2),
                AuthorRow::Agent(AgentRow::Claude),
                KindRow::Question,
                "question",
            ),
            entry(
                3,
                None,
                AuthorRow::Human,
                KindRow::Answer { to: 2 },
                "answer",
            ),
            entry(
                4,
                Some(5),
                AuthorRow::Agent(AgentRow::Local),
                KindRow::Review {
                    of: 1,
                    verdict: VerdictRow::Approve,
                },
                "approved",
            ),
            entry(
                5,
                Some(1),
                AuthorRow::Agent(AgentRow::Codex),
                KindRow::Decision,
                "decision",
            ),
            entry(
                6,
                Some(1),
                AuthorRow::Agent(AgentRow::Codex),
                KindRow::Artifact,
                "artifact",
            ),
            entry(
                7,
                Some(1),
                AuthorRow::Agent(AgentRow::Codex),
                KindRow::Assumption,
                "old assumption",
            ),
            entry(
                8,
                Some(1),
                AuthorRow::Agent(AgentRow::Codex),
                KindRow::Assumption,
                "new assumption",
            ),
            entry(
                9,
                Some(1),
                AuthorRow::Agent(AgentRow::Gemini),
                KindRow::Finding {
                    confidence: ConfidenceRow::Low,
                },
                "low finding",
            ),
            entry(
                10,
                Some(1),
                AuthorRow::Agent(AgentRow::Claude),
                KindRow::Finding {
                    confidence: ConfidenceRow::High,
                },
                "high finding",
            ),
            entry(
                11,
                Some(5),
                AuthorRow::Agent(AgentRow::Local),
                KindRow::Review {
                    of: 1,
                    verdict: VerdictRow::ChangesRequested,
                },
                "changes",
            ),
        ];
        entries[4].refs = vec![RefRow::Entry(4)];
        entries[5].refs = vec![RefRow::Path("out.md".to_owned())];
        entries[7].supersedes = Some(7);

        GoalRecord {
            id: 1,
            revision: 9,
            fingerprint: 0x0102_0304_0506_0708,
            tasks,
            entries,
            goal_calls: 7,
            task_calls: vec![(1, 2), (2, 1), (4, 3), (5, 1)],
        }
    }

    #[test]
    fn v1_empty_record_has_stable_golden_bytes() {
        const GOLDEN: &[u8] = &[
            1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let bytes = codec::encode(&empty_v1()).expect("encode");
        assert_eq!(
            bytes, GOLDEN,
            "update only with an explicit format decision"
        );
        let decoded: GoalRecord = codec::decode(GOLDEN).expect("decode golden");
        assert_eq!(codec::encode(&decoded).expect("re-encode"), GOLDEN);
    }

    #[test]
    fn v1_populated_record_has_stable_golden_bytes_and_rebuilds_domain() {
        let golden = include_bytes!("fixtures/goal_record_v1_populated.bin");
        let record = populated_v1();
        let bytes = codec::encode(&record).expect("encode");
        assert_eq!(
            bytes.as_slice(),
            golden,
            "change only with an explicit format decision"
        );

        let decoded: GoalRecord = codec::decode(golden).expect("decode golden");
        let state = decoded.into_state().expect("rebuild domain");
        assert_eq!(state.goal(), GoalId::from_raw(1));
        assert_eq!(state.fingerprint, 0x0102_0304_0506_0708);
        assert_eq!(state.plan.tasks().len(), 6);
        assert_eq!(state.board.entries().len(), 11);
        assert_eq!(state.ledger.calls(), 7);
        assert_eq!(state.ledger.task_calls(TaskId::from_raw(4)), 3);
    }

    #[test]
    fn corrupt_snapshot_rows_are_rejected_during_domain_reconstruction() {
        let mut fixtures = Vec::new();

        let mut invalid_order = populated_v1();
        invalid_order.tasks[1].depends_on = vec![99];
        fixtures.push(("invalid task prerequisite/order", invalid_order));

        let mut blank_text = populated_v1();
        blank_text.tasks[0].instruction.clear();
        fixtures.push(("blank text", blank_text));

        let mut invalid_board_ref = populated_v1();
        invalid_board_ref.entries[0].refs = vec![RefRow::Entry(99)];
        fixtures.push(("invalid board reference", invalid_board_ref));

        let mut empty_done_outputs = populated_v1();
        empty_done_outputs.tasks[0].state = StateRow::Done {
            agent: AgentRow::Codex,
            outputs: vec![],
        };
        fixtures.push(("empty Done outputs", empty_done_outputs));

        for (label, fixture) in fixtures {
            assert!(
                matches!(fixture.into_state(), Err(StoreError::Corrupt(_))),
                "{label} must be reported as corrupt"
            );
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct WrongVersion {
        id: i64,
        revision: i64,
    }

    impl Record for WrongVersion {
        type Id = i64;
        fn id(&self) -> i64 {
            self.id
        }
    }
    impl SchemaTag for WrongVersion {
        const SCHEMA_TAG: &'static str = "rusty_orch::GoalRecord::v2";
    }
    impl IndexedField<ById> for WrongVersion {
        type IndexValue = i64;
        fn indexed_value(&self) -> &i64 {
            &self.id
        }
    }
    impl ScannableField<Revision> for WrongVersion {
        type ScanValue = i64;
        fn scannable_value(&self) -> i64 {
            self.revision
        }
        fn set_scannable_value(&mut self, value: i64) {
            self.revision = value;
        }
    }

    #[test]
    fn wrong_schema_version_is_rejected_before_record_decode() {
        let dir = rusty_multimodal_db_engine::test_support::fresh_temp_dir("orch_schema")
            .expect("temp dir");
        let path = dir.join("goals.mmap");
        let store: GenericMmapStore<GoalRecord, ById, Revision> =
            GenericMmapStore::create(vec![empty_v1()], &path).expect("create v1");
        drop(store);

        let error = match GenericMmapStore::<WrongVersion, ById, Revision>::open_portable(&path) {
            Ok(_) => panic!("v2 must reject v1"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("schema tag mismatch"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
