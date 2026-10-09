//! `TaskState`: the indexed fold over events that rules consult. `Card` is its projection.

use crate::command::Response;
use crate::event::{EscalateReason, Event};
use crate::ids::*;
use crate::record::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum State {
    Planning,
    PlanGate,
    Build,
    Test,
    Review,
    MergeGate,
    Approved,
    Escalated,
    Closed,
    Cancelled,
}

impl State {
    pub const ALL: [State; 10] = [
        State::Planning,
        State::PlanGate,
        State::Build,
        State::Test,
        State::Review,
        State::MergeGate,
        State::Approved,
        State::Escalated,
        State::Closed,
        State::Cancelled,
    ];
    pub fn terminal(self) -> bool {
        matches!(self, State::Closed | State::Cancelled)
    }
    /// The role that holds the default turn, if any.
    pub fn default_role(self) -> Option<Role> {
        match self {
            State::Planning => Some(Role::Planner),
            State::Build => Some(Role::Coder),
            State::Test => Some(Role::Tester),
            State::Review => Some(Role::Reviewer),
            _ => None,
        }
    }
    /// Consultation table: roles that may receive a yielded turn here.
    pub fn consultable(self, role: Role) -> bool {
        match self {
            State::Planning => matches!(role, Role::Coder | Role::Tester),
            State::Build => matches!(role, Role::Planner | Role::Tester),
            State::Test => matches!(role, Role::Planner | Role::Coder),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gate {
    Plan,
    Merge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnKind {
    Default,
    Consultation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BudgetField {
    Messages,
    Bytes,
    Reads,
    ReadsPerTurn,
    Turns,
    Iterations,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub messages: u64,
    pub bytes: u64,
    pub reads: u64,
    pub reads_per_turn: u64,
    pub turns: u64,
    pub iterations: u64,
}

impl Budget {
    pub fn get(&self, f: BudgetField) -> u64 {
        match f {
            BudgetField::Messages => self.messages,
            BudgetField::Bytes => self.bytes,
            BudgetField::Reads => self.reads,
            BudgetField::ReadsPerTurn => self.reads_per_turn,
            BudgetField::Turns => self.turns,
            BudgetField::Iterations => self.iterations,
        }
    }
    pub fn set(&mut self, f: BudgetField, v: u64) {
        match f {
            BudgetField::Messages => self.messages = v,
            BudgetField::Bytes => self.bytes = v,
            BudgetField::Reads => self.reads = v,
            BudgetField::ReadsPerTurn => self.reads_per_turn = v,
            BudgetField::Turns => self.turns = v,
            BudgetField::Iterations => self.iterations = v,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spent {
    pub messages: u64,
    pub bytes: u64,
    pub reads: u64,
    pub reads_this_turn: u64,
    pub turns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub id: TurnId,
    pub role: Role,
    pub kind: TurnKind,
    pub deadline: Time,
    pub return_to: Option<Role>,
    pub read_arts: HashSet<ArtId>,
    /// Snapshot at grant, for the guarded consultation return (T22).
    pub state_at: State,
    pub candidate_at: Option<ArtId>,
    pub run_at: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub candidate: ArtId,
    pub status: Option<RunStatus>,
    pub report: Option<ArtId>,
    pub log: Option<ArtId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRequest {
    pub msg: MsgId,
    pub requester: Role,
    pub gate: bool,
    pub deadline: Option<Time>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub role: Role,
    pub candidate: ArtId,
    pub run: RunId,
    pub kind: VerdictKind,
    pub msg: MsgId,
}

/// Projection served by `task_card`. Never carries the token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub task: TaskId,
    pub rev: Rev,
    pub state: State,
    pub turn: Option<(Role, TurnKind, Time)>,
    pub iteration: u64,
    pub brief: Option<ArtId>,
    pub spec: Option<ArtId>,
    pub candidate: Option<ArtId>,
    pub run: Option<(RunId, Option<RunStatus>, Option<ArtId>)>,
    pub pending_request: Option<MsgId>,
    pub budget: Budget,
    pub spent: Spent,
    pub rejections: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskState {
    pub task: TaskId,
    /// Keys every execution token and run secret. Never logged: the host
    /// keeps it beside the store and sets it on the driver. All zero in the
    /// in-memory tests.
    pub master: [u8; 32],
    pub rev: Rev,
    pub repo: String,
    pub profile_digest: Sha256,
    pub human: PrincipalId,
    pub budget: Budget,
    pub spent: Spent,
    pub turn_ms: u64,
    pub request_ms: u64,
    pub state: State,
    pub iteration: u64,
    pub principals: HashMap<PrincipalId, Principal>,
    pub assigned: HashMap<Role, PrincipalId>,
    pub brief: Option<ArtId>,
    pub approved_spec: Option<ArtId>,
    pub gate_request: Option<(MsgId, ArtId)>,
    pub candidate: Option<ArtId>,
    pub run: Option<Run>,
    pub revoked_runs: HashSet<RunId>,
    pub next_run: u64,
    pub artifacts: BTreeMap<ArtId, ArtifactRecord>,
    pub messages: Vec<Message>,
    pub turn: Option<Turn>,
    pub next_turn: u64,
    pub pending_request: Option<PendingRequest>,
    pub verdicts: Vec<VerdictRecord>,
    pub merge_approval: Option<(ArtId, RunId)>,
    pub escalated_from: Option<State>,
    pub rejections: u64,
    pub ops: HashMap<(PrincipalId, OpId), (Sha256, Response)>,
    /// Fields already escalated for; each fires once (E3).
    pub exhausted: HashSet<BudgetField>,
    pub opened: bool,
}

impl TaskState {
    pub fn new(task: TaskId) -> TaskState {
        TaskState {
            task,
            master: [0; 32],
            rev: Rev(0),
            repo: String::new(),
            profile_digest: Sha256([0; 32]),
            human: PrincipalId("human".into()),
            budget: Budget {
                messages: 40,
                bytes: 5_000_000,
                reads: 150,
                reads_per_turn: 30,
                turns: 60,
                iterations: 4,
            },
            spent: Spent::default(),
            turn_ms: 600_000,
            request_ms: 86_400_000,
            state: State::Planning,
            iteration: 0,
            principals: HashMap::new(),
            assigned: HashMap::new(),
            brief: None,
            approved_spec: None,
            gate_request: None,
            candidate: None,
            run: None,
            revoked_runs: HashSet::new(),
            next_run: 1,
            artifacts: BTreeMap::new(),
            messages: Vec::new(),
            turn: None,
            next_turn: 1,
            pending_request: None,
            verdicts: Vec::new(),
            merge_approval: None,
            escalated_from: None,
            rejections: 0,
            ops: HashMap::new(),
            exhausted: HashSet::new(),
            opened: false,
        }
    }

    pub fn fold(task: TaskId, events: &[Event]) -> TaskState {
        Self::fold_with(task, [0; 32], events)
    }

    /// [`Self::fold`] under a master secret.
    pub fn fold_with(task: TaskId, master: [u8; 32], events: &[Event]) -> TaskState {
        let mut s = TaskState::new(task);
        s.master = master;
        for e in events {
            s.apply(e);
        }
        s
    }

    pub fn role_of(&self, p: &PrincipalId) -> Option<Role> {
        match self.principals.get(p).map(|x| &x.kind) {
            Some(PrincipalKind::Agent { role, .. }) if self.assigned.get(role) == Some(p) => {
                Some(*role)
            }
            _ => None,
        }
    }

    pub fn vendor_of(&self, p: &PrincipalId) -> Option<&str> {
        match self.principals.get(p).map(|x| &x.kind) {
            Some(PrincipalKind::Agent { vendor, .. }) => Some(vendor),
            _ => None,
        }
    }

    pub fn next_msg_id(&self) -> MsgId {
        MsgId(self.messages.len() as u64 + 1)
    }

    pub fn next_art_id(&self) -> ArtId {
        ArtId(self.artifacts.len() as u64 + 1)
    }

    pub fn message(&self, id: MsgId) -> Option<&Message> {
        self.messages.get(id.0.checked_sub(1)? as usize)
    }

    pub fn current_candidate(&self) -> Option<&ArtifactRecord> {
        self.artifacts.get(&self.candidate?)
    }

    /// Selected run for the current candidate, if any.
    pub fn selected_run(&self) -> Option<&Run> {
        let r = self.run.as_ref()?;
        (Some(r.candidate) == self.candidate).then_some(r)
    }

    pub fn verdict(&self, role: Role, kind: VerdictKind) -> Option<&VerdictRecord> {
        let run = self.selected_run()?;
        self.verdicts.iter().find(|v| {
            v.role == role && v.kind == kind && v.candidate == run.candidate && v.run == run.id
        })
    }

    /// Execution token for `principal` in `turn`: a digest keyed by the
    /// task's master secret. Not in the log; a reader of the log cannot
    /// forge it without the master.
    pub fn token_for(&self, principal: &PrincipalId, turn: TurnId) -> Token {
        Token(self.keyed(format!("token|{}|{}|{}", self.task, principal, turn.0)))
    }

    /// Run secret for `run`, keyed like [`Self::token_for`].
    pub fn secret_for(&self, run: RunId) -> RunSecret {
        RunSecret(self.keyed(format!("run|{}|{}", self.task, run.0)))
    }

    fn keyed(&self, label: String) -> Sha256 {
        let mut bytes = Vec::with_capacity(32 + label.len());
        bytes.extend_from_slice(&self.master);
        bytes.extend_from_slice(label.as_bytes());
        Sha256::of(&bytes)
    }

    pub fn card(&self) -> Card {
        Card {
            task: self.task.clone(),
            rev: self.rev,
            state: self.state,
            turn: self.turn.as_ref().map(|t| (t.role, t.kind, t.deadline)),
            iteration: self.iteration,
            brief: self.brief,
            spec: self.approved_spec,
            candidate: self.candidate,
            run: self.selected_run().map(|r| (r.id, r.status, r.report)),
            pending_request: self.pending_request.as_ref().map(|p| p.msg),
            budget: self.budget,
            spent: self.spent,
            rejections: self.rejections,
        }
    }

    /// Apply one event. Total: every event is representable in the fold.
    pub fn apply(&mut self, e: &Event) {
        self.rev = Rev(self.rev.0 + 1);
        match e {
            Event::TaskOpened {
                task,
                repo,
                profile_digest,
                budget,
                turn_ms,
                request_ms,
                human,
            } => {
                self.task = task.clone();
                self.repo = repo.clone();
                self.profile_digest = *profile_digest;
                self.budget = *budget;
                self.turn_ms = *turn_ms;
                self.request_ms = *request_ms;
                self.human = human.clone();
                self.opened = true;
                let h = Principal {
                    id: human.clone(),
                    kind: PrincipalKind::Human,
                };
                self.principals.insert(human.clone(), h);
            }
            Event::Assigned { role, principal } => {
                self.assigned.insert(*role, principal.id.clone());
                self.principals
                    .insert(principal.id.clone(), principal.clone());
            }
            Event::TurnGranted {
                role,
                turn,
                kind,
                deadline,
                return_to,
            } => {
                self.turn = Some(Turn {
                    id: *turn,
                    role: *role,
                    kind: *kind,
                    deadline: *deadline,
                    return_to: *return_to,
                    read_arts: HashSet::new(),
                    state_at: self.state,
                    candidate_at: self.candidate,
                    run_at: self.run.as_ref().map(|r| r.id),
                });
                self.next_turn = turn.0 + 1;
                self.spent.turns += 1;
                self.spent.reads_this_turn = 0;
            }
            Event::TurnEnded { turn, cause: _ } => {
                if self.turn.as_ref().map(|t| t.id) == Some(*turn) {
                    self.turn = None;
                }
            }
            Event::MessageAppended(m) => self.messages.push(m.clone()),
            Event::ArtifactStored(a) => {
                if a.kind == ArtifactKind::Brief {
                    self.brief = Some(a.id);
                }
                if let Some(r) = self.run.as_mut() {
                    if a.run == Some(r.id) {
                        match a.kind {
                            ArtifactKind::TestReport => r.report = Some(a.id),
                            ArtifactKind::Log => r.log = Some(a.id),
                            _ => {}
                        }
                    }
                }
                self.artifacts.insert(a.id, a.clone());
            }
            Event::CandidateSubmitted { candidate, .. } => {
                self.candidate = Some(*candidate);
                self.merge_approval = None;
            }
            Event::RunStarted { candidate, run } => {
                self.run = Some(Run {
                    id: *run,
                    candidate: *candidate,
                    status: None,
                    report: None,
                    log: None,
                });
                self.next_run = run.0 + 1;
            }
            Event::RunRevoked { run } => {
                self.revoked_runs.insert(*run);
                if self.run.as_ref().map(|r| r.id) == Some(*run) {
                    self.run = None;
                }
            }
            Event::TestReportStored { status, .. } => {
                if let Some(r) = self.run.as_mut() {
                    r.status = Some(*status);
                }
            }
            Event::RequestOpened {
                msg,
                requester,
                gate,
                deadline,
            } => {
                self.pending_request = Some(PendingRequest {
                    msg: *msg,
                    requester: *requester,
                    gate: *gate,
                    deadline: *deadline,
                });
                if *gate {
                    let spec = self
                        .message(*msg)
                        .and_then(|m| m.draft.refs.first())
                        .and_then(|r| match r {
                            Ref::Art { id, .. } => Some(*id),
                            Ref::Msg(_) => None,
                        });
                    if let Some(spec) = spec {
                        self.gate_request = Some((*msg, spec));
                    }
                }
            }
            Event::RequestSettled { .. } => self.pending_request = None,
            Event::StateChanged { from, to } => {
                if *to == State::Escalated {
                    self.escalated_from = Some(*from);
                }
                if *to == State::Build
                    && matches!(
                        from,
                        State::Test | State::Review | State::MergeGate | State::Approved
                    )
                {
                    self.iteration += 1;
                }
                self.state = *to;
            }
            Event::HumanApproval { gate, subject, run } => match gate {
                Gate::Plan => {
                    self.approved_spec = Some(*subject);
                    self.gate_request = None;
                    self.pending_request = None;
                }
                Gate::Merge => {
                    if let Some(r) = run {
                        self.merge_approval = Some((*subject, *r));
                    }
                }
            },
            Event::HumanRejection { target, .. } => {
                if *target == State::Planning {
                    self.approved_spec = None;
                    self.gate_request = None;
                    self.pending_request = None;
                    self.candidate = None;
                    self.run = None;
                }
                self.merge_approval = None;
            }
            Event::Rerun { .. } => self.merge_approval = None,
            Event::MergeReceipt { .. } => {}
            Event::Cancelled { .. } => {}
            Event::Escalated { reason, .. } => {
                if let EscalateReason::Budget(f) = reason {
                    self.exhausted.insert(*f);
                }
            }
            Event::Resumed { target } => {
                if *target == State::Planning {
                    self.approved_spec = None;
                    self.gate_request = None;
                    self.pending_request = None;
                    self.candidate = None;
                    self.run = None;
                    self.merge_approval = None;
                }
            }
            Event::BudgetExtended { field, limit } => {
                self.budget.set(*field, *limit);
                self.exhausted.remove(field);
            }
            Event::Charged {
                messages,
                bytes,
                reads,
                turn,
                ..
            } => {
                self.spent.messages += messages;
                self.spent.bytes += bytes;
                self.spent.reads += reads;
                if turn.is_some() && self.turn.as_ref().map(|t| t.id) == *turn {
                    self.spent.reads_this_turn += reads;
                }
            }
            Event::ReadDeduped { turn, art } => {
                if let Some(t) = self.turn.as_mut() {
                    if t.id == *turn {
                        t.read_arts.insert(*art);
                    }
                }
            }
            Event::Rejected { .. } => self.rejections += 1,
            Event::OpAccepted {
                principal,
                op,
                payload,
                response,
            } => {
                self.ops.insert(
                    (principal.clone(), op.clone()),
                    (*payload, response.clone()),
                );
            }
            Event::VerdictsStaled { candidate } => {
                self.verdicts.retain(|v| v.candidate != *candidate);
                self.merge_approval = None;
            }
        }
        if let Event::MessageAppended(m) = e {
            if let (Body::Verdict(v), Some(role)) = (&m.draft.body, m.from_role) {
                self.verdicts.push(VerdictRecord {
                    role,
                    candidate: v.subject,
                    run: v.run,
                    kind: v.verdict,
                    msg: m.id,
                });
            }
        }
    }
}
