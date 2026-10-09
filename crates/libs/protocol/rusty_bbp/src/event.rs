//! Events: the append-only log. `TaskState` is a fold over these.

use crate::command::Response;
use crate::ids::*;
use crate::record::*;
use crate::state::{Budget, BudgetField, Gate, State, TurnKind};
use rusty_serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEnd {
    Pass,
    Yield,
    Verdict,
    Candidate,
    Request,
    Deadline,
    Budget,
    Aborted,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscalateReason {
    Budget(BudgetField),
    RequestDeadline,
    RunError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    TaskOpened {
        task: TaskId,
        repo: String,
        profile_digest: Sha256,
        budget: Budget,
        turn_ms: u64,
        request_ms: u64,
        human: PrincipalId,
    },
    Assigned {
        role: Role,
        principal: Principal,
    },
    TurnGranted {
        role: Role,
        turn: TurnId,
        kind: TurnKind,
        token: Token,
        deadline: Time,
        return_to: Option<Role>,
    },
    TurnEnded {
        turn: TurnId,
        cause: TurnEnd,
    },
    MessageAppended(Message),
    ArtifactStored(ArtifactRecord),
    CandidateSubmitted {
        candidate: ArtId,
        spec: ArtId,
    },
    RunStarted {
        candidate: ArtId,
        run: RunId,
        secret: RunSecret,
    },
    RunRevoked {
        run: RunId,
    },
    TestReportStored {
        candidate: ArtId,
        run: RunId,
        status: RunStatus,
        report: ArtId,
    },
    RequestOpened {
        msg: MsgId,
        requester: Role,
        gate: bool,
        deadline: Option<Time>,
    },
    RequestSettled {
        msg: MsgId,
        by: MsgId,
    },
    StateChanged {
        from: State,
        to: State,
    },
    HumanApproval {
        gate: Gate,
        subject: ArtId,
        run: Option<RunId>,
    },
    HumanRejection {
        from: State,
        target: State,
        reason: String,
    },
    Rerun {
        candidate: ArtId,
    },
    MergeReceipt {
        candidate: ArtId,
        revision: String,
    },
    Cancelled {
        reason: String,
    },
    Escalated {
        reason: EscalateReason,
        from: State,
    },
    Resumed {
        target: State,
    },
    BudgetExtended {
        field: BudgetField,
        limit: u64,
    },
    Charged {
        principal: PrincipalId,
        turn: Option<TurnId>,
        messages: u64,
        bytes: u64,
        reads: u64,
    },
    ReadDeduped {
        turn: TurnId,
        art: ArtId,
    },
    Rejected {
        principal: Option<PrincipalId>,
        code: crate::command::Code,
    },
    /// Accepted external operation and the response it produced (R15 replay).
    OpAccepted {
        principal: PrincipalId,
        op: OpId,
        payload: Sha256,
        response: Response,
    },
    VerdictsStaled {
        candidate: ArtId,
    },
}
