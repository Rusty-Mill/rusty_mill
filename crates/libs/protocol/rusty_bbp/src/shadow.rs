//! Shadow mode: what an event-driven policy would have done beside each real
//! grant. Returned from `handle`, never logged: a log event would change `rev`
//! and every trace. Phase 1 only, and only for tasks with one live turn, where
//! `wake_set` is defined.

use crate::ids::*;
use crate::state::{State, TurnKind};
use crate::wake::WakeReason;
use rusty_serde::{Deserialize, Serialize};

/// One shadow record, stamped with the log `rev` of the real `turn_granted`
/// event it sits beside (the first, if a schedule grants several). A grant is one
/// logged event, so a `rev` is recorded at most once and a host can dedupe on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shadow {
    pub rev: Rev,
    /// The wake rules (`WAKE_RULES`) the `would` set was computed under. A
    /// record written before this field existed has none and reads as 1.
    #[rusty_serde(default = "first_rules")]
    pub rules: u32,
    /// The task state just before the grant. A grant that crosses the turn
    /// budget escalates in the same command; this is still the state it was
    /// made in.
    pub state: State,
    pub kind: ShadowKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowKind {
    /// `schedule()` chose the holder. `granted` is who it granted; `would` is
    /// what `wake_set` returned just before, best first.
    Auto {
        granted: Vec<RoleId>,
        would: Vec<(RoleId, WakeReason)>,
    },
    /// A forced grant (yield, human `ask`, request settlement, consultation
    /// return). It bypasses `pick_next`, so its counterfactual is itself: it is
    /// reported, never compared.
    Explicit { granted: RoleId, kind: TurnKind },
}

fn first_rules() -> u32 {
    1
}

/// How an automatic grant compares with the event-driven pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// The first selectable role of `would` is the role actually granted.
    Agree,
    /// `would` has no selectable role: it is empty, or holds observers only. The
    /// event-driven rule then falls back to the state's default holder, which is
    /// the turn-based grant, so this agrees by construction; it is counted apart
    /// because it is liveness, not evidence.
    Fallback,
    /// `would` names a different selectable role first.
    Disagree,
}

impl Shadow {
    /// The comparison for an automatic grant; `None` for a forced one.
    pub fn comparison(&self) -> Option<Comparison> {
        match &self.kind {
            // Shadow records exist only at one live turn, where an observer is
            // not selectable.
            ShadowKind::Auto { granted, would } => {
                Some(match would.iter().find(|(_, why)| why.selectable(1)) {
                    None => Comparison::Fallback,
                    Some((r, _)) if granted.first() == Some(r) => Comparison::Agree,
                    Some(_) => Comparison::Disagree,
                })
            }
            ShadowKind::Explicit { .. } => None,
        }
    }
}
