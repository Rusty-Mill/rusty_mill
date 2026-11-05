//! Events: the append-only log. `TaskState` is a fold over these.

use crate::claims::{
    Affects, ClaimEscalation, ClaimOutcome, ClaimStatus, ClaimsConfig, MarkOutcome, Mode,
    PauseReason, ReopenCause,
};
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

/// The log schema of an event-driven task. A build that predates the `policy`
/// field reads schema 2 and 3 and would drop an unknown `policy` key without
/// error, replaying the task as turn-based; refusing schema 4 is the fence
/// that keeps it from doing so. Written only for `EventDriven`.
pub const LOG_SCHEMA_EVENT_DRIVEN: u32 = 4;

/// The log schema of a claims-mode task (claims addendum). A build that
/// predates claims refuses it rather than replaying it as a stage task.
pub const LOG_SCHEMA_CLAIMS: u32 = 5;

/// The schema `open` records for a task with this cap and policy.
pub fn schema_for(max_live_turns: u32, policy: SchedulingPolicy) -> u32 {
    if policy == SchedulingPolicy::EventDriven {
        LOG_SCHEMA_EVENT_DRIVEN
    } else if max_live_turns > 1 {
        LOG_SCHEMA_CONCURRENT
    } else {
        LOG_SCHEMA
    }
}

/// Does this build replay a log of this schema?
pub fn schema_supported(schema: u32) -> bool {
    matches!(
        schema,
        LOG_SCHEMA | LOG_SCHEMA_CONCURRENT | LOG_SCHEMA_EVENT_DRIVEN | LOG_SCHEMA_CLAIMS
    )
}

fn one_turn() -> u32 {
    1
}

// Skipped when serialized so a one-turn task's log is byte-identical to schema 2.
fn serialized_cap(cap: &u32) -> bool {
    *cap <= 1
}

fn is_stages(m: &Mode) -> bool {
    m.is_stages()
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
        /// `TurnBased` is not written; `EventDriven` is written only under
        /// schema 4, and a fold honors the field only there, so a turn-based log is byte for byte
        /// the one the golden in `tests/schema.rs` pins.
        #[rusty_serde(default, skip_serializing_if = "is_turn_based")]
        policy: SchedulingPolicy,
        /// Claims addendum. Written only in claims mode (schema 5).
        #[rusty_serde(default, skip_serializing_if = "is_stages")]
        mode: Mode,
        #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
        claims: Option<ClaimsConfig>,
        /// Claims mode (#597): SHA-256 of the frozen holdout suite. Written
        /// only when present.
        #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
        holdout_digest: Option<Sha256>,
        /// Claims mode (#598): SHA-256 of the green baseline report. Written
        /// only when present.
        #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
        baseline_digest: Option<Sha256>,
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
    // ---- claims mode (schema 5 only)
    /// The session credential for an assignment; the secret is never logged.
    CredentialIssued {
        role: RoleId,
        principal: PrincipalId,
        digest: Sha256,
    },
    ClaimAccepted {
        claim: MsgId,
        score: i64,
        evidence: u64,
        #[rusty_serde(default, skip_serializing_if = "is_false")]
        sole_candidate: bool,
        #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
        supersedes_lapsed: Option<MsgId>,
    },
    ClaimDropped {
        claim: MsgId,
        score: i64,
        evidence: u64,
    },
    ClaimVoided {
        claim: MsgId,
        by: MsgId,
    },
    ClaimEscalated {
        claim: MsgId,
        reason: ClaimEscalation,
    },
    ClaimReopened {
        claim: MsgId,
        by: ReopenCause,
        epoch: u32,
    },
    ClaimSuperseded {
        claim: MsgId,
        by: MsgId,
        prior: ClaimStatus,
    },
    /// A human decision on a claim, before its effects.
    ClaimDecided {
        claim: MsgId,
        outcome: ClaimOutcome,
    },
    ClaimBudgetExtended {
        claim: MsgId,
    },
    /// One run debited from a claim (candidate stored or human rerun).
    ClaimRunDebited {
        claim: MsgId,
        candidate: ArtId,
    },
    RerunHeld {
        claim: MsgId,
        candidate: ArtId,
    },
    RerunWithdrawn {
        candidate: ArtId,
    },
    /// `failing(c)` after a report or an endorsement.
    ClaimFailing {
        claim: MsgId,
        failing: Vec<String>,
    },
    ClaimActing {
        claim: MsgId,
        epoch: u32,
        mark: u64,
        reference: String,
        at: Time,
    },
    ClaimActionResult {
        mark: u64,
        outcome: MarkOutcome,
    },
    /// #597: the task's holdout suite moved to `digest`, written by the host
    /// for pure additions.
    HoldoutExtended {
        digest: Sha256,
    },
    /// #597: the human adopted holdout suite `digest`, which removes or
    /// changes existing tests.
    HoldoutUpdated {
        digest: Sha256,
    },
    /// #598: the human revised the intent; `brief` is the new brief artifact,
    /// stored just before. Bumps the fold's `intent_version`.
    IntentRevised {
        brief: ArtId,
        #[rusty_serde(default)]
        affects: Affects,
    },
    /// #598: the host paused the task (a ceiling or the cost breaker). The
    /// task goes `escalated`; the human resumes it with `resumed(open)`.
    TaskPaused {
        reason: PauseReason,
        detail: String,
    },
    /// D82: the human took over agent session `session` on `claim`, written
    /// before any input is possible. Under schema 5, written only when a
    /// takeover happens (D83).
    TakeoverStarted {
        takeover: u64,
        claim: MsgId,
        epoch: u32,
        session: String,
        client: String,
        until: Time,
        reason: String,
    },
    /// D82: one batch of what the human typed, the bytes as the mux
    /// delivered them. `sha256` is [`crate::claims::input_chain`] over the
    /// previous batch.
    TakeoverInput {
        takeover: u64,
        seq: u64,
        bytes: crate::ids::Base64Bytes,
        sha256: Sha256,
    },
    /// D82: input permission ended at `at`, by the human's release or the
    /// task's end. The host may record batches already delivered for
    /// [`crate::TAKEOVER_DRAIN_MS`], then writes the one `takeover_ended`.
    TakeoverClosing {
        takeover: u64,
        reason: crate::claims::TakeoverEnd,
        at: Time,
    },
    /// D82: the takeover is over; the totals close the chain. Always the
    /// host's, after `takeover_closing` or for its own reasons.
    TakeoverEnded {
        takeover: u64,
        reason: crate::claims::TakeoverEnd,
        batches: u64,
        last: Option<Sha256>,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}
