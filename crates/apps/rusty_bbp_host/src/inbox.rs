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
        }
    }
}

/// One open escalation, as the inbox shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Escalation {
    /// `<task>/<subject>/<seq>`: stable while the escalation is open.
    pub id: String,
    pub task: TaskId,
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
}

/// The open escalations of `tasks` in `dir`: irreversible first, then oldest.
pub fn read(dir: &Path, tasks: &[TaskId]) -> Result<Vec<Escalation>, String> {
    let mut out = Vec::new();
    for task in tasks {
        let d = open_driver(dir, task)?;
        let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
        out.extend(open(&d.state, &events));
    }
    out.sort_by_key(|e| (e.reversible != Some(false), e.seq, e.id.clone()));
    Ok(out)
}

/// The JSON `bbp inbox` prints.
pub fn json(dir: &Path, tasks: &[TaskId]) -> Result<String, String> {
    rusty_serde::json::to_string(&read(dir, tasks)?).map_err(|e| e.to_string())
}

/// The open escalations of one task, from its state and its log.
pub fn open(st: &TaskState, events: &[Event]) -> Vec<Escalation> {
    if st.mode != Mode::Claims {
        return vec![];
    }
    let mut claim_at: BTreeMap<MsgId, (usize, ClaimEscalation)> = BTreeMap::new();
    let mut mark_at: BTreeMap<u64, (usize, MsgId)> = BTreeMap::new();
    let mut held_at: BTreeMap<ArtId, usize> = BTreeMap::new();
    let mut task_at = None;
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
            Event::Escalated { .. } => task_at = Some(seq),
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
    if let (State::Escalated, Some(seq)) = (st.state, task_at) {
        out.push(Escalation {
            id: format!("{}/task/{seq}", st.task.0),
            task: st.task.clone(),
            rev: st.rev,
            seq,
            reason: Reason::TaskTotal,
            claim: None,
            stakes: None,
            reversible: None,
            evidence: vec![],
            candidates: vec![],
            mark: None,
            candidate: None,
        });
    }
    out
}

fn of_claim(st: &TaskState, c: MsgId, seq: usize, reason: Reason) -> Escalation {
    let rec = st.claims.get(c);
    let view = rec.map(|r| card(st, r, &Sight::all()));
    let evidence = view.as_ref().map(strongest).unwrap_or_default();
    let candidates = match (reason, &view) {
        (Reason::Stuck, Some(v)) => live_candidates(st, &v.candidates),
        _ => vec![],
    };
    Escalation {
        id: format!("{}/{c}/{seq}", st.task.0),
        task: st.task.clone(),
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
    }
}

/// The two strongest scoring items each way, by `|delta|`, newest first on ties.
fn strongest(v: &ClaimView) -> Vec<EvidenceView> {
    let side = |d: Direction| {
        let mut items: Vec<&EvidenceView> = v
            .evidence
            .iter()
            .filter(|i| i.direction == d && i.scoring)
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
