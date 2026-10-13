//! Shadow mode: what an event-driven policy would have done beside each real
//! grant. Returned from `handle`, never logged: a log event would change `rev`
//! and every trace. Phase 1 only, and only for tasks with one live turn, where
//! `wake_set` is defined.

use crate::ids::*;
use crate::state::{State, TurnKind};
use crate::wake::WakeReason;

/// One shadow record, stamped with the log `rev` after the real grant it sits
/// beside. A grant is one logged `turn_granted`, so a `rev` is recorded at most
/// once and a host can dedupe on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub rev: Rev,
    /// The task state the grant was made in.
    pub state: State,
    pub kind: ShadowKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

/// How an automatic grant compares with the event-driven pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// The first of `would` is the role actually granted.
    Agree,
    /// `would` is empty. The event-driven rule then falls back to the state's
    /// default holder, which is the turn-based grant, so this agrees by
    /// construction; it is counted apart because it is liveness, not evidence.
    Fallback,
    /// `would` names a different role first.
    Disagree,
}

impl Shadow {
    /// The comparison for an automatic grant; `None` for a forced one.
    pub fn comparison(&self) -> Option<Comparison> {
        match &self.kind {
            ShadowKind::Auto { granted, would } => Some(match would.first() {
                None => Comparison::Fallback,
                Some((r, _)) if granted.first() == Some(r) => Comparison::Agree,
                Some(_) => Comparison::Disagree,
            }),
            ShadowKind::Explicit { .. } => None,
        }
    }
}
