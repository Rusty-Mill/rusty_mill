//! Event-driven scheduling, phase 1: which roles the board state makes
//! eligible for the next turn. Pure. Nothing here grants a turn; the
//! scheduling check in `engine.rs` stays the only thing that does.
//!
//! Eligibility is derived from the nine Role fields, so no role is named.
//! Three triggers, all state-derived:
//!
//! - **Addressed**: a message that names the role (or everyone) arrived since
//!   the role's last turn ended, and the role may read it.
//! - **Artifact**: a spec, diff, candidate or test report the role may read
//!   was stored since its last turn ended.
//! - **State**: the role serves the current working state and that state's
//!   turn became available since the role's last turn ended.
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

/// Why a role is eligible. Declaration order is priority order: an addressed
/// role outranks an artifact watcher, which outranks the state's own holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WakeReason {
    Addressed,
    Artifact,
    State,
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
    // conservative, and nothing calls it yet, so it grants no extra slots.
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
    let seen = st.last_turn_end.get(&role.id).copied().unwrap_or(Rev(0));
    if can_act(st, role) {
        if addressed(st, role, seen) {
            return Some(WakeReason::Addressed);
        }
        if watches_artifact(st, role, seen) {
            return Some(WakeReason::Artifact);
        }
    }
    (req.served_by(role) && st.ready_rev > seen).then_some(WakeReason::State)
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
                Recipient::All => true,
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
