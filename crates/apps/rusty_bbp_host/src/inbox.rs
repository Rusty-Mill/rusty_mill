//! `bbp inbox`: the open escalations of claims-mode tasks, for the decision
//! inbox (#595). A pure read: it folds each task's log and writes nothing.
//!
//! An escalation is open while the thing it escalated still waits for the
//! human: an `escalated` claim, an overdue action mark with no result, a
//! held rerun, or a task stopped on a task total. Its id names the task, the
//! subject and the log position of the escalating event, so a newer
//! escalation of the same claim has a different id. Every action the inbox
//! takes is fenced on `rev`.

use crate::open_driver;
use rusty_bbp::claims::{card, ClaimView, EvidenceView, Sight};
use rusty_bbp::engine::exhausted_fields;
use rusty_bbp::event::Event;
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Why the human is asked, in BBP's words (`claim_escalated` reasons, plus
/// `rerun_held` and `task_total`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Reason {
    Irreversible,
    SupersedesActed,
    Budget,
    Stuck,
    DepCollapsed,
    ReopenedAfterAction,
    ActionUnconfirmed,
    RerunHeld,
    TaskTotal,
    /// #598: the intent changed under a claim that was acted on.
    IntentChanged,
    /// #598: the host paused the task on a ceiling or the cost breaker.
    TaskPaused,
}

impl Reason {
    fn of(e: ClaimEscalation) -> Reason {
        match e {
            ClaimEscalation::Irreversible => Reason::Irreversible,
            ClaimEscalation::SupersedesActed => Reason::SupersedesActed,
            ClaimEscalation::Budget => Reason::Budget,
            ClaimEscalation::Stuck => Reason::Stuck,
            ClaimEscalation::DepCollapsed => Reason::DepCollapsed,
            ClaimEscalation::ReopenedAfterAction => Reason::ReopenedAfterAction,
            ClaimEscalation::ActionUnconfirmed(_) => Reason::ActionUnconfirmed,
            ClaimEscalation::IntentChanged => Reason::IntentChanged,
        }
    }
}

/// One open escalation, as the inbox shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Escalation {
    /// `<task>/<subject>/<seq>`: stable while the escalation is open.
    pub id: String,
    pub task: TaskId,
    /// The task's brief, which a Refine answer refs (C-R10).
    pub brief: Option<ArtId>,
    /// The task revision this was read at; act with `--fence <rev>`.
    pub rev: Rev,
    /// Position in the log of the event that opened it.
    pub seq: usize,
    pub reason: Reason,
    /// The claim's card, unfiltered (the human channel sees everything).
    pub claim: Option<ClaimView>,
    pub stakes: Option<Stakes>,
    pub reversible: Option<bool>,
    /// The strongest two support and two counter items, by `|delta|`.
    pub evidence: Vec<EvidenceView>,
    /// For a stuck question: every live candidate, side by side.
    pub candidates: Vec<ClaimView>,
    /// `action_unconfirmed`: the mark to reconcile.
    pub mark: Option<u64>,
    /// `rerun_held`: the candidate waiting on budget.
    pub candidate: Option<ArtId>,
    /// The claim and candidates shown that were acted on, with their action
    /// references: what a Reject or Choose would leave to remediate.
    pub acted: Vec<Acted>,
    /// `task_total`: each exhausted task budget field, its limit, and one
    /// allowance (the limit the task opened with).
    pub totals: Vec<Total>,
    /// `task_paused`: why the host paused it. A ceiling pause recurs on
    /// resume unless that ceiling was raised in the Factory config.
    pub pause: Option<Pause>,
}

/// Why the host paused a task (#598).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Pause {
    pub reason: PauseReason,
    pub detail: String,
}

/// A claim that was acted on: a mark with no `aborted` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Acted {
    pub claim: MsgId,
    /// The references of its marks that were not aborted (a PR, a deploy id).
    pub marks: Vec<String>,
}

/// One exhausted task budget field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Total {
    pub field: BudgetField,
    pub spent: u64,
    pub limit: u64,
    /// One allowance: the field's limit when the task opened.
    pub allowance: u64,
}

/// The open escalations of `tasks` in `dir`: irreversible first, then task
/// by task in the order given, oldest first within each task. A log position
/// orders only its own task's events, and the log records no time for an
/// escalation, so no age is compared across tasks.
pub fn read(dir: &Path, tasks: &[TaskId]) -> Result<Vec<Escalation>, String> {
    let mut out = Vec::new();
    for (n, task) in tasks.iter().enumerate() {
        let d = open_driver(dir, task)?;
        let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
        out.extend(open(&d.state, &events).into_iter().map(|e| (n, e)));
    }
    out.sort_by_key(|(n, e)| (e.reversible != Some(false), *n, e.seq, e.id.clone()));
    Ok(out.into_iter().map(|(_, e)| e).collect())
}

/// The JSON `bbp inbox` prints.
pub fn json(dir: &Path, tasks: &[TaskId]) -> Result<String, String> {
    rusty_serde::json::to_string(&read(dir, tasks)?).map_err(|e| e.to_string())
}

/// The open escalations of one task, from its state and its log.
/// An ended task has none: its human verbs are refused, so nothing in it is actionable.
pub fn open(st: &TaskState, events: &[Event]) -> Vec<Escalation> {
    if st.mode != Mode::Claims || st.state.terminal() {
        return vec![];
    }
    let mut claim_at: BTreeMap<MsgId, (usize, ClaimEscalation)> = BTreeMap::new();
    let mut mark_at: BTreeMap<u64, (usize, MsgId)> = BTreeMap::new();
    let mut held_at: BTreeMap<ArtId, usize> = BTreeMap::new();
    // The latest task-level stop: a task total or a host pause.
    let mut task_at: Option<(usize, Option<Pause>)> = None;
    for (seq, e) in events.iter().enumerate() {
        match e {
            Event::ClaimEscalated {
                claim,
                reason: ClaimEscalation::ActionUnconfirmed(mark),
            } => {
                mark_at.insert(*mark, (seq, *claim));
            }
            Event::ClaimEscalated { claim, reason } => {
                claim_at.insert(*claim, (seq, *reason));
            }
            Event::RerunHeld { candidate, .. } => {
                held_at.insert(*candidate, seq);
            }
            Event::Escalated { .. } => task_at = Some((seq, None)),
            Event::TaskPaused { reason, detail } => {
                task_at = Some((
                    seq,
                    Some(Pause {
                        reason: *reason,
                        detail: detail.clone(),
                    }),
                ))
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (c, (seq, why)) in &claim_at {
        if st
            .claims
            .get(*c)
            .is_some_and(|r| r.status == ClaimStatus::Escalated)
        {
            out.push(of_claim(st, *c, *seq, Reason::of(*why)));
        }
    }
    for (m, (seq, c)) in &mark_at {
        let pending = st
            .claims
            .get(*c)
            .and_then(|r| r.marks.iter().find(|x| x.id == *m))
            .is_some_and(|x| x.outcome.is_none());
        if pending {
            let mut e = of_claim(st, *c, *seq, Reason::ActionUnconfirmed);
            e.id = format!("{}/mark:{m}/{seq}", st.task.0);
            e.mark = Some(*m);
            out.push(e);
        }
    }
    for (k, c) in &st.claims.held {
        let seq = held_at.get(k).copied().unwrap_or(0);
        let mut e = of_claim(st, *c, seq, Reason::RerunHeld);
        e.id = format!("{}/{k}/{seq}", st.task.0);
        e.candidate = Some(*k);
        out.push(e);
    }
    if let (State::Escalated, Some((seq, pause))) = (st.state, task_at) {
        let opened = events.iter().find_map(|e| match e {
            Event::TaskOpened { budget, .. } => Some(*budget),
            _ => None,
        });
        out.push(Escalation {
            id: format!("{}/task/{seq}", st.task.0),
            task: st.task.clone(),
            brief: st.brief,
            rev: st.rev,
            seq,
            reason: match pause {
                Some(_) => Reason::TaskPaused,
                None => Reason::TaskTotal,
            },
            claim: None,
            stakes: None,
            reversible: None,
            evidence: vec![],
            candidates: vec![],
            mark: None,
            candidate: None,
            acted: vec![],
            totals: totals(st, opened.unwrap_or(st.budget)),
            pause,
        });
    }
    out
}

fn totals(st: &TaskState, opened: Budget) -> Vec<Total> {
    exhausted_fields(st)
        .into_iter()
        .map(|field| Total {
            field,
            spent: match field {
                BudgetField::Messages => st.spent.messages,
                BudgetField::Bytes => st.spent.bytes,
                BudgetField::Reads => st.spent.reads,
                BudgetField::ReadsPerTurn => 0,
                BudgetField::Turns => st.spent.turns,
                BudgetField::Iterations => st.iteration,
            },
            limit: st.budget.get(field),
            allowance: opened.get(field),
        })
        .collect()
}

/// Which of `ids` were acted on, with their live action references.
fn acted(st: &TaskState, ids: impl Iterator<Item = MsgId>) -> Vec<Acted> {
    ids.filter_map(|c| st.claims.get(c))
        .filter(|r| r.acted_on())
        .map(|r| Acted {
            claim: r.id,
            marks: r
                .marks
                .iter()
                .filter(|m| m.outcome != Some(MarkOutcome::Aborted))
                .map(|m| m.reference.clone())
                .collect(),
        })
        .collect()
}

fn of_claim(st: &TaskState, c: MsgId, seq: usize, reason: Reason) -> Escalation {
    let rec = st.claims.get(c);
    let view = rec.map(|r| card(st, r, &Sight::all()));
    let evidence = view.as_ref().map(strongest).unwrap_or_default();
    let candidates = match (reason, &view) {
        (Reason::Stuck, Some(v)) => live_candidates(st, &v.candidates),
        _ => vec![],
    };
    let shown = std::iter::once(c).chain(candidates.iter().map(|k| k.id));
    let acted = acted(st, shown);
    Escalation {
        id: format!("{}/{c}/{seq}", st.task.0),
        task: st.task.clone(),
        brief: st.brief,
        rev: st.rev,
        seq,
        reason,
        claim: view,
        stakes: rec.map(|r| r.body.stakes),
        reversible: rec.map(|r| r.body.reversible),
        evidence,
        candidates,
        mark: None,
        candidate: None,
        acted,
        totals: vec![],
        pause: None,
    }
}

/// The two strongest scoring items each way from the claim's current epoch,
/// by `|delta|`, newest first on ties. Evidence from before a reopen is history.
fn strongest(v: &ClaimView) -> Vec<EvidenceView> {
    let side = |d: Direction| {
        let mut items: Vec<&EvidenceView> = v
            .evidence
            .iter()
            .filter(|i| i.direction == d && i.scoring && i.epoch == v.epoch)
            .collect();
        items.sort_by_key(|i| {
            (
                std::cmp::Reverse(i.delta.unsigned_abs()),
                std::cmp::Reverse(i.msg),
            )
        });
        items.into_iter().take(2).cloned().collect::<Vec<_>>()
    };
    let mut out = side(Direction::Support);
    out.extend(side(Direction::Counter));
    out
}

fn live_candidates(st: &TaskState, ids: &[MsgId]) -> Vec<ClaimView> {
    ids.iter()
        .filter_map(|id| st.claims.get(*id))
        .filter(|r| {
            matches!(
                r.status,
                ClaimStatus::Open | ClaimStatus::Accepted | ClaimStatus::Escalated
            )
        })
        .map(|r| card(st, r, &Sight::all()))
        .collect()
}
