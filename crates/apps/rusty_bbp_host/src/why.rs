//! `bbp why`: why each turn was granted, replayed from the task's log.
//!
//! The log records who was granted a turn, not why. The reason is a pure
//! function of the board just before the grant, so it is recomputed: the log is
//! folded to the event before each `turn_granted`, `wake_set` runs on that
//! state, and the policy's pick rule is applied to it. Read-only: nothing is
//! written, and the core is used as it is.
//!
//! The pick rule is the core's `pick_next`, which is crate-private there, so it
//! is restated here (a few lines). A grant the restated rule does not
//! reproduce is reported as [`How::Unexplained`], never as a match, so a drift
//! between the two shows up in the output and in `tests/why.rs`.
//!
//! Grants that bypass the policy (a settled request, a yield or `ask`, the
//! return from a consultation) are recognised from the events around them and
//! labelled [`How::Explicit`].

use crate::open_driver;
use crate::shadow;
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One entry of `wake_set`: a role and why the board made it eligible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Woken {
    pub role: RoleId,
    pub reason: WakeReason,
}

/// A grant the scheduling policy did not make.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Cause {
    /// A request was settled (a human answer or decision) and its requester regranted.
    Settlement,
    /// A consultation turn: a yield, an `ask`, or a settlement to a role that is not the default holder.
    Consultation { return_to: Option<RoleId> },
    /// The consulted role's turn ended and the turn went back to the role that asked.
    ConsultationReturn,
}

/// How the grant was decided.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum How {
    /// `EventDriven`: the first selectable role of `wake_set`, with its reason.
    WakeSet(WakeReason),
    /// `EventDriven`: nothing selectable, so the `TurnBased` choice, the default holder.
    Fallback,
    /// `TurnBased`: the default holder of the state (shadow mode compares it).
    DefaultHolder,
    /// Forced; the policy was not consulted.
    Explicit(Cause),
    /// The restated rule would have picked another role (or none). Not a match.
    Unexplained { replayed: Option<RoleId> },
}

/// A `TurnBased` grant beside what the event-driven rule would have done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shadowed {
    /// `wake_set` just before the grant, best first.
    pub would: Vec<Woken>,
    /// `agree`, `fallback` (no selectable role) or `disagree`.
    pub comparison: &'static str,
    /// The wake rules version of the record `bbp shadow` kept for this grant, if any.
    #[rusty_serde(skip_serializing_if = "Option::is_none")]
    pub logged_rules: Option<u32>,
    /// Whether that record's set equals the replayed one (it can differ if the rules changed).
    #[rusty_serde(skip_serializing_if = "Option::is_none")]
    pub logged_same: Option<bool>,
}

/// One `turn_granted`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Grant {
    pub turn: u64,
    /// The rev of the `turn_granted` event.
    pub rev: u64,
    pub state: State,
    pub role: RoleId,
    pub kind: TurnKind,
    pub how: How,
    /// `EventDriven` only: `wake_set` just before the grant, best first.
    #[rusty_serde(skip_serializing_if = "Option::is_none")]
    pub wake_set: Option<Vec<Woken>>,
    /// `TurnBased` with one live turn only, and not for a forced grant.
    #[rusty_serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadowed>,
}

/// What `bbp why` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub task: String,
    pub policy: SchedulingPolicy,
    /// The `WAKE_RULES` of the build that replayed, not of the one that ran.
    pub wake_rules: u32,
    pub grants: Vec<Grant>,
}

fn woken(set: Vec<(RoleId, WakeReason)>) -> Vec<Woken> {
    set.into_iter()
        .map(|(role, reason)| Woken { role, reason })
        .collect()
}

/// A forced grant, from the events around it. The order the core emits them in:
/// a settlement is `request_settled` then the grant; a consultation is a grant
/// of kind `Consultation`; its return is the end of that turn then a `Default`
/// grant to the role it named.
fn forced(
    role: &RoleId,
    kind: TurnKind,
    prev: Option<&Event>,
    turns: &BTreeMap<TurnId, (TurnKind, Option<RoleId>)>,
    return_to: &Option<RoleId>,
) -> Option<Cause> {
    if matches!(prev, Some(Event::RequestSettled { .. })) {
        return Some(Cause::Settlement);
    }
    if kind == TurnKind::Consultation {
        return Some(Cause::Consultation {
            return_to: return_to.clone(),
        });
    }
    if let Some(Event::TurnEnded { turn, .. }) = prev {
        if let Some((TurnKind::Consultation, Some(back))) = turns.get(turn) {
            if back == role {
                return Some(Cause::ConsultationReturn);
            }
        }
    }
    None
}

/// Explain one automatic grant of `role` against the state `pre` just before it.
fn automatic(
    pre: &TaskState,
    role: &RoleId,
    logged: &[Shadow],
    rev: u64,
) -> (How, Option<Vec<Woken>>, Option<Shadowed>) {
    let serving: Vec<RoleId> = pre.roster.serving(pre.state).into_iter().cloned().collect();
    let first = serving.first().cloned();
    let one_live = pre.max_live_turns == 1;
    let raw = if one_live { wake_set(pre) } else { Vec::new() };
    match pre.policy {
        SchedulingPolicy::EventDriven => {
            let pick = raw
                .iter()
                .find(|(_, why)| why.selectable(pre.max_live_turns));
            let (expected, how) = match pick {
                Some((r, why)) => (Some(r.clone()), How::WakeSet(*why)),
                None => (first, How::Fallback),
            };
            let how = if expected.as_ref() == Some(role) {
                how
            } else {
                How::Unexplained { replayed: expected }
            };
            (how, Some(woken(raw)), None)
        }
        SchedulingPolicy::TurnBased => {
            let ok = if one_live {
                first.as_ref() == Some(role)
            } else {
                serving.contains(role)
            };
            let how = if ok {
                How::DefaultHolder
            } else {
                How::Unexplained { replayed: first }
            };
            // Shadow mode exists only at one live turn.
            let shadowed = one_live.then(|| {
                let record = Shadow {
                    rev: Rev(rev),
                    rules: WAKE_RULES,
                    state: pre.state,
                    kind: ShadowKind::Auto {
                        granted: vec![role.clone()],
                        would: raw.clone(),
                    },
                };
                let comparison = match record.comparison() {
                    Some(Comparison::Agree) => "agree",
                    Some(Comparison::Disagree) => "disagree",
                    Some(Comparison::Fallback) | None => "fallback",
                };
                let kept = logged.iter().find(|s| s.rev.0 == rev);
                Shadowed {
                    would: woken(raw.clone()),
                    comparison,
                    logged_rules: kept.map(|s| s.rules),
                    logged_same: kept.map(
                        |s| matches!(&s.kind, ShadowKind::Auto { would, .. } if *would == raw),
                    ),
                }
            });
            (how, None, shadowed)
        }
    }
}

/// Replay `events` and explain every grant. `logged` are the shadow records the
/// host kept for the task (empty if there are none).
pub fn explain(task: &TaskId, events: &[Event], logged: &[Shadow]) -> Report {
    let mut st = TaskState::fold(task.clone(), &[]);
    let mut turns: BTreeMap<TurnId, (TurnKind, Option<RoleId>)> = BTreeMap::new();
    let mut grants = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if let Event::TurnGranted {
            role,
            turn,
            kind,
            return_to,
            ..
        } = e
        {
            let prev = i.checked_sub(1).and_then(|p| events.get(p));
            let rev = st.rev.0 + 1;
            let (how, wake_set, shadow) = match forced(role, *kind, prev, &turns, return_to) {
                Some(cause) => (How::Explicit(cause), None, None),
                None => automatic(&st, role, logged, rev),
            };
            grants.push(Grant {
                turn: turn.0,
                rev,
                state: st.state,
                role: role.clone(),
                kind: *kind,
                how,
                wake_set,
                shadow,
            });
            turns.insert(*turn, (*kind, return_to.clone()));
        }
        st.apply(e);
    }
    Report {
        task: task.0.clone(),
        policy: st.policy,
        wake_rules: WAKE_RULES,
        grants,
    }
}

/// Read `task`'s log (and its shadow records) and explain its grants; only turn
/// `turn` if given.
pub fn of(dir: &Path, task: &TaskId, turn: Option<u64>) -> Result<Report, String> {
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let logged = shadow::read(dir, task)?.records;
    let mut report = explain(task, &events, &logged);
    if let Some(n) = turn {
        let total = report.grants.len();
        report.grants.retain(|g| g.turn == n);
        if report.grants.is_empty() {
            return Err(format!("no turn {n}: the task has {total} grants"));
        }
    }
    Ok(report)
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId, turn: Option<u64>) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task, turn)?).map_err(|e| e.to_string())
}
