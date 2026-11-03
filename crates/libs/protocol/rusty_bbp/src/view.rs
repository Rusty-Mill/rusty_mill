//! Read-only host projection. No tokens, master key, or runner secrets.
use crate::*;
use rusty_serde::{Deserialize, Serialize};

/// A consistent, structured human view of one task, folded from one event prefix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    pub card: Card,
    pub repo: String,
    pub roster: Vec<Role>,
    pub principals: Vec<Principal>,
    pub proposal: Option<ArtId>,
    /// The snapshot of `proposal` when it is a co-edited version.
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_snapshot: Option<Sha256>,
    pub request: Option<Message>,
    pub receipt: Option<(ArtId, String)>,
    pub cancelled: Option<String>,
    pub escalation: Option<EscalateReason>,
}
impl TaskView {
    /// Fold public records only; never serialize TaskState's secret material.
    pub fn from_events(task: TaskId, events: &[Event]) -> Self {
        let st = TaskState::fold(task, events);
        let mut principals: Vec<_> = st
            .assigned
            .values()
            .filter_map(|p| st.principals.get(p).cloned())
            .collect();
        principals.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Self {
            card: st.card(),
            repo: st.repo.clone(),
            roster: st.roster.roles().to_vec(),
            principals,
            proposal: st.gate_request.map(|(_, art)| art),
            proposal_snapshot: st
                .gate_request
                .and_then(|(_, art)| st.versions.get(&art))
                .map(|v| v.snapshot),
            request: st
                .pending_request
                .as_ref()
                .and_then(|p| st.message(p.msg))
                .cloned(),
            receipt: events.iter().rev().find_map(|e| match e {
                Event::MergeReceipt {
                    candidate,
                    revision,
                } => Some((*candidate, revision.clone())),
                _ => None,
            }),
            cancelled: events.iter().rev().find_map(|e| match e {
                Event::Cancelled { reason } => Some(reason.clone()),
                _ => None,
            }),
            escalation: events.iter().rev().find_map(|e| match e {
                Event::Escalated { reason, .. } => Some(*reason),
                _ => None,
            }),
        }
    }
}
