//! Commands: everything an external party can ask the core to do.

use crate::ids::*;
use crate::record::*;
use crate::state::{Card, Gate, State};
use rusty_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Opens the task. Issued once by the moderator before anything else.
    Open(OpenTask),
    Assign {
        role: Role,
        principal: Principal,
    },
    /// Driver-supplied clock tick. Fires deadlines.
    Tick,
    /// Host ended a turn early (process died, inference timed out).
    AbortTurn,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTask {
    pub task: TaskId,
    pub repo: String,
    pub profile_digest: Sha256,
    pub budget: crate::state::Budget,
    pub turn_ms: u64,
    pub request_ms: u64,
    pub brief: BlobRef,
    pub human: PrincipalId,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HumanAction {
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
    /// Driver boundary: the referenced blob is not in the store.
    BlobMissing,
    /// Driver boundary: the claimed payload is not what the stored bytes decode to.
    PayloadMismatch,
}
