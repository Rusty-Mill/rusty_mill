//! Claims mode (addendum `bbp-claims-addendum.md`): claim and evidence records,
//! the per-claim book the fold keeps, and the pure arithmetic of the update
//! rule. Pure: no I/O, no clock, no floats.

use crate::ids::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How a task runs: the stage pipeline, or claims.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Stages,
    Claims,
}

impl Mode {
    pub fn is_stages(&self) -> bool {
        *self == Mode::Stages
    }
}

/// The numbers of the update rule, fixed at open. The defaults are the owner's
/// (2026-11-02); they are judgment, not data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimsConfig {
    pub accept_at: i64,
    pub accept_at_strict: i64,
    pub drop_at: i64,
    pub min_authors: u32,
    pub settle_window: u32,
    pub settle_delta: i64,
    pub evidence_base: u64,
    pub mark_timeout_secs: u64,
}

impl Default for ClaimsConfig {
    fn default() -> Self {
        ClaimsConfig {
            accept_at: 8,
            accept_at_strict: 12,
            drop_at: -8,
            min_authors: 2,
            settle_window: 6,
            settle_delta: 2,
            evidence_base: 8,
            mark_timeout_secs: 3600,
        }
    }
}

impl ClaimsConfig {
    /// The largest `evidence_base`: with the stakes (×4) and reversibility
    /// (×2) factors, every budget and extension stays far inside `u64`.
    pub const MAX_EVIDENCE_BASE: u64 = 1 << 20;

    /// The first field that breaks the open-time rules, if any.
    pub fn invalid_field(&self) -> Option<&'static str> {
        if self.accept_at <= 0 {
            return Some("accept_at");
        }
        if self.accept_at_strict < self.accept_at {
            return Some("accept_at_strict");
        }
        if self.drop_at >= 0 {
            return Some("drop_at");
        }
        if self.min_authors < 1 {
            return Some("min_authors");
        }
        if self.settle_window < 2 {
            return Some("settle_window");
        }
        if self.settle_delta < 0 {
            return Some("settle_delta");
        }
        if !(1..=Self::MAX_EVIDENCE_BASE).contains(&self.evidence_base) {
            return Some("evidence_base");
        }
        if self.mark_timeout_secs < 60 {
            return Some("mark_timeout_secs");
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Stakes {
    Low,
    Medium,
    High,
}

impl Stakes {
    pub fn factor(self) -> u64 {
        match self {
            Stakes::Low => 1,
            Stakes::Medium => 2,
            Stakes::High => 4,
        }
    }
}

/// Body of a `claim` message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimBody {
    pub text: String,
    pub domains: Vec<String>,
    #[rusty_serde(default)]
    pub deps: Vec<MsgId>,
    #[rusty_serde(default)]
    pub answers: Option<MsgId>,
    #[rusty_serde(default)]
    pub supersedes: Option<MsgId>,
    pub stakes: Stakes,
    pub reversible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Support,
    Counter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    Executed,
    Cited,
    Opinion,
}

/// Body of an `evidence` message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBody {
    pub claim: MsgId,
    pub direction: Direction,
    pub kind: EvidenceKind,
    pub strength: u8,
    #[rusty_serde(default)]
    pub note: String,
}

pub const MAX_DOMAINS: usize = 8;
/// The domain of a claim that changes protected tests (#597 G2). Its
/// candidate may touch protected paths, and it accepts only with support
/// from a principal who authored neither its candidate nor its diffs.
pub const TESTS_DOMAIN: &str = "tests";
pub const MAX_DEPS: usize = 16;
pub const DOMAIN_CAP: usize = 32;

/// `weight(kind, direction) × strength` with its sign (the update rule).
pub fn delta(kind: EvidenceKind, direction: Direction, strength: u8) -> i64 {
    let weight: i64 = match (kind, direction) {
        (EvidenceKind::Executed, Direction::Support) => 4,
        (EvidenceKind::Executed, Direction::Counter) => 8,
        (EvidenceKind::Cited, _) => 2,
        (EvidenceKind::Opinion, _) => 1,
    };
    let signed = match direction {
        Direction::Support => weight,
        Direction::Counter => -weight,
    };
    signed * i64::from(strength)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClaimStatus {
    Open,
    Accepted,
    Dropped,
    Void,
    Escalated,
    Superseded,
}

impl ClaimStatus {
    pub fn name(self) -> &'static str {
        match self {
            ClaimStatus::Open => "OPEN",
            ClaimStatus::Accepted => "ACCEPTED",
            ClaimStatus::Dropped => "DROPPED",
            ClaimStatus::Void => "VOID",
            ClaimStatus::Escalated => "ESCALATED",
            ClaimStatus::Superseded => "SUPERSEDED",
        }
    }
}

/// Why a claim waits for the human.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClaimEscalation {
    Irreversible,
    SupersedesActed,
    Budget,
    Stuck,
    DepCollapsed,
    ReopenedAfterAction,
    /// Carries the mark id.
    ActionUnconfirmed(u64),
}

/// What reopened a claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReopenCause {
    /// A human `decision reopen` (or `budget_extended` + reopen).
    Human,
    /// The evidence message that reopened it.
    Evidence(MsgId),
    /// The dep that left `accepted`.
    Dep(MsgId),
}

/// Human outcomes on one claim (C-R8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimOutcome {
    Accept,
    Drop,
    Reopen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkOutcome {
    Done,
    Aborted,
}

/// One evidence item as the fold keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub msg: MsgId,
    pub author: PrincipalId,
    pub kind: EvidenceKind,
    pub direction: Direction,
    /// The author's stated strength, as posted.
    pub strength: u8,
    /// The author's note, as posted (empty if none).
    pub note: String,
    pub delta: i64,
    pub epoch: u32,
    /// Admitted while the claim was `open` (or reopened by it).
    pub scoring: bool,
    /// Executed evidence against a settled outcome, recorded only because a
    /// dep was closed (owner, 2026-11-02: the claim stays dead). A human
    /// reopen, possible once the dep is live again, makes it count.
    pub dormant: bool,
}

/// An action mark (C-R9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub id: u64,
    pub epoch: u32,
    pub reference: String,
    pub at: Time,
    pub outcome: Option<MarkOutcome>,
    pub escalated: bool,
}

/// Everything the fold knows about one claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRecord {
    pub id: MsgId,
    pub author: PrincipalId,
    pub body: ClaimBody,
    pub status: ClaimStatus,
    pub epoch: u32,
    pub items: Vec<Item>,
    pub runs: u64,
    pub extensions: u64,
    pub marks: Vec<Mark>,
    /// The candidate stored last naming this claim.
    pub current: Option<ArtId>,
    /// Profiles failing on the current candidate (Failing runs block).
    pub failing: BTreeSet<String>,
    /// The report that put each `failing` profile there (for the card).
    pub failing_from: BTreeMap<String, ArtId>,
    /// `sole_candidate` of the last `claim_accepted` (C-R7).
    pub sole_candidate: bool,
    /// The rev of the last change to status, score or evidence.
    pub changed: Rev,
    /// Score at the last settlement, for the card.
    pub last_score: i64,
}

impl ClaimRecord {
    pub fn new(id: MsgId, author: PrincipalId, body: ClaimBody, rev: Rev) -> ClaimRecord {
        ClaimRecord {
            id,
            author,
            body,
            status: ClaimStatus::Open,
            epoch: 1,
            items: Vec::new(),
            runs: 0,
            extensions: 0,
            marks: Vec::new(),
            current: None,
            failing: BTreeSet::new(),
            failing_from: BTreeMap::new(),
            sole_candidate: false,
            changed: rev,
            last_score: 0,
        }
    }

    fn scoring(&self) -> impl Iterator<Item = &Item> {
        let epoch = self.epoch;
        self.items
            .iter()
            .filter(move |i| i.scoring && i.epoch == epoch)
    }

    pub fn score(&self) -> i64 {
        self.scoring().map(|i| i.delta).sum()
    }

    pub fn authors(&self, d: Direction) -> usize {
        self.scoring()
            .filter(|i| i.direction == d)
            .map(|i| &i.author)
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// At least one scoring item in direction `d` that is not an opinion.
    pub fn grounded(&self, d: Direction) -> bool {
        self.scoring()
            .any(|i| i.direction == d && i.kind != EvidenceKind::Opinion)
    }

    pub fn strict(&self) -> bool {
        self.body.stakes == Stakes::High || !self.body.reversible
    }

    pub fn accept_at(&self, c: &ClaimsConfig) -> i64 {
        if self.strict() {
            c.accept_at_strict
        } else {
            c.accept_at
        }
    }

    pub fn base(&self, c: &ClaimsConfig) -> u64 {
        let rev = if self.body.reversible { 1 } else { 2 };
        c.evidence_base * self.body.stakes.factor() * rev
    }

    pub fn budget(&self, c: &ClaimsConfig) -> u64 {
        self.base(c) * (1 + self.extensions)
    }

    pub fn spent(&self) -> u64 {
        self.items.len() as u64 + self.runs
    }

    pub fn exhausted(&self, c: &ClaimsConfig) -> bool {
        self.spent() >= self.budget(c)
    }

    /// Acted on: a mark with no `aborted` result, in any epoch.
    pub fn acted_on(&self) -> bool {
        self.marks
            .iter()
            .any(|m| m.outcome != Some(MarkOutcome::Aborted))
    }

    /// The settling rule: the last `window` scoring items moved the score by
    /// at most `delta`.
    pub fn settled_flat(&self, c: &ClaimsConfig) -> bool {
        let deltas: Vec<i64> = self.scoring().map(|i| i.delta).collect();
        let w = c.settle_window as usize;
        if deltas.len() < w {
            return false;
        }
        let moved: i64 = deltas[deltas.len() - w..].iter().sum();
        moved.abs() <= c.settle_delta
    }

    pub fn closed(&self) -> bool {
        matches!(
            self.status,
            ClaimStatus::Dropped | ClaimStatus::Void | ClaimStatus::Superseded
        )
    }
}

/// The claims of one task, keyed by claim message id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClaimsBook {
    pub config: ClaimsConfig,
    pub claims: BTreeMap<MsgId, ClaimRecord>,
    /// Candidate artifact → the claim it names.
    pub candidates: BTreeMap<ArtId, MsgId>,
    /// Live and past runs in claims mode: run → (candidate, revoked).
    pub runs: BTreeMap<RunId, ClaimRun>,
    /// Held human reruns: candidate → claim.
    pub held: BTreeMap<ArtId, MsgId>,
    pub next_mark: u64,
    /// Artifacts each principal has been charged a read for (R11 in claims
    /// mode: one charge per principal and artifact).
    pub read_arts: BTreeSet<(PrincipalId, ArtId)>,
    /// SHA-256 of the task's frozen holdout suite, if it has one (#597).
    pub holdout_digest: Option<Sha256>,
    /// Every holdout digest the task has left behind (#597). A retired suite
    /// is never adopted again: its old passes would count once more.
    pub holdout_retired: BTreeSet<Sha256>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRun {
    pub candidate: ArtId,
    pub revoked: bool,
    pub report: Option<ArtId>,
    pub log: Option<ArtId>,
}

impl ClaimsBook {
    pub fn get(&self, id: MsgId) -> Option<&ClaimRecord> {
        self.claims.get(&id)
    }

    /// Claims that list `id` in `deps`, in id order.
    pub fn dependents(&self, id: MsgId) -> Vec<MsgId> {
        self.claims
            .values()
            .filter(|c| c.body.deps.contains(&id))
            .map(|c| c.id)
            .collect()
    }

    /// The first dep of `c` that is dropped, void or superseded.
    pub fn closed_dep(&self, c: &ClaimRecord) -> Option<MsgId> {
        c.body
            .deps
            .iter()
            .copied()
            .find(|d| self.claims.get(d).is_some_and(|x| x.closed()))
    }

    /// Is `target` among `deps` or their deps, transitively?
    pub fn reaches(&self, deps: &[MsgId], target: MsgId) -> bool {
        let mut stack: Vec<MsgId> = deps.to_vec();
        let mut seen = BTreeSet::new();
        while let Some(d) = stack.pop() {
            if d == target {
                return true;
            }
            if !seen.insert(d) {
                continue;
            }
            if let Some(c) = self.claims.get(&d) {
                stack.extend(c.body.deps.iter().copied());
            }
        }
        false
    }

    /// Live candidates (not dropped, void or superseded) answering `question`.
    pub fn candidates_of(&self, question: MsgId) -> Vec<MsgId> {
        self.claims
            .values()
            .filter(|c| c.body.answers == Some(question) && !c.closed())
            .map(|c| c.id)
            .collect()
    }

    /// The live (not revoked) run of `candidate`, if any.
    pub fn live_run(&self, candidate: ArtId) -> Option<RunId> {
        self.runs
            .iter()
            .find(|(_, r)| r.candidate == candidate && !r.revoked)
            .map(|(id, _)| *id)
    }

    /// The latest final report of any run of `candidate`.
    pub fn latest_report(&self, candidate: ArtId) -> Option<ArtId> {
        self.runs
            .values()
            .filter(|r| r.candidate == candidate)
            .filter_map(|r| r.report)
            .max()
    }

    /// The latest final report of any run of a candidate naming `claim`.
    pub fn latest_claim_report(&self, claim: MsgId) -> Option<ArtId> {
        self.runs
            .values()
            .filter(|r| self.candidates.get(&r.candidate) == Some(&claim))
            .filter_map(|r| r.report)
            .max()
    }

    /// Marks with no result and no escalation yet, with their deadline.
    pub fn pending_marks(&self) -> impl Iterator<Item = (MsgId, u64, Time)> + '_ {
        let ms = self.config.mark_timeout_secs.saturating_mul(1_000);
        self.claims.values().flat_map(move |c| {
            c.marks
                .iter()
                .filter(|m| m.outcome.is_none() && !m.escalated)
                .map(move |m| (c.id, m.id, m.at.plus(ms)))
        })
    }

    /// The earliest time a pending mark becomes overdue.
    pub fn next_mark_deadline(&self) -> Option<Time> {
        self.pending_marks().map(|(_, _, t)| t).min()
    }
}

// ---------------------------------------------------------------- fold

/// Fold a claims-mode message: a claim opens a record, evidence joins one.
pub(crate) fn on_message(st: &mut crate::state::TaskState, m: &crate::record::Message) {
    let rev = st.rev;
    match &m.draft.body {
        crate::record::Body::Claim(b) => {
            st.claims
                .claims
                .insert(m.id, ClaimRecord::new(m.id, m.from.clone(), b.clone(), rev));
        }
        crate::record::Body::Evidence(e) => {
            let dormant = st.claims.get(e.claim).is_some_and(|c| {
                let against = matches!(
                    (c.status, e.kind, e.direction),
                    (
                        ClaimStatus::Accepted,
                        EvidenceKind::Executed,
                        Direction::Counter
                    ) | (
                        ClaimStatus::Dropped,
                        EvidenceKind::Executed,
                        Direction::Support
                    )
                );
                against && st.claims.closed_dep(c).is_some()
            });
            if let Some(c) = st.claims.claims.get_mut(&e.claim) {
                c.items.push(Item {
                    msg: m.id,
                    author: m.from.clone(),
                    kind: e.kind,
                    direction: e.direction,
                    strength: e.strength,
                    note: e.note.clone(),
                    delta: delta(e.kind, e.direction, e.strength),
                    epoch: c.epoch,
                    scoring: c.status == ClaimStatus::Open,
                    dormant,
                });
                c.changed = rev;
            }
        }
        _ => {}
    }
}

/// Fold a claims-mode artifact: a candidate naming a claim becomes current.
pub(crate) fn on_artifact(st: &mut crate::state::TaskState, a: &crate::record::ArtifactRecord) {
    if let (crate::record::ArtifactKind::Log, Some(run)) = (a.kind, a.run) {
        if let Some(r) = st.claims.runs.get_mut(&run) {
            r.log = Some(a.id);
        }
        return;
    }
    let crate::record::ArtifactPayload::Candidate(k) = &a.payload else {
        return;
    };
    let Some(claim) = k.claim else {
        return;
    };
    st.claims.candidates.insert(a.id, claim);
    if !k.suite.is_build() {
        return; // a holdout candidate never becomes current
    }
    if let Some(c) = st.claims.claims.get_mut(&claim) {
        c.current = Some(a.id);
    }
}

/// Fold the claims-mode events.
pub(crate) fn apply(st: &mut crate::state::TaskState, e: &crate::event::Event) {
    use crate::event::Event as E;
    let rev = st.rev;
    let book = &mut st.claims;
    // The report a newly failing profile comes from: the claim's latest,
    // which the runner has just stored (on the current candidate, or on a
    // holdout candidate of the same code).
    let failing_report = match e {
        E::ClaimFailing { claim, .. } => book.latest_claim_report(*claim),
        _ => None,
    };
    let mut touch = |id: &MsgId, f: &mut dyn FnMut(&mut ClaimRecord)| {
        if let Some(c) = book.claims.get_mut(id) {
            f(c);
            c.changed = rev;
        }
    };
    match e {
        E::ClaimAccepted {
            claim,
            score,
            sole_candidate,
            ..
        } => touch(claim, &mut |c| {
            c.status = ClaimStatus::Accepted;
            c.last_score = *score;
            c.sole_candidate = *sole_candidate;
        }),
        E::ClaimDropped { claim, score, .. } => touch(claim, &mut |c| {
            c.status = ClaimStatus::Dropped;
            c.last_score = *score;
        }),
        E::ClaimVoided { claim, .. } => touch(claim, &mut |c| c.status = ClaimStatus::Void),
        E::ClaimEscalated { claim, reason } => match reason {
            ClaimEscalation::ActionUnconfirmed(mark) => touch(claim, &mut |c| {
                if let Some(m) = c.marks.iter_mut().find(|m| m.id == *mark) {
                    m.escalated = true;
                }
            }),
            _ => touch(claim, &mut |c| c.status = ClaimStatus::Escalated),
        },
        E::ClaimReopened { claim, epoch, by } => touch(claim, &mut |c| {
            c.status = ClaimStatus::Open;
            c.epoch = *epoch;
            c.sole_candidate = false;
            // Reopened by evidence: that item is the new epoch's first scoring
            // one. Reopened by the human: dormant items count in the new epoch.
            for i in c.items.iter_mut() {
                let wakes = match by {
                    ReopenCause::Evidence(m) => i.msg == *m,
                    ReopenCause::Human => i.dormant,
                    ReopenCause::Dep(_) => false,
                };
                if wakes {
                    i.epoch = *epoch;
                    i.scoring = true;
                    i.dormant = false;
                }
            }
        }),
        E::ClaimSuperseded { claim, .. } => {
            touch(claim, &mut |c| c.status = ClaimStatus::Superseded);
            book.held.retain(|_, c| c != claim);
        }
        E::ClaimBudgetExtended { claim } => touch(claim, &mut |c| c.extensions += 1),
        E::ClaimRunDebited { claim, .. } => touch(claim, &mut |c| c.runs += 1),
        E::RerunHeld { claim, candidate } => {
            book.held.insert(*candidate, *claim);
        }
        E::RerunWithdrawn { candidate } => {
            book.held.remove(candidate);
        }
        E::ClaimFailing { claim, failing } => touch(claim, &mut |c| {
            c.failing = failing.iter().cloned().collect();
            let mut from = BTreeMap::new();
            for name in &c.failing {
                let set_by = c.failing_from.get(name).copied().or(failing_report);
                if let Some(r) = set_by {
                    from.insert(name.clone(), r);
                }
            }
            c.failing_from = from;
        }),
        E::ClaimActing {
            claim,
            epoch,
            mark,
            reference,
            at,
        } => {
            book.next_mark = mark + 1;
            touch(claim, &mut |c| {
                c.marks.push(Mark {
                    id: *mark,
                    epoch: *epoch,
                    reference: reference.clone(),
                    at: *at,
                    outcome: None,
                    escalated: false,
                })
            });
        }
        E::HoldoutExtended { digest } | E::HoldoutUpdated { digest } => {
            // Open claims now need a fresh holdout pass under `digest`;
            // accepted ones stay accepted (holdout_passed reads the digest).
            // Every claim with a candidate is due again, so it changed.
            if let Some(old) = book.holdout_digest.replace(*digest) {
                book.holdout_retired.insert(old);
            }
            for c in book.claims.values_mut().filter(|c| c.current.is_some()) {
                c.changed = rev;
            }
        }
        E::ClaimActionResult { mark, outcome } => {
            for c in book.claims.values_mut() {
                if let Some(m) = c.marks.iter_mut().find(|m| m.id == *mark) {
                    m.outcome = Some(*outcome);
                    c.changed = rev;
                }
            }
        }
        _ => {}
    }
}

/// The claim that owns mark `id`.
pub fn mark_owner(book: &ClaimsBook, id: u64) -> Option<(MsgId, &Mark)> {
    book.claims
        .values()
        .find_map(|c| c.marks.iter().find(|m| m.id == id).map(|m| (c.id, m)))
}

// ---------------------------------------------------------------- views

/// One evidence item on a card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceView {
    pub msg: MsgId,
    pub author: PrincipalId,
    pub direction: Direction,
    pub kind: EvidenceKind,
    /// The author's stated strength, as posted.
    pub strength: u8,
    /// The author's note, as posted (empty if none).
    pub note: String,
    pub delta: i64,
    pub epoch: u32,
    pub scoring: bool,
}

/// A claim card: the human-readable view (and the delta entry).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimView {
    pub id: MsgId,
    pub text: String,
    pub domains: Vec<String>,
    pub status: ClaimStatus,
    pub score: i64,
    /// The score this claim accepts at: the task's frozen line, strict for
    /// high stakes or an irreversible claim (#594, D56).
    pub accept_at: i64,
    pub confidence: u32,
    pub epoch: u32,
    pub spent: u64,
    pub budget: u64,
    pub evidence: Vec<EvidenceView>,
    pub deps: Vec<MsgId>,
    pub candidates: Vec<MsgId>,
    /// C-R7: accepted while its question had fewer than two candidates.
    pub sole_candidate: bool,
    /// The candidate stored last naming this claim, if the viewer may read it.
    pub current: Option<ArtId>,
    /// `failing(c)`, each with the report that set it (Failing runs block).
    pub failing: Vec<FailingView>,
    /// The test-path changes of the current candidate's latest report.
    pub tests_changed: Vec<crate::record::TestChange>,
    /// The cited support that endorses those changes, if any.
    pub endorsement: Option<MsgId>,
    /// The task has a holdout suite and the current candidate has no passed
    /// holdout run on it yet: the accept line waits for one (#597).
    pub holdout_due: bool,
}

/// A failing profile and the report that set it (hidden if unreadable).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailingView {
    pub profile: String,
    pub report: Option<ArtId>,
}

/// What a viewer may see. The human channel and the host pass `all`.
pub struct Sight<'a> {
    pub message: Box<dyn Fn(MsgId) -> bool + 'a>,
    pub artifact: Box<dyn Fn(ArtId) -> bool + 'a>,
}

impl Sight<'_> {
    /// The unfiltered view: the human channel and the host.
    pub fn all() -> Sight<'static> {
        Sight {
            message: Box::new(|_| true),
            artifact: Box::new(|_| true),
        }
    }
}

/// The card of `c`, with nested content filtered by `sight`.
pub fn card(st: &crate::state::TaskState, c: &ClaimRecord, sight: &Sight) -> ClaimView {
    let book = &st.claims;
    let score = shown_score(c);
    let current = c.current.filter(|k| (sight.artifact)(*k));
    let report = current
        .and_then(|k| book.latest_report(k))
        .filter(|r| (sight.artifact)(*r));
    let tests_changed = match report
        .and_then(|r| st.artifacts.get(&r))
        .map(|a| &a.payload)
    {
        Some(crate::record::ArtifactPayload::TestReport(r)) => r.tests_changed.clone(),
        _ => Vec::new(),
    };
    let endorsement = current
        .filter(|_| !tests_changed.is_empty())
        .and_then(|k| crate::claims_engine::endorsement(st, c.id, k))
        .filter(|m| (sight.message)(*m));
    ClaimView {
        id: c.id,
        text: c.body.text.clone(),
        domains: c.body.domains.clone(),
        status: c.status,
        score,
        accept_at: c.accept_at(&book.config),
        confidence: confidence_pct(score),
        epoch: c.epoch,
        spent: c.spent(),
        budget: c.budget(&book.config),
        evidence: c
            .items
            .iter()
            .filter(|i| (sight.message)(i.msg))
            .map(|i| EvidenceView {
                msg: i.msg,
                author: i.author.clone(),
                direction: i.direction,
                kind: i.kind,
                strength: i.strength,
                note: i.note.clone(),
                delta: i.delta,
                epoch: i.epoch,
                scoring: i.scoring,
            })
            .collect(),
        deps: c
            .body
            .deps
            .iter()
            .copied()
            .filter(|d| (sight.message)(*d))
            .collect(),
        candidates: book
            .claims
            .values()
            .filter(|x| x.body.answers == Some(c.id) && (sight.message)(x.id))
            .map(|x| x.id)
            .collect(),
        sole_candidate: c.sole_candidate,
        current,
        failing: c
            .failing
            .iter()
            .map(|p| FailingView {
                profile: p.clone(),
                report: c
                    .failing_from
                    .get(p)
                    .copied()
                    .filter(|r| (sight.artifact)(*r)),
            })
            .collect(),
        tests_changed,
        endorsement,
        holdout_due: !crate::claims_engine::holdout_passed(st, c),
    }
}

/// The score a card shows: the live epoch's score while open, the score at
/// settlement afterwards.
pub fn shown_score(c: &ClaimRecord) -> i64 {
    match c.status {
        ClaimStatus::Open | ClaimStatus::Escalated => c.score(),
        _ => c.last_score,
    }
}

/// The one-line compaction: `msg:17 ACCEPTED +12 (96%) 5 evd · text…`.
pub fn line(c: &ClaimRecord) -> String {
    // The text part is at most 60 characters, ellipsis included.
    const TEXT: usize = 60;
    let score = shown_score(c);
    let text: String = if c.body.text.chars().count() > TEXT {
        let mut t: String = c.body.text.chars().take(TEXT - 1).collect();
        t.push('…');
        t
    } else {
        c.body.text.clone()
    };
    format!(
        "{} {} {:+} ({}%) {} evd · {}",
        c.id,
        c.status.name(),
        score,
        confidence_pct(score),
        c.items.len(),
        text
    )
}

/// Claims changed after `after` with a domain in `domains`, plus their
/// dependents (transitively), in id order. Only claims `sight` allows are
/// returned or followed, so a hidden claim is neither shown nor hinted at.
pub fn delta_ids(book: &ClaimsBook, after: Rev, domains: &[String], sight: &Sight) -> Vec<MsgId> {
    let mut out: BTreeSet<MsgId> = book
        .claims
        .values()
        .filter(|c| c.changed > after && c.body.domains.iter().any(|d| domains.contains(d)))
        .filter(|c| (sight.message)(c.id))
        .map(|c| c.id)
        .collect();
    let mut stack: Vec<MsgId> = out.iter().copied().collect();
    while let Some(id) = stack.pop() {
        for d in book.dependents(id) {
            if (sight.message)(d) && out.insert(d) {
                stack.push(d);
            }
        }
    }
    out.into_iter().collect()
}

/// Shown confidence, whole percent: `100 / (1 + 3^(−score/4))`, in integers.
/// For people only; no rule reads it.
pub fn confidence_pct(score: i64) -> u32 {
    // p = 3^(s/4) / (3^(s/4) + 1). Work in quarter-powers with a fixed table so
    // the result is exact for multiples of 4 and monotone in between.
    // 3^(k/4) × 1e6 for k = 0..3.
    const QUARTER: [u128; 4] = [1_000_000, 1_316_074, 1_732_051, 2_279_507];
    let s = score.clamp(-160, 160);
    let k = s.unsigned_abs();
    let whole = (k / 4) as u32;
    let frac = (k % 4) as usize;
    let num = 3u128.pow(whole) * QUARTER[frac];
    let den = 1_000_000u128;
    // For a non-negative score p = num / (num + den); for a negative one,
    // p = den / (num + den).
    let top = if s >= 0 { num } else { den };
    let pct = (top * 200 + (num + den)) / (2 * (num + den));
    pct as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_matches_the_defaults() {
        assert_eq!(confidence_pct(0), 50);
        assert_eq!(confidence_pct(8), 90);
        assert_eq!(confidence_pct(12), 96);
        assert_eq!(confidence_pct(-8), 10);
        assert!(confidence_pct(4) > 50 && confidence_pct(4) < 90);
    }

    #[test]
    fn deltas_follow_the_weights() {
        use Direction::*;
        use EvidenceKind::*;
        assert_eq!(delta(Executed, Support, 1), 4);
        assert_eq!(delta(Executed, Counter, 1), -8);
        assert_eq!(delta(Cited, Support, 3), 6);
        assert_eq!(delta(Opinion, Counter, 2), -2);
    }

    #[test]
    fn config_rules_name_the_field() {
        let ok = ClaimsConfig::default();
        assert_eq!(ok.invalid_field(), None);
        let bad = ClaimsConfig { accept_at: 0, ..ok };
        assert_eq!(bad.invalid_field(), Some("accept_at"));
        let bad = ClaimsConfig {
            mark_timeout_secs: 59,
            ..ok
        };
        assert_eq!(bad.invalid_field(), Some("mark_timeout_secs"));
    }
}
