//! Decision replay (#599): how one claim moved, event by event, and the rule
//! behind each change. Pure and read-only: the log is folded with the same
//! [`TaskState::apply`] the store uses, and nothing is written.
//!
//! The log records what happened, not which rule fired, so the rule is read
//! from the events around a change:
//!
//! - Every human command ends with its `op_accepted`, so the events between
//!   two of them are one command. A `claim_accepted` or `claim_dropped` whose
//!   command also decided that outcome for that claim is the human's
//!   ([`Rule::Human`]); otherwise it is the accept or drop line, with the
//!   numbers it compared taken from the state just before. A human reopen
//!   that the line then settles is the line's.
//! - A command that decided `accept` on one candidate of a question and `drop`
//!   on another is a `choose` ([`Rule::Choose`]). A `choose` that finds no
//!   other live candidate writes only the accept and reads as a decide.
//! - Escalations, reopenings, voids and supersessions name their cause in the
//!   event.
//!
//! Steps come from the claim's own message, its evidence messages, the
//! status and `claim_failing` events naming it, each build candidate that
//! becomes its current one, and each final holdout report that speaks for
//! the current code (D112). Budget, run-debit and action mark events change
//! no score or status and make no step.

use crate::claims::{
    confidence_pct, ClaimBody, ClaimEscalation, ClaimOutcome, ClaimRecord, ClaimStatus, Direction,
    EvidenceKind, ReopenCause,
};
use crate::claims_engine::{builds_current, manifest, on_tested_tree, report_of};
use crate::event::Event;
use crate::ids::{ArtId, MsgId, PrincipalId, Rev, TaskId};
use crate::record::{ArtifactPayload, Body, Ref, Suite};
use crate::state::TaskState;
use rusty_serde::Serialize;
use std::collections::BTreeMap;

/// One claim's replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct History {
    pub claim: MsgId,
    pub author: PrincipalId,
    pub body: ClaimBody,
    pub steps: Vec<Step>,
}

/// One event that changed the claim, and the claim just after it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The rev of the event.
    pub rev: Rev,
    pub change: Change,
    pub status: ClaimStatus,
    pub score: i64,
    /// [`confidence_pct`] of `score`, for JSON readers; no rule reads it.
    pub confidence: u32,
    pub epoch: u32,
}

/// What the event did to the claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Change {
    Posted,
    /// An evidence message on the claim. `scoring` is as appended: evidence on
    /// a settled claim is recorded only, and a later reopen can make it count
    /// (the reopen step's `score` shows that).
    Evidence {
        msg: MsgId,
        author: PrincipalId,
        kind: EvidenceKind,
        direction: Direction,
        delta: i64,
        scoring: bool,
        /// The refs the message cites, as posted: an executed item names
        /// the report it read.
        refs: Vec<Ref>,
    },
    Status {
        from: ClaimStatus,
        rule: Rule,
        /// The earlier items a reopen made scoring in its new epoch: the
        /// reopening evidence, or the dormant items a human reopen woke.
        /// Empty for any other change.
        #[rusty_serde(default, skip_serializing_if = "Vec::is_empty")]
        woke: Vec<MsgId>,
    },
    /// The profiles failing on the current candidate; non-empty blocks accept.
    Failing {
        profiles: Vec<String>,
        /// The current candidate the set is on: the code its latest report
        /// tested (a holdout report tests a holdout candidate of that code).
        candidate: Option<ArtId>,
    },
    /// A build candidate naming the claim became its current one.
    Candidate {
        candidate: ArtId,
    },
    /// A final holdout report that speaks for the current code: a holdout
    /// candidate of `candidate`'s code, run under the task's holdout suite
    /// on the tree `candidate` was tested on. Its profiles, passed and failed.
    Report {
        report: ArtId,
        candidate: ArtId,
        passed: Vec<String>,
        failed: Vec<String>,
    },
}

/// The rule that changed a claim's status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Rule {
    /// Score reached the accept line with enough distinct supporting authors.
    AcceptLine {
        accept_at: i64,
        support_authors: u32,
        /// The task's distinct-author minimum the line compared against.
        min_authors: u32,
    },
    /// Score reached the drop line with enough distinct countering authors.
    DropLine {
        drop_at: i64,
        counter_authors: u32,
        min_authors: u32,
    },
    /// A human `decide` accept (or drop) on this claim.
    Human {
        accept: bool,
    },
    /// A human `choose` on the claim's question.
    Choose {
        question: MsgId,
    },
    Escalation(ClaimEscalation),
    Reopen(ReopenCause),
    /// A dep closed.
    Void {
        by: MsgId,
    },
    /// An accepted claim replaced it.
    Superseded {
        by: MsgId,
    },
}

/// Replay `claim` over `events` (a whole task log, from `task_opened`), through
/// rev `at` if given. `None` if the claim was not posted by then.
pub fn claim_history(
    task: &TaskId,
    events: &[Event],
    claim: MsgId,
    at: Option<Rev>,
) -> Option<History> {
    let mut st = TaskState::fold(task.clone(), &[]);
    let mut steps = Vec::new();
    let mut decided = BTreeMap::new();
    for (i, e) in events.iter().enumerate() {
        if at.is_some_and(|at| st.rev >= at) {
            break;
        }
        if command_starts(events, i) {
            decided = decisions(&st, &events[i..]);
        }
        let named = touches(e, claim, &st);
        let before = named.then(|| st.claims.get(claim).cloned()).flatten();
        let rule = rule(e, before.as_ref(), &st, &decided);
        st.apply(e);
        let Some(after) = named.then(|| st.claims.get(claim)).flatten() else {
            continue;
        };
        if let Some(change) = change(e, before.as_ref(), after, rule, &st) {
            steps.push(step(st.rev, change, after));
        }
    }
    let rec = st.claims.get(claim)?;
    Some(History {
        claim,
        author: rec.author.clone(),
        body: rec.body.clone(),
        steps,
    })
}

fn step(rev: Rev, change: Change, c: &ClaimRecord) -> Step {
    let score = c.score();
    Step {
        rev,
        change,
        status: c.status,
        score,
        confidence: confidence_pct(score),
        epoch: c.epoch,
    }
}

/// Event `i` is the first of a command: the log's first, or the one after a
/// command's closing `op_accepted` or `rejected`.
/// Per claim, the human outcome a command decided and the rule its effect gets.
type Decided = BTreeMap<MsgId, (ClaimOutcome, Rule)>;

fn command_starts(events: &[Event], i: usize) -> bool {
    i == 0
        || matches!(
            events.get(i - 1),
            Some(Event::OpAccepted { .. } | Event::Rejected { .. })
        )
}

/// The accept and drop decisions of the command starting at `rest[0]`: the
/// outcome and the rule its effect gets, `Choose` for both sides of a choose,
/// else `Human`. A reopen is left out: its event names its cause, and the
/// accept or drop that may follow it is the line's.
fn decisions(st: &TaskState, rest: &[Event]) -> Decided {
    let mut picks: Vec<(MsgId, ClaimOutcome)> = Vec::new();
    for e in rest {
        match e {
            Event::ClaimDecided { claim, outcome } => picks.push((*claim, *outcome)),
            Event::OpAccepted { .. } | Event::Rejected { .. } => break,
            _ => {}
        }
    }
    let question = |c: MsgId| st.claims.get(c).and_then(|r| r.body.answers);
    let chose = |q: MsgId, o: ClaimOutcome| {
        picks
            .iter()
            .any(|(c, x)| *x == o && question(*c) == Some(q))
    };
    picks
        .iter()
        .filter(|(_, o)| *o != ClaimOutcome::Reopen)
        .map(|(c, o)| {
            let rule = match question(*c) {
                Some(q) if chose(q, ClaimOutcome::Accept) && chose(q, ClaimOutcome::Drop) => {
                    Rule::Choose { question: q }
                }
                _ => Rule::Human {
                    accept: *o == ClaimOutcome::Accept,
                },
            };
            (*c, (*o, rule))
        })
        .collect()
}

/// The event names `claim`. Read before the event is applied, so a report
/// names it through its already stored candidate.
fn touches(e: &Event, claim: MsgId, st: &TaskState) -> bool {
    match e {
        Event::ArtifactStored(a) => {
            matches!(&a.payload, ArtifactPayload::Candidate(k) if k.claim == Some(claim))
        }
        Event::TestReportStored { candidate, .. } => {
            st.claims.candidates.get(candidate) == Some(&claim)
        }
        Event::MessageAppended(m) => match &m.draft.body {
            Body::Claim(_) => m.id == claim,
            Body::Evidence(b) => b.claim == claim,
            _ => false,
        },
        Event::ClaimAccepted { claim: c, .. }
        | Event::ClaimDropped { claim: c, .. }
        | Event::ClaimVoided { claim: c, .. }
        | Event::ClaimEscalated { claim: c, .. }
        | Event::ClaimReopened { claim: c, .. }
        | Event::ClaimSuperseded { claim: c, .. }
        | Event::ClaimFailing { claim: c, .. } => *c == claim,
        _ => false,
    }
}

/// The rule behind a status event, read before it is applied.
fn rule(
    e: &Event,
    before: Option<&ClaimRecord>,
    st: &TaskState,
    decided: &Decided,
) -> Option<Rule> {
    let c = before?;
    let cfg = &st.claims.config;
    let count = |d: Direction| u32::try_from(c.authors(d)).unwrap_or(u32::MAX);
    let human = |claim: &MsgId, want: ClaimOutcome| {
        decided
            .get(claim)
            .filter(|(o, _)| *o == want)
            .map(|(_, r)| r.clone())
    };
    match e {
        Event::ClaimAccepted { claim, .. } => Some(human(claim, ClaimOutcome::Accept).unwrap_or(
            Rule::AcceptLine {
                accept_at: c.accept_at(cfg),
                support_authors: count(Direction::Support),
                min_authors: cfg.min_authors,
            },
        )),
        Event::ClaimDropped { claim, .. } => {
            Some(human(claim, ClaimOutcome::Drop).unwrap_or(Rule::DropLine {
                drop_at: cfg.drop_at,
                counter_authors: count(Direction::Counter),
                min_authors: cfg.min_authors,
            }))
        }
        Event::ClaimVoided { by, .. } => Some(Rule::Void { by: *by }),
        Event::ClaimEscalated { reason, .. } => Some(Rule::Escalation(*reason)),
        Event::ClaimReopened { by, .. } => Some(Rule::Reopen(*by)),
        Event::ClaimSuperseded { by, .. } => Some(Rule::Superseded { by: *by }),
        _ => None,
    }
}

/// The step's change, from the event and the claim before and after it.
/// `st` is the state just after the event.
fn change(
    e: &Event,
    before: Option<&ClaimRecord>,
    after: &ClaimRecord,
    rule: Option<Rule>,
    st: &TaskState,
) -> Option<Change> {
    match e {
        Event::MessageAppended(m) => match &m.draft.body {
            Body::Claim(_) => Some(Change::Posted),
            Body::Evidence(b) => {
                let item = after.items.iter().find(|i| i.msg == m.id)?;
                Some(Change::Evidence {
                    msg: m.id,
                    author: item.author.clone(),
                    kind: b.kind,
                    direction: b.direction,
                    delta: item.delta,
                    scoring: item.scoring,
                    refs: m.draft.evidence.clone(),
                })
            }
            _ => None,
        },
        Event::ClaimFailing { failing, .. } => Some(Change::Failing {
            profiles: failing.clone(),
            candidate: after.current,
        }),
        Event::ArtifactStored(_) => {
            let current = after.current?;
            (before?.current != Some(current)).then_some(Change::Candidate { candidate: current })
        }
        Event::TestReportStored {
            candidate, report, ..
        } => holdout_report(st, after, *candidate, *report),
        _ => {
            let before = before?;
            Some(Change::Status {
                from: before.status,
                rule: rule?,
                woke: woke(before, after),
            })
        }
    }
}

/// The items a reopen made scoring in its new epoch. Reopening moves only
/// the items it wakes to the new epoch, and items are only ever appended.
fn woke(before: &ClaimRecord, after: &ClaimRecord) -> Vec<MsgId> {
    if after.epoch == before.epoch {
        return Vec::new();
    }
    before
        .items
        .iter()
        .zip(&after.items)
        .filter(|(b, a)| a.scoring && a.epoch == after.epoch && b.epoch != a.epoch)
        .map(|(_, a)| a.msg)
        .collect()
}

/// A [`Change::Report`] for `report` of holdout candidate `k`, if it speaks
/// for `c`'s current code under the task's holdout suite.
fn holdout_report(st: &TaskState, c: &ClaimRecord, k: ArtId, report: ArtId) -> Option<Change> {
    let current = c.current?;
    let rep = report_of(st, report)?;
    let speaks = manifest(st, k).is_some_and(|m| m.suite == Suite::Holdout)
        && Some(rep.profile_digest) == st.claims.holdout_digest
        && builds_current(st, c, k)
        && on_tested_tree(st, c.id, k, rep);
    if !speaks {
        return None;
    }
    let failed = rep.failed_profiles();
    let passed = rep
        .listed_profiles()
        .into_iter()
        .filter(|p| !failed.contains(p))
        .collect();
    Some(Change::Report {
        report,
        candidate: current,
        passed,
        failed,
    })
}
