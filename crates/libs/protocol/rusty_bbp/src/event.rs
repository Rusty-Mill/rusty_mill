//! Events: the append-only log. `TaskState` is a fold over these.

use crate::command::Response;
use crate::ids::*;
use crate::policy::SchedulingPolicy;
use crate::record::*;
use crate::role::Role;
use crate::state::{Budget, BudgetField, Gate, Slice, State, TurnKind};
use rusty_serde::{Deserialize, Serialize};

/// The log schema of a task that allows one live turn. Schema 1 was v0.4.1 (a
/// fixed role enumeration, no roster); it does not replay.
pub const LOG_SCHEMA: u32 = 2;

/// The log schema of a task that allows several live turns. Written only when
/// `max_live_turns > 1`, so a build that reads schema 2 alone refuses such a
/// log instead of replaying it as serialized.
pub const LOG_SCHEMA_CONCURRENT: u32 = 3;

/// The schema `open` records for a task with this cap.
pub fn schema_for(max_live_turns: u32) -> u32 {
    if max_live_turns > 1 {
        LOG_SCHEMA_CONCURRENT
    } else {
        LOG_SCHEMA
    }
}

/// Does this build replay a log of this schema?
pub fn schema_supported(schema: u32) -> bool {
    schema == LOG_SCHEMA || schema == LOG_SCHEMA_CONCURRENT
}

fn one_turn() -> u32 {
    1
}

// Skipped when serialized so a one-turn task's log is byte-identical to schema 2.
fn serialized_cap(cap: &u32) -> bool {
    *cap <= 1
}

// Skipped when serialized so a turn-based task's log is byte-identical to schema 2.
fn is_turn_based(p: &SchedulingPolicy) -> bool {
    *p == SchedulingPolicy::TurnBased
}

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
        /// Cap on concurrently live turns. Present only in schema 3; absent
        /// reads as 1.
        #[rusty_serde(default = "one_turn", skip_serializing_if = "serialized_cap")]
        max_live_turns: u32,
        human: PrincipalId,
        /// Added after schema 2 shipped. Absent reads as `TurnBased` and
        /// `TurnBased` is not written, so a turn-based log is byte for byte
        /// the one the golden in `tests/schema.rs` pins.
        #[rusty_serde(default, skip_serializing_if = "is_turn_based")]
        policy: SchedulingPolicy,
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
        #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
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
