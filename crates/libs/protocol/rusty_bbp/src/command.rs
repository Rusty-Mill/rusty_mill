//! Commands: everything an external party can ask the core to do.

use crate::ids::*;
use crate::record::*;
use crate::role::Role;
use crate::state::{Card, Gate, State};
use rusty_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Opens the task. Issued once by the moderator before anything else.
    Open(OpenTask),
    Assign {
        role: RoleId,
        principal: Principal,
    },
    /// Driver-supplied clock tick. Fires deadlines.
    Tick,
    /// Host ended a turn early (process died, inference timed out). `Some`
    /// ends that turn only, and does nothing if it is not live; `None` ends
    /// every live turn.
    AbortTurn {
        turn: Option<TurnId>,
    },
    Agent {
        principal: PrincipalId,
        /// Adapter-injected; `None` for `task_card`.
        token: Option<Token>,
        op: OpId,
        action: AgentAction,
    },
    Runner {
        op: OpId,
        run: RunId,
        secret: RunSecret,
        blob: BlobRef,
        payload: ArtifactPayload,
    },
    Human {
        op: OpId,
        action: HumanAction,
    },
    /// Claims mode: a write by the host (orch), outside any role. Today only
    /// action marks (C-R9).
    Host {
        op: OpId,
        action: HostAction,
    },
}

/// Host writes in claims mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostAction {
    /// Record use of an accepted claim before acting on it.
    Acting {
        claim: MsgId,
        epoch: u32,
        reference: String,
    },
    /// The outcome of a mark.
    ActionResult {
        mark: u64,
        outcome: crate::claims::MarkOutcome,
    },
    /// #597 G3: a new holdout suite version that only adds files to the
    /// current one. The host checks that before writing it.
    HoldoutExtended { digest: Sha256 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTask {
    pub task: TaskId,
    /// The roster for the task. Validated by the core; immutable afterwards.
    pub roster: Vec<Role>,
    pub repo: String,
    pub profile_digest: Sha256,
    pub budget: crate::state::Budget,
    pub turn_ms: u64,
    pub request_ms: u64,
    /// At most this many turns are live at once, one per role. 1 is the
    /// serialized v0.5 behavior; the core accepts 1 to the roster size.
    pub max_live_turns: u32,
    pub brief: BlobRef,
    pub human: PrincipalId,
    /// Added after schema 2 shipped: a payload without it opens as `TurnBased`.
    #[rusty_serde(default)]
    pub policy: crate::policy::SchedulingPolicy,
    /// Stage pipeline (default) or claims mode.
    #[rusty_serde(default)]
    pub mode: crate::claims::Mode,
    /// Claims mode only: the update rule's numbers. `None` takes the defaults.
    #[rusty_serde(default)]
    pub claims: Option<crate::claims::ClaimsConfig>,
    /// Claims mode only: SHA-256 of the frozen holdout suite (#597). `None`
    /// opens a task without one.
    #[rusty_serde(default)]
    pub holdout_digest: Option<Sha256>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentAction {
    Post(Draft),
    PutArtifact {
        blob: BlobRef,
        payload: ArtifactPayload,
    },
    Read {
        after: Option<MsgId>,
        limit: usize,
    },
    GetArtifact {
        id: ArtId,
    },
    TaskCard,
    /// Claims mode: claims changed after `after` in `domains`, plus dependents.
    ClaimsDelta {
        after: Rev,
        domains: Vec<String>,
    },
    /// Claims mode: one claim's card.
    ClaimCard {
        claim: MsgId,
    },
    /// Claims mode: one claim's compaction line.
    ClaimLine {
        claim: MsgId,
    },
}

impl AgentAction {
    /// A read-only view: not fenced by a turn, not recorded for replay (R15).
    pub fn is_view(&self) -> bool {
        matches!(
            self,
            AgentAction::TaskCard
                | AgentAction::ClaimsDelta { .. }
                | AgentAction::ClaimCard { .. }
                | AgentAction::ClaimLine { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HumanAction {
    /// Apply any human action only to the exact public revision shown.
    /// Checked inside the transactional driver, including conflict retries.
    Fenced {
        expected_rev: Rev,
        action: Box<HumanAction>,
    },
    Approve {
        gate: Gate,
        subject: ArtId,
        run: Option<RunId>,
    },
    Reject {
        expected_rev: Rev,
        target: State,
        reason: String,
    },
    Post(Draft),
    Rerun {
        candidate: ArtId,
        expected_rev: Rev,
    },
    MergeReceipt {
        candidate: ArtId,
        revision: String,
    },
    Resume {
        target: State,
        expected_rev: Rev,
    },
    BudgetExtended {
        field: crate::state::BudgetField,
        limit: u64,
    },
    Cancel {
        expected_rev: Rev,
        reason: String,
    },
    /// Claims mode: settle or reopen one claim (C-R8).
    ClaimDecision {
        claim: MsgId,
        outcome: crate::claims::ClaimOutcome,
    },
    /// Claims mode: accept `chosen` and drop the other live candidates of `question`.
    Choose {
        question: MsgId,
        chosen: MsgId,
    },
    /// Claims mode: one more `base(c)` of budget; approves a held rerun.
    ClaimBudgetExtended {
        claim: MsgId,
    },
    /// Claims mode: withdraw a held rerun.
    WithdrawRerun {
        candidate: ArtId,
    },
    /// Claims mode: the human reconciles a mark.
    ClaimActionResult {
        mark: u64,
        outcome: crate::claims::MarkOutcome,
    },
    /// Claims mode: end the task.
    Close {
        expected_rev: Rev,
    },
    /// Claims mode (#597 G3): adopt a holdout suite version that removes or
    /// changes existing tests. Only the human may.
    HoldoutUpdated {
        digest: Sha256,
    },
}

/// What a command returns. Stored with the accepted op so replays reproduce it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Posted(MsgId),
    Stored(ArtId),
    Read {
        messages: Vec<Message>,
        next: Option<MsgId>,
        more: bool,
    },
    Artifact(ArtifactRecord),
    Card(Card),
    Rejected(Rejection),
    /// Claims mode views.
    Claims(Vec<crate::claims::ClaimView>),
    Line(Option<String>),
    /// A mark was recorded.
    Marked(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub code: Code,
    pub detail: String,
}

impl Rejection {
    pub fn new(code: Code, detail: impl Into<String>) -> Rejection {
        Rejection {
            code,
            detail: detail.into(),
        }
    }
}

/// Every rejection code in the spec, plus `forbidden` for authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Code {
    /// Human CLI syntax was invalid; no command was dispatched.
    BadHumanInput,
    Forbidden,
    Terminal,
    OpConflict,
    StaleRev,
    UnknownKind,
    UnknownVersion,
    RefsRequired,
    EvidenceRequired,
    EvidenceUnbound,
    RunPending,
    NotPassed,
    BlockingRequired,
    RefUnresolved,
    WrongKind,
    FragmentTooLong,
    NotAssigned,
    RoleForbidden,
    WrongState,
    ConsultationOnly,
    StaleCandidate,
    SpecMismatch,
    SubjectMissing,
    DecisionForbidden,
    BodyTooLong,
    BadRecipient,
    HumanRequired,
    BadReplyTo,
    TurnBudgetExhausted,
    BudgetExhausted,
    Immutable,
    ApprovalForbidden,
    BadToken,
    TokenExpired,
    StaleTurn,
    KindForbidden,
    TooLarge,
    BadRunSecret,
    StaleRun,
    DuplicateReport,
    InconsistentReport,
    ProfileMismatch,
    NoApprovedSpec,
    StaleSubject,
    NotReviewed,
    NotOpen,
    AlreadyOpen,
    YieldNested,
    /// `assign` named a role the task's roster does not contain.
    UnknownRole,
    /// `open` carried a roster that failed validation.
    BadRoster,
    /// `open` carried a `max_live_turns` outside 1 to the roster size.
    BadLiveTurns,
    /// A co-edited spec edit is anchored on a line edited since its base.
    StaleBase,
    /// A co-edited spec edit is malformed: no ops, an empty insert, a line break.
    BadEdit,
    /// Driver boundary: the referenced blob is not in the store.
    BlobMissing,
    /// Driver boundary: the claimed payload is not what the stored bytes decode to.
    PayloadMismatch,
    /// Claims mode: `open` carried a bad `claims` value (detail names the field).
    BadClaimsConfig,
    /// Claims mode: the session credential is missing, foreign or expired.
    BadCredential,
    /// The operation belongs to the other mode.
    WrongMode,
    /// A claim or evidence body breaks its field rules.
    BadClaim,
    /// A new claim names a dropped, void or superseded dep.
    DepClosed,
    /// The claim's evidence budget is spent.
    ClaimBudget,
    /// A candidate names a claim that is void or escalated.
    ClaimNotOpen,
    /// A second result for one mark.
    Duplicate,
    /// Executed support cites a run whose candidate changed tests, unendorsed.
    TestsUnendorsed,
    /// A holdout candidate does not build the claim's current code, or the
    /// task has no holdout suite.
    HoldoutMismatch,
}
