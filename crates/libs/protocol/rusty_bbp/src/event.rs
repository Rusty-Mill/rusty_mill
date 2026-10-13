//! Events: the append-only log. `TaskState` is a fold over these.

use crate::command::Response;
use crate::ids::*;
use crate::record::*;
use crate::role::Role;
use crate::state::{Budget, BudgetField, Gate, Slice, State, TurnKind};
use rusty_serde::{Deserialize, Serialize};

/// The log schema this crate writes and reads. Schema 1 was v0.4.1 (a fixed
/// role enumeration, no roster); it does not replay.
pub const LOG_SCHEMA: u32 = 2;

/// The `schema` of the first `task_opened` in `events`, if the log has one.
pub fn log_schema(events: &[Event]) -> Option<u32> {
    events.iter().find_map(|e| match e {
        Event::TaskOpened { schema, .. } => Some(*schema),
        _ => None,
    })
}

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
        /// Log schema version ([`LOG_SCHEMA`]). A reader refuses any other.
        schema: u32,
        /// The roster, immutable for the task.
        roster: Vec<Role>,
        task: TaskId,
        repo: String,
        profile_digest: Sha256,
        budget: Budget,
        turn_ms: u64,
        request_ms: u64,
        /// Cap on concurrently live turns. Absent in logs written before it
        /// existed, which read as 0 and fold as 1.
        #[rusty_serde(default)]
        max_live_turns: u32,
        human: PrincipalId,
    },
    Assigned {
        role: RoleId,
        principal: Principal,
    },
    /// The turn's token is not logged: it is derived from the task's master
    /// secret, which lives outside the log, so a log reader cannot forge it.
    TurnGranted {
        role: RoleId,
        turn: TurnId,
        kind: TurnKind,
        deadline: Time,
        return_to: Option<RoleId>,
        /// The turn's share of the task budget, reserved at grant. `None`
        /// when `max_live_turns` is 1: the whole task budget, with the
        /// crossing write accepted (R11).
        #[rusty_serde(default)]
        slice: Option<Slice>,
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
    /// The run secret is not logged, for the same reason as the turn token.
    RunStarted {
        candidate: ArtId,
        run: RunId,
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
        requester: RoleId,
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
