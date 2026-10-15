//! Event-driven scheduling, phase 1: which roles the board state makes
//! eligible for the next turn. Pure. Nothing here grants a turn; the
//! scheduling check in `engine.rs` stays the only thing that does.
//!
//! Eligibility is derived from the nine Role fields, so no role is named.
//! Triggers, all state-derived, best first:
//!
//! - **Addressed**: a message that names the role arrived since the role's
//!   last turn ended, and the role may read it. A message to `all` names no
//!   one: it addresses nobody for scheduling (it is still readable by everyone
//!   who could read it). A role that has never had a turn counts every earlier
//!   message as new.
//! - **State**: the role serves the current working state and that state's
//!   turn became available since the role's last turn ended. Being assigned
//!   makes the state's turn available to its server, so a server assigned
//!   after the state began is not skipped.
//! - **Artifact**: a spec, diff, candidate or test report the role may read
//!   was stored in the current state since the role's last turn ended, or, for a
//!   role that has never had a turn, since it was assigned.
//! - **Observer**: the same, for a role that does not serve the state. It is
//!   recorded, and selectable only when several turns may be live.
//!
//! Explicit grants (a yield, a human `ask`, a settled request, a consultation
//! return) are not triggers: they name their grantee directly and bypass the
//! scheduling check, so they never need to be derived from state.

use crate::engine::{admits, visible};
use crate::ids::*;
use crate::record::*;
use crate::role::*;
use crate::state::*;
use rusty_serde::{Deserialize, Serialize};

/// The version of the wake rules `wake_set` implements. Stamped on every shadow
/// record, so a consumer can keep records written under different rules apart
/// (`bbp shadow` reports `by_rules`; its plain totals still add them).
/// Bump it whenever a trigger, the priority, a baseline or the addressing rule
/// changes. 1 is the rules of the first shadow comparison; 2 adds the assignment
/// baseline, the state-entry floor, observers, State above Artifact, and `all`
/// addressing no one.
pub const WAKE_RULES: u32 = 2;

/// Why a role is eligible. Declaration order is priority order: an addressed
/// role outranks the state's own server, which outranks an artifact watcher
/// that serves the state, which outranks an observer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WakeReason {
    Addressed,
    State,
    Artifact,
    Observer,
}

impl WakeReason {
    /// May a role woken for this reason take a turn? An observer is recorded
    /// but selectable only when `max_live_turns` allows more than one.
    pub fn selectable(self, max_live_turns: u32) -> bool {
        self != WakeReason::Observer || max_live_turns > 1
    }
}

/// Artifact kinds that wake a reader. `brief` is the human's, `log` is raw
/// output, and agents write no others.
const WAKING_ARTIFACTS: [ArtifactKind; 4] = [
    ArtifactKind::Spec,
    ArtifactKind::Diff,
    ArtifactKind::Candidate,
    ArtifactKind::TestReport,
];

/// The roles eligible for the next default turn, best first: by reason, then
/// by `turn_order`. Empty when no default turn can be granted at all (a live
/// turn, an open request, a gate or terminal state, a `test` still waiting for
/// its report) or when no role has a trigger.
///
/// A role is skipped when it is unassigned, when it authored the event that
/// would wake it (self-wake exclusion), when the event is not newer than the
/// end of its last turn (cooldown), when it may not read the event
/// (visibility), or when nothing it may post would be accepted in this state
/// (it could not act).
pub fn wake_set(st: &TaskState) -> Vec<(RoleId, WakeReason)> {
    // Any live turn: with several allowed (`max_live_turns > 1`) this stays
    // conservative. `pick_next` calls it only for a one-live-turn task.
    if !st.opened || !st.turns.is_empty() || st.pending_request.is_some() {
        return Vec::new();
    }
    let Some(req) = REQUIREMENTS.iter().find(|q| q.state == st.state) else {
        return Vec::new();
    };
    if st.state == State::Test && st.selected_run().and_then(|r| r.status).is_none() {
        return Vec::new();
    }
    let mut woken: Vec<(WakeReason, u16, RoleId)> = st
        .roster
        .roles()
        .iter()
        .filter(|r| st.assigned.contains_key(&r.id))
        .filter_map(|r| reason(st, req, r).map(|why| (why, r.turn_order, r.id.clone())))
        .collect();
    woken.sort();
    woken.into_iter().map(|(why, _, id)| (id, why)).collect()
}

fn reason(st: &TaskState, req: &Requirement, role: &Role) -> Option<WakeReason> {
    // `last` is the end of the role's latest turn (0 if none). A role that has
    // never run has seen nothing of the messages, but nothing of the artifacts
    // from before it was assigned or before the state began either.
    let last = st.last_turn_end.get(&role.id).copied();
    let assigned = st.assigned_rev.get(&role.id).copied().unwrap_or(Rev(0));
    let serves = req.served_by(role);
    let acts = can_act(st, role);
    if acts && addressed(st, role, last.unwrap_or(Rev(0))) {
        return Some(WakeReason::Addressed);
    }
    if serves && st.ready_rev.max(assigned) > last.unwrap_or(Rev(0)) {
        return Some(WakeReason::State);
    }
    let floor = last.unwrap_or(assigned).max(st.entered_rev);
    if acts && watches_artifact(st, role, floor) {
        return Some(if serves {
            WakeReason::Artifact
        } else {
            WakeReason::Observer
        });
    }
    None
}

/// Could the role post anything but `pass` that this state admits? Uses the
/// engine's own admission rule (`admits`), so it cannot drift from `post`.
pub fn can_act(st: &TaskState, role: &Role) -> bool {
    role.post.iter().any(|k| {
        !matches!(k, MessageKind::Pass | MessageKind::Decision)
            && admits(st.state, role, *k).is_ok()
    })
}

fn addressed(st: &TaskState, role: &Role, seen: Rev) -> bool {
    st.messages.iter().zip(&st.msg_rev).any(|(m, rev)| {
        *rev > seen
            && m.draft.kind != MessageKind::Pass
            && m.from_role.as_ref() != Some(&role.id)
            && m.draft.to.iter().any(|r| match r {
                Recipient::Role(id) => *id == role.id,
                // `all` names no one: it is a broadcast, not a request to this role.
                Recipient::All => false,
                Recipient::Human => false,
            })
            && visible(st, role, m)
    })
}

fn watches_artifact(st: &TaskState, role: &Role, seen: Rev) -> bool {
    let me = st.assigned.get(&role.id);
    st.artifacts.values().any(|a| {
        st.art_rev.get(&a.id).is_some_and(|rev| *rev > seen)
            && Some(&a.author) != me
            && WAKING_ARTIFACTS.contains(&a.kind)
            && role.holds(Cap::Read(a.kind))
    })
}
