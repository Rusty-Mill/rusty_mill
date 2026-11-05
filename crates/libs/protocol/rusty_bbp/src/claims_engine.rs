//! Claims mode: the processing rules of `bbp-claims-addendum.md`. Admission
//! (C-R1..C-R9), the update rule, dependencies, budgets, runs, failing-run
//! blocks, supersession and action marks. Pure, like the rest of the engine:
//! every effect is an event emitted through `Work`, so the fold is the truth.

use crate::claims::*;
use crate::command::*;
use crate::engine::{
    body_len, check_common, check_rev, get_artifact_record, rej, resolve_kind, role_data,
    transition, visible, Work, MAX_ARTIFACT_BYTES, R, RUNNER,
};
use crate::event::Event;
use crate::ids::*;
use crate::record::*;
use crate::role::{Cap, Role};
use crate::state::{State, TaskState};

// ---------------------------------------------------------------- admission

/// Operations that only exist in claims mode. A stage task refuses them as
/// `wrong_mode` before any turn check.
pub(crate) fn claims_only(a: &AgentAction) -> bool {
    match a {
        AgentAction::Post(d) => matches!(d.kind, MessageKind::Claim | MessageKind::Evidence),
        AgentAction::PutArtifact { payload, .. } => match payload {
            ArtifactPayload::Source(_) => true,
            ArtifactPayload::Candidate(c) => c.claim.is_some(),
            _ => false,
        },
        AgentAction::ClaimsDelta { .. }
        | AgentAction::ClaimCard { .. }
        | AgentAction::ClaimLine { .. } => true,
        AgentAction::Read { .. } | AgentAction::GetArtifact { .. } | AgentAction::TaskCard => false,
    }
}

/// The open-time rules of claims mode (`bad_claims_config`).
pub(crate) fn check_open(o: &OpenTask) -> R<()> {
    let cfg = o.claims.unwrap_or_default();
    if let Some(field) = cfg.invalid_field() {
        return Err(rej(Code::BadClaimsConfig, field));
    }
    if o.max_live_turns != 1 {
        return Err(rej(
            Code::BadClaimsConfig,
            "max_live_turns: claims mode grants no agent turn",
        ));
    }
    if o.policy != crate::policy::SchedulingPolicy::TurnBased {
        return Err(rej(
            Code::BadClaimsConfig,
            "policy: claims mode has no scheduling policy",
        ));
    }
    Ok(())
}

/// C-R1: the session credential of the principal's current assignment.
pub(crate) fn check_credential(st: &TaskState, p: &PrincipalId, token: Option<Token>) -> R<()> {
    if st.state.terminal() {
        return Err(rej(Code::BadCredential, "the task has ended"));
    }
    match (token, st.credential_for(p)) {
        (Some(t), Some(expected)) if t == expected => Ok(()),
        (None, _) => Err(rej(Code::BadCredential, "no session credential")),
        _ => Err(rej(
            Code::BadCredential,
            "credential is not this principal's current one",
        )),
    }
}

/// C-R2: agent writes need the task `open`.
fn require_task_open(st: &TaskState) -> R<()> {
    if st.state != State::Open {
        return Err(rej(
            Code::WrongState,
            format!("task is {:?}, not open", st.state),
        ));
    }
    Ok(())
}

fn claim_of(st: &TaskState, id: MsgId) -> R<&ClaimRecord> {
    if st.message(id).is_none() {
        return Err(rej(Code::RefUnresolved, format!("{id}")));
    }
    st.claims
        .get(id)
        .ok_or_else(|| rej(Code::WrongKind, format!("{id} is not a claim")))
}

fn status(st: &TaskState, id: MsgId) -> Option<ClaimStatus> {
    st.claims.get(id).map(|c| c.status)
}

// ---------------------------------------------------------------- agent

pub(crate) fn agent(w: &mut Work, p: &PrincipalId, role: &RoleId, a: &AgentAction) -> R<Response> {
    match a {
        AgentAction::TaskCard => Ok(Response::Card(w.st.card())),
        AgentAction::Read { after, limit } => read(w, p, role, *after, *limit),
        AgentAction::GetArtifact { id } => get_artifact(w, p, role, *id),
        AgentAction::PutArtifact { blob, payload } => put_artifact(w, p, role, *blob, payload),
        AgentAction::Post(d) => {
            let r = role_data(&w.st, role)?.clone();
            post(w, p, Some(&r), d)
        }
        AgentAction::ClaimsDelta { after, domains } => {
            let reader = role_data(&w.st, role)?;
            let sight = sight_of(&w.st, reader);
            let ids = delta_ids(&w.st.claims, *after, domains, &sight);
            Ok(Response::Claims(cards(&w.st, &ids, &sight)))
        }
        AgentAction::ClaimCard { claim } => {
            let reader = role_data(&w.st, role)?;
            let sight = sight_of(&w.st, reader);
            Ok(Response::Claims(cards(&w.st, &[*claim], &sight)))
        }
        AgentAction::ClaimLine { claim } => {
            let reader = role_data(&w.st, role)?;
            let sight = sight_of(&w.st, reader);
            let shown = (sight.message)(*claim);
            Ok(Response::Line(
                w.st.claims.get(*claim).filter(|_| shown).map(line),
            ))
        }
    }
}

/// What `role` may read: the same rule as `read` (`read_messages`).
fn sight_of<'a>(st: &'a TaskState, role: &'a Role) -> Sight<'a> {
    Sight {
        message: Box::new(move |id: MsgId| st.message(id).is_some_and(|m| visible(st, role, m))),
        artifact: Box::new(move |id: ArtId| {
            st.artifacts
                .get(&id)
                .is_some_and(|a| role.holds(Cap::Read(a.kind)))
        }),
    }
}

fn cards(st: &TaskState, ids: &[MsgId], sight: &Sight) -> Vec<ClaimView> {
    ids.iter()
        .filter(|id| (sight.message)(**id))
        .filter_map(|id| st.claims.get(*id))
        .map(|c| card(st, c, sight))
        .collect()
}

/// R11 in claims mode: every `read` call is charged; no turn holds a quota.
fn read(
    w: &mut Work,
    p: &PrincipalId,
    role: &RoleId,
    after: Option<MsgId>,
    limit: usize,
) -> R<Response> {
    let reader = role_data(&w.st, role)?.clone();
    w.emit(Event::Charged {
        principal: p.clone(),
        turn: None,
        messages: 0,
        bytes: 0,
        reads: 1,
    });
    let limit = limit.clamp(1, READ_PAGE);
    let start = after.map(|a| a.0 as usize).unwrap_or(0);
    let visible_msgs =
        w.st.messages
            .iter()
            .skip(start)
            .filter(|m| visible(&w.st, &reader, m));
    let mut out: Vec<Message> = Vec::new();
    let mut more = false;
    for m in visible_msgs {
        if out.len() == limit {
            more = true;
            break;
        }
        out.push(m.clone());
    }
    let next = out.last().map(|m| m.id).or(after);
    Ok(Response::Read {
        messages: out,
        next,
        more,
    })
}

/// R11 in claims mode: one read charge per (principal, artifact).
fn get_artifact(w: &mut Work, p: &PrincipalId, role: &RoleId, id: ArtId) -> R<Response> {
    let rec = get_artifact_record(&w.st, role, id)?;
    if !w.st.claims.read_arts.contains(&(p.clone(), id)) {
        w.emit(Event::Charged {
            principal: p.clone(),
            turn: None,
            messages: 0,
            bytes: 0,
            reads: 1,
        });
    }
    Ok(Response::Artifact(rec))
}

fn put_artifact(
    w: &mut Work,
    p: &PrincipalId,
    role: &RoleId,
    blob: BlobRef,
    payload: &ArtifactPayload,
) -> R<Response> {
    let kind = payload.kind();
    match kind {
        ArtifactKind::Diff | ArtifactKind::Candidate | ArtifactKind::Source => {}
        ArtifactKind::Spec => {
            return Err(rej(Code::WrongMode, "claims mode has no spec"));
        }
        ArtifactKind::Brief | ArtifactKind::TestReport | ArtifactKind::Log => {
            return Err(rej(Code::KindForbidden, "agents cannot write this kind"));
        }
    }
    let r = role_data(&w.st, role)?;
    if !r.holds(Cap::Write(kind)) {
        return Err(rej(Code::KindForbidden, r.lacks(Cap::Write(kind))));
    }
    require_task_open(&w.st)?;
    if blob.len > MAX_ARTIFACT_BYTES {
        return Err(rej(Code::TooLarge, "artifact exceeds max_artifact_bytes"));
    }
    let claim = match payload {
        ArtifactPayload::Candidate(c) => Some(check_candidate(&w.st, r, c)?),
        ArtifactPayload::Source(s) => {
            check_source(s)?;
            None
        }
        _ => None,
    };
    let id = w.st.next_art_id();
    w.emit(Event::ArtifactStored(ArtifactRecord {
        id,
        author: p.clone(),
        kind,
        blob,
        payload: payload.clone(),
        spec: None,
        candidate: None,
        run: None,
    }));
    w.emit(Event::Charged {
        principal: p.clone(),
        turn: None,
        messages: 0,
        bytes: blob.len,
        reads: 0,
    });
    if let Some(c) = claim {
        start_run(w, c, id);
        evaluate(w, c);
    }
    Ok(Response::Stored(id))
}

/// A claims-mode candidate names a live claim with budget left (Runs and
/// candidates). Returns that claim.
fn check_candidate(st: &TaskState, role: &Role, k: &Candidate) -> R<MsgId> {
    let c = k
        .claim
        .ok_or_else(|| rej(Code::BadClaim, "a claims-mode candidate names its claim"))?;
    let rec = claim_of(st, c)?;
    if !runnable(rec.status) {
        return Err(rej(
            Code::ClaimNotOpen,
            format!("{c} is {}", rec.status.name()),
        ));
    }
    if rec.exhausted(&st.claims.config) {
        return Err(rej(Code::ClaimBudget, format!("{c} has no budget left")));
    }
    for d in &k.diffs {
        resolve_kind(st, *d, ArtifactKind::Diff)?;
    }
    if k.suite == Suite::Holdout {
        check_holdout(st, role, rec, k)?;
    }
    Ok(c)
}

/// A holdout candidate (#597): stored only by a holdout role, on a task with
/// a holdout suite, building exactly the claim's current code.
fn check_holdout(st: &TaskState, role: &Role, rec: &ClaimRecord, k: &Candidate) -> R<()> {
    if !role.holds(Cap::Holdout) {
        return Err(rej(Code::KindForbidden, role.lacks(Cap::Holdout)));
    }
    if st.claims.holdout_digest.is_none() {
        return Err(rej(Code::HoldoutMismatch, "the task has no holdout suite"));
    }
    let current = rec.current.and_then(|id| manifest(st, id)).ok_or_else(|| {
        rej(
            Code::HoldoutMismatch,
            format!("{} has no candidate", rec.id),
        )
    })?;
    if !k.same_code(current) {
        return Err(rej(
            Code::HoldoutMismatch,
            format!("base and diffs must equal those of {}", rec.id),
        ));
    }
    Ok(())
}

fn manifest(st: &TaskState, k: ArtId) -> Option<&Candidate> {
    match &st.artifacts.get(&k)?.payload {
        ArtifactPayload::Candidate(c) => Some(c),
        _ => None,
    }
}

/// Does `k` build the code of `c`'s current candidate? True for the current
/// candidate itself and for a holdout candidate of the same manifest naming
/// `c`: another claim's candidate of equal code never speaks for `c`.
fn builds_current(st: &TaskState, c: &ClaimRecord, k: ArtId) -> bool {
    let Some(cur) = c.current else {
        return false;
    };
    if st.claims.candidates.get(&k) != Some(&c.id) {
        return false;
    }
    if cur == k {
        return true;
    }
    match (manifest(st, k), manifest(st, cur)) {
        (Some(a), Some(b)) => a.suite == Suite::Holdout && a.same_code(b),
        _ => false,
    }
}

/// The holdout condition of the accept line (#597): with a holdout suite and
/// a current candidate, the latest holdout report on the current code under
/// the task's holdout digest, stored after that candidate became current,
/// `passed` on the tree the current candidate's latest report tested. A
/// manifest names its base by any ref, so equal manifests alone do not prove
/// that two checkouts built the same tree, and a report without a tree proves
/// nothing. A later holdout failure supersedes an earlier pass. True when the
/// task has no holdout suite or the claim no candidate.
pub(crate) fn holdout_passed(st: &TaskState, c: &ClaimRecord) -> bool {
    let (Some(digest), Some(cur)) = (st.claims.holdout_digest, c.current) else {
        return true;
    };
    let Some(tested) = latest_report(st, cur).and_then(|r| r.tree) else {
        return false;
    };
    let latest = st
        .claims
        .runs
        .values()
        .filter(|r| r.candidate != cur && builds_current(st, c, r.candidate))
        .filter_map(|r| r.report)
        .filter(|id| *id > cur)
        .filter_map(|id| Some((id, report_of(st, id)?)))
        .filter(|(_, rep)| rep.profile_digest == digest)
        .max_by_key(|(id, _)| *id);
    latest.is_some_and(|(_, rep)| rep.status == RunStatus::Passed && rep.tree == Some(tested))
}

/// A run may start on an open claim or re-check a settled one.
fn runnable(s: ClaimStatus) -> bool {
    matches!(
        s,
        ClaimStatus::Open | ClaimStatus::Accepted | ClaimStatus::Dropped
    )
}

/// A human rerun may also target an escalated claim, typically one waiting
/// on budget: the rerun is then held for the extension.
fn rerun_admits(s: ClaimStatus) -> bool {
    runnable(s) || s == ClaimStatus::Escalated
}

fn check_source(s: &Source) -> R<()> {
    if s.excerpt.chars().count() > SOURCE_EXCERPT_CAP {
        return Err(rej(Code::BadClaim, "source excerpt over 4,000 characters"));
    }
    if Sha256::of(s.excerpt.as_bytes()) != s.digest {
        return Err(rej(
            Code::BadClaim,
            "source digest does not match its excerpt",
        ));
    }
    Ok(())
}

/// Debit one unit from `c` and start a run of `k`, revoking its live run.
fn start_run(w: &mut Work, c: MsgId, k: ArtId) {
    if let Some(old) = w.st.claims.live_run(k) {
        w.emit(Event::RunRevoked { run: old });
    }
    w.emit(Event::ClaimRunDebited {
        claim: c,
        candidate: k,
    });
    let run = RunId(w.st.next_run);
    w.emit(Event::RunStarted { candidate: k, run });
}

/// `post` of a `claim` or `evidence`, by an agent (`role` set) or the human.
fn post(w: &mut Work, p: &PrincipalId, role: Option<&Role>, d: &Draft) -> R<Response> {
    if role.is_none() && d.kind == MessageKind::Answer {
        return human_answer(w, p, d);
    }
    if !matches!(d.kind, MessageKind::Claim | MessageKind::Evidence) {
        return Err(rej(
            Code::WrongMode,
            format!("{} is not posted in claims mode", d.kind.name()),
        ));
    }
    if let Some(r) = role {
        if !r.holds(Cap::Post(d.kind)) {
            return Err(rej(Code::RoleForbidden, r.lacks(Cap::Post(d.kind))));
        }
    }
    require_task_open(&w.st)?;
    check_common(&w.st, d)?;
    let target = match &d.body {
        Body::Claim(b) => {
            check_claim(&w.st, b)?;
            None
        }
        Body::Evidence(e) => Some(check_evidence(&w.st, d, e)?),
        _ => return Err(rej(Code::UnknownKind, "body shape does not match kind")),
    };
    let bytes = body_len(&d.body).unwrap_or(0) as u64;
    let id = w.st.next_msg_id();
    w.emit(Event::MessageAppended(Message {
        id,
        from: p.clone(),
        from_role: role.map(|r| r.id.clone()),
        draft: d.clone(),
    }));
    // R11: human posts are not charged to the agents' task budget.
    if role.is_some() {
        w.emit(Event::Charged {
            principal: p.clone(),
            turn: None,
            messages: 1,
            bytes,
            reads: 0,
        });
    }
    if let (Some(c), Body::Evidence(e)) = (target, &d.body) {
        after_evidence(w, id, c, e, d);
    }
    Ok(Response::Posted(id))
}

/// C-R10: the human's `answer` clarifies the intent (Refine, #595). It refs
/// the brief, so #598 can turn it into an intent version, and replies to a
/// claim if it is about one. It needs no turn, is not charged, changes no
/// claim, and is taken while the task is open or escalated.
fn human_answer(w: &mut Work, p: &PrincipalId, d: &Draft) -> R<Response> {
    if w.st.state.terminal() {
        return Err(rej(Code::WrongState, format!("task is {:?}", w.st.state)));
    }
    check_common(&w.st, d)?;
    let brief = w.st.brief.map(Ref::art);
    if brief.as_ref().is_none_or(|b| !d.refs.contains(b)) {
        return Err(rej(Code::RefsRequired, "an answer refs the task's brief"));
    }
    if let Some(c) = d.reply_to {
        claim_of(&w.st, c)?;
    }
    let id = w.st.next_msg_id();
    w.emit(Event::MessageAppended(Message {
        id,
        from: p.clone(),
        from_role: None,
        draft: d.clone(),
    }));
    Ok(Response::Posted(id))
}

/// C-R3: a claim's fields.
fn check_claim(st: &TaskState, b: &ClaimBody) -> R<()> {
    if b.text.is_empty() {
        return Err(rej(Code::BadClaim, "text is empty"));
    }
    if b.domains.is_empty() || b.domains.len() > MAX_DOMAINS {
        return Err(rej(Code::BadClaim, "domains: 1 to 8"));
    }
    if let Some(bad) = b.domains.iter().find(|d| !domain_ok(d)) {
        return Err(rej(
            Code::BadClaim,
            format!("domain {bad:?} is not [a-z0-9_-]{{1,32}}"),
        ));
    }
    if b.deps.len() > MAX_DEPS {
        return Err(rej(Code::BadClaim, "deps: at most 16"));
    }
    for dep in &b.deps {
        let rec = claim_of(st, *dep)?;
        if rec.closed() {
            return Err(rej(
                Code::DepClosed,
                format!("{dep} is {}", rec.status.name()),
            ));
        }
    }
    if let Some(q) = b.answers {
        claim_of(st, q)?;
    }
    if let Some(x) = b.supersedes {
        let rec = claim_of(st, x)?;
        if rec.status == ClaimStatus::Superseded {
            return Err(rej(Code::BadClaim, format!("{x} is already superseded")));
        }
        if st.claims.reaches(&b.deps, x) {
            return Err(rej(
                Code::BadClaim,
                format!("a replacement cannot rest on {x}, which it retires"),
            ));
        }
    }
    Ok(())
}

fn domain_ok(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= DOMAIN_CAP
        && d.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// C-R4: evidence targets a claim with budget left and is bound to what it
/// cites. Returns the claim.
fn check_evidence(st: &TaskState, d: &Draft, e: &EvidenceBody) -> R<MsgId> {
    let rec = claim_of(st, e.claim)?;
    if !d.refs.contains(&Ref::Msg(e.claim)) {
        return Err(rej(Code::BadClaim, "refs must include the claim"));
    }
    if !(1..=3).contains(&e.strength) {
        return Err(rej(Code::BadClaim, "strength is 1 to 3"));
    }
    if rec.exhausted(&st.claims.config) {
        return Err(rej(
            Code::ClaimBudget,
            format!("{} has no budget left", e.claim),
        ));
    }
    match e.kind {
        EvidenceKind::Opinion => Ok(e.claim),
        EvidenceKind::Cited if d.evidence.is_empty() => Err(rej(
            Code::EvidenceRequired,
            "cited evidence cites something",
        )),
        EvidenceKind::Cited => Ok(e.claim),
        EvidenceKind::Executed => {
            if e.strength != 1 {
                return Err(rej(Code::BadClaim, "executed evidence has strength 1"));
            }
            check_executed(st, d, e).map(|_| e.claim)
        }
    }
}

/// Executed evidence cites a final runner report of a run whose candidate
/// names the same claim, in the right direction.
fn check_executed(st: &TaskState, d: &Draft, e: &EvidenceBody) -> R<()> {
    let id = d
        .evidence
        .iter()
        .find_map(|r| match r {
            Ref::Art { id, .. } => Some(*id),
            Ref::Msg(_) => None,
        })
        .ok_or_else(|| rej(Code::EvidenceRequired, "executed evidence cites a report"))?;
    let rec = resolve_kind(st, id, ArtifactKind::TestReport)?;
    let ArtifactPayload::TestReport(r) = &rec.payload else {
        return Err(rej(Code::EvidenceUnbound, "not a test report"));
    };
    if rec.author.0 != RUNNER {
        return Err(rej(Code::EvidenceUnbound, "the runner did not write it"));
    }
    if st.claims.candidates.get(&r.candidate) != Some(&e.claim) {
        return Err(rej(
            Code::EvidenceUnbound,
            "the report's candidate names another claim",
        ));
    }
    match (e.direction, r.status) {
        (_, RunStatus::Error) => Err(rej(
            Code::EvidenceUnbound,
            "an error report is not evidence",
        )),
        (Direction::Support, RunStatus::Failed) => {
            Err(rej(Code::NotPassed, "support needs a passed report"))
        }
        (Direction::Counter, RunStatus::Passed) => {
            Err(rej(Code::EvidenceUnbound, "counter needs a failed report"))
        }
        (Direction::Support, RunStatus::Passed)
            if needs_endorsement(st, e.claim, r) && !endorsed_code(st, e.claim, r.candidate) =>
        {
            let paths: Vec<&str> = r.tests_changed.iter().map(|t| t.path.as_str()).collect();
            Err(rej(
                Code::TestsUnendorsed,
                format!(
                    "{} changes tests ({}); another principal must endorse it first",
                    r.candidate,
                    paths.join(", ")
                ),
            ))
        }
        _ => Ok(()),
    }
}

/// Interim test-change rule (owner, 2026-11-02): executed support citing a
/// report needs an endorsement when the candidate modifies or deletes a test
/// file, or adds one while clearing a block (`failing(c)` non-empty). Adding
/// tests alone never needs one.
fn needs_endorsement(st: &TaskState, c: MsgId, r: &Report) -> bool {
    let blocked = st.claims.get(c).is_some_and(|x| !x.failing.is_empty());
    r.tests_changed
        .iter()
        .any(|t| t.change != FileChange::Added || blocked)
}

/// Interim test-change rule: `k`'s test change on `c` is endorsed by cited
/// support citing `k` from a principal who authored neither `k` nor its diffs.
fn endorsed(st: &TaskState, c: MsgId, k: ArtId) -> bool {
    endorsement(st, c, k).is_some()
}

/// `endorsed`, where a holdout candidate is covered by the endorsement of
/// the current candidate whose code it builds.
fn endorsed_code(st: &TaskState, c: MsgId, k: ArtId) -> bool {
    if endorsed(st, c, k) {
        return true;
    }
    let Some(rec) = st.claims.get(c) else {
        return false;
    };
    match rec.current {
        Some(cur) if cur != k && builds_current(st, rec, k) => endorsed(st, c, cur),
        _ => false,
    }
}

/// The first evidence on `c` that endorses `k`'s test change, if any.
pub(crate) fn endorsement(st: &TaskState, c: MsgId, k: ArtId) -> Option<MsgId> {
    let rec = st.claims.get(c)?;
    let authors = authors_of(st, k);
    rec.items
        .iter()
        .filter(|i| i.kind == EvidenceKind::Cited && i.direction == Direction::Support)
        .filter(|i| !authors.contains(&i.author))
        .find(|i| cites(st, i.msg, k))
        .map(|i| i.msg)
}

/// A `tests` claim with a candidate accepts only with scoring support from a
/// principal who authored neither the candidate nor its diffs (#597 G2): a
/// builder never weakens protected tests on its own word.
fn tests_supported(st: &TaskState, c: &ClaimRecord) -> bool {
    let Some(k) = c.current else {
        return true;
    };
    if !c.body.domains.iter().any(|d| d == TESTS_DOMAIN) {
        return true;
    }
    let authors = authors_of(st, k);
    c.items.iter().any(|i| {
        i.scoring
            && i.epoch == c.epoch
            && i.direction == Direction::Support
            && !authors.contains(&i.author)
    })
}

/// The author of candidate `k` and of each of its diffs.
fn authors_of(st: &TaskState, k: ArtId) -> Vec<PrincipalId> {
    let Some(a) = st.artifacts.get(&k) else {
        return Vec::new();
    };
    let mut out = vec![a.author.clone()];
    if let ArtifactPayload::Candidate(c) = &a.payload {
        out.extend(
            c.diffs
                .iter()
                .filter_map(|d| st.artifacts.get(d))
                .map(|d| d.author.clone()),
        );
    }
    out
}

fn cites(st: &TaskState, msg: MsgId, k: ArtId) -> bool {
    st.message(msg).is_some_and(|m| {
        m.draft
            .evidence
            .iter()
            .any(|r| matches!(r, Ref::Art { id, .. } if *id == k))
    })
}

// ---------------------------------------------------------------- the update rule

/// After evidence `m` on `c` is appended: score it, reopen on executed
/// evidence against a settled outcome, or record it (Evidence after settlement).
fn after_evidence(w: &mut Work, m: MsgId, c: MsgId, e: &EvidenceBody, d: &Draft) {
    if e.kind == EvidenceKind::Cited && e.direction == Direction::Support {
        clear_if_endorsed(w, c, d);
    }
    let against = match (status(&w.st, c), e.kind, e.direction) {
        (Some(ClaimStatus::Open), ..) => {
            evaluate(w, c);
            return;
        }
        (Some(ClaimStatus::Accepted), EvidenceKind::Executed, Direction::Counter) => true,
        (Some(ClaimStatus::Dropped), EvidenceKind::Executed, Direction::Support) => true,
        _ => false,
    };
    // A claim resting on a closed dep is dead: evidence is recorded only.
    if !against || closed_dep(&w.st, c).is_some() {
        return; // recorded
    }
    if acted(&w.st, c) {
        escalate(w, c, ClaimEscalation::ReopenedAfterAction);
    } else {
        reopen(w, c, ReopenCause::Evidence(m));
        budget_line(w, c);
    }
}

/// An endorsement unlocks the latest report of the current candidate, and
/// of each holdout candidate building its code, when it passed with a test
/// change (a holdout report also under the task's current holdout digest):
/// its listed profiles leave `failing(c)` at this commit, each only when the
/// report is newer than the latest failure of that profile on the code.
fn clear_if_endorsed(w: &mut Work, c: MsgId, d: &Draft) {
    let Some(rec) = w.st.claims.get(c) else {
        return;
    };
    let Some(k) = rec.current else {
        return;
    };
    let names_k = d
        .evidence
        .iter()
        .any(|r| matches!(r, Ref::Art { id, .. } if *id == k));
    if !names_k || !endorsed(&w.st, c, k) {
        return;
    }
    let passes: Vec<(ArtId, Vec<String>)> =
        w.st.claims
            .candidates
            .iter()
            .filter(|(id, claim)| **claim == c && builds_current(&w.st, rec, **id))
            .filter_map(|(id, _)| unlockable(&w.st, c, *id))
            .collect();
    let st = &w.st;
    let cleared: Vec<String> = rec
        .failing
        .iter()
        .filter(|n| {
            let failed = last_failure(st, rec, n);
            passes
                .iter()
                .any(|(id, listed)| listed.contains(n) && failed.is_none_or(|f| f < *id))
        })
        .cloned()
        .collect();
    set_failing(w, c, |f| {
        for n in &cleared {
            f.remove(n);
        }
    });
}

/// The latest failed report on `c`'s current code that failed profile `n`.
/// Derived from the runs, since `failing_from` keeps the first failure.
fn last_failure(st: &TaskState, c: &ClaimRecord, n: &str) -> Option<ArtId> {
    st.claims
        .runs
        .values()
        .filter(|r| builds_current(st, c, r.candidate))
        .filter_map(|r| r.report)
        .filter(|id| {
            report_of(st, *id).is_some_and(|rep| {
                rep.status == RunStatus::Failed && rep.failed_profiles().iter().any(|p| p == n)
            })
        })
        .max()
}

/// `k`'s latest report and its listed profiles, when it passed with a test
/// change and, for a holdout candidate, under the current holdout digest and
/// on the tree the current candidate tested.
fn unlockable(st: &TaskState, c: MsgId, k: ArtId) -> Option<(ArtId, Vec<String>)> {
    let id = st.claims.latest_report(k)?;
    let r = report_of(st, id)?;
    let holdout = manifest(st, k).is_some_and(|m| m.suite == Suite::Holdout);
    let current_suite = !holdout || Some(r.profile_digest) == st.claims.holdout_digest;
    let ok = r.status == RunStatus::Passed
        && !r.tests_changed.is_empty()
        && current_suite
        && on_tested_tree(st, c, k, r);
    ok.then(|| (id, r.listed_profiles()))
}

/// Can a pass of `k` on `c` speak for the current code? The current
/// candidate's own report always can; a holdout candidate's only on the tree
/// (`Some`) that the current candidate's latest report tested, since equal
/// manifests may resolve a moving base to another tree.
fn on_tested_tree(st: &TaskState, c: MsgId, k: ArtId, rep: &Report) -> bool {
    let Some(cur) = st.claims.get(c).and_then(|r| r.current) else {
        return false;
    };
    if k == cur {
        return true;
    }
    let tested = latest_report(st, cur).and_then(|r| r.tree);
    tested.is_some() && rep.tree == tested
}

fn report_of(st: &TaskState, id: ArtId) -> Option<&Report> {
    match &st.artifacts.get(&id)?.payload {
        ArtifactPayload::TestReport(r) => Some(r),
        _ => None,
    }
}

fn latest_report(st: &TaskState, k: ArtId) -> Option<Report> {
    report_of(st, st.claims.latest_report(k)?).cloned()
}

fn set_failing(w: &mut Work, c: MsgId, f: impl FnOnce(&mut std::collections::BTreeSet<String>)) {
    let Some(rec) = w.st.claims.get(c) else {
        return;
    };
    let mut next = rec.failing.clone();
    f(&mut next);
    if next != rec.failing {
        w.emit(Event::ClaimFailing {
            claim: c,
            failing: next.into_iter().collect(),
        });
    }
}

/// The first dep of `c` that is dropped, void or superseded.
fn closed_dep(st: &TaskState, c: MsgId) -> Option<MsgId> {
    st.claims.get(c).and_then(|r| st.claims.closed_dep(r))
}

fn acted(st: &TaskState, c: MsgId) -> bool {
    st.claims.get(c).is_some_and(|r| r.acted_on())
}

/// Evaluate an `open` claim: accept, drop, budget, settling, first match wins.
fn evaluate(w: &mut Work, c: MsgId) {
    let cfg = w.st.claims.config;
    let Some(rec) = w.st.claims.get(c) else {
        return;
    };
    if rec.status != ClaimStatus::Open {
        return;
    }
    if accept_line(&w.st, rec) {
        accept(w, c, false);
    } else if drop_line(rec, &cfg) {
        let (score, evidence) = (rec.score(), rec.items.len() as u64);
        w.emit(Event::ClaimDropped {
            claim: c,
            score,
            evidence,
        });
        changed(w, c, ClaimStatus::Open);
    } else if rec.exhausted(&cfg) {
        escalate(w, c, ClaimEscalation::Budget);
    } else if rec.settled_flat(&cfg) {
        escalate(w, c, ClaimEscalation::Stuck);
    }
}

fn accept_line(st: &TaskState, c: &ClaimRecord) -> bool {
    let cfg = &st.claims.config;
    c.score() >= c.accept_at(cfg)
        && c.authors(Direction::Support) >= cfg.min_authors as usize
        && c.grounded(Direction::Support)
        && c.failing.is_empty()
        && holdout_passed(st, c)
        && tests_supported(st, c)
        && c.body
            .deps
            .iter()
            .all(|d| status(st, *d) == Some(ClaimStatus::Accepted))
}

fn drop_line(c: &ClaimRecord, cfg: &ClaimsConfig) -> bool {
    c.score() <= cfg.drop_at
        && c.authors(Direction::Counter) >= cfg.min_authors as usize
        && c.grounded(Direction::Counter)
}

/// The budget line alone, for a claim just reopened by anything but a human.
fn budget_line(w: &mut Work, c: MsgId) {
    let cfg = w.st.claims.config;
    if w.st
        .claims
        .get(c)
        .is_some_and(|r| r.status == ClaimStatus::Open && r.exhausted(&cfg))
    {
        escalate(w, c, ClaimEscalation::Budget);
    }
}

/// Accept `c`, by the accept line or by the human (who bypasses the
/// irreversible and acted-on supersession escalations). Takes supersession
/// effect in the same transaction.
fn accept(w: &mut Work, c: MsgId, human: bool) {
    let Some(rec) = w.st.claims.get(c).cloned() else {
        return;
    };
    if !human && !rec.body.reversible {
        escalate(w, c, ClaimEscalation::Irreversible);
        return;
    }
    let mut lapsed = None;
    let mut retire = None;
    if let Some(x) = rec.body.supersedes {
        match w.st.claims.get(x) {
            Some(xr) if xr.status == ClaimStatus::Superseded => lapsed = Some(x),
            Some(xr) if !human && xr.acted_on() => {
                escalate(w, c, ClaimEscalation::SupersedesActed);
                return;
            }
            Some(_) => retire = Some(x),
            None => {}
        }
    }
    let sole = rec
        .body
        .answers
        .is_some_and(|q| w.st.claims.candidates_of(q).len() < 2);
    w.emit(Event::ClaimAccepted {
        claim: c,
        score: rec.score(),
        evidence: rec.items.len() as u64,
        sole_candidate: sole,
        supersedes_lapsed: lapsed,
    });
    if let Some(x) = retire {
        supersede(w, x, c);
    }
    changed(w, c, rec.status);
}

/// `x` is replaced by the accepted `by`: closed whatever its status, held
/// reruns withdrawn (by the fold), dependents handled as for a closed dep.
fn supersede(w: &mut Work, x: MsgId, by: MsgId) {
    let Some(prior) = status(&w.st, x) else {
        return;
    };
    w.emit(Event::ClaimSuperseded {
        claim: x,
        by,
        prior,
    });
    changed(w, x, prior);
}

fn escalate(w: &mut Work, c: MsgId, reason: ClaimEscalation) {
    let Some(was) = status(&w.st, c) else {
        return;
    };
    w.emit(Event::ClaimEscalated { claim: c, reason });
    changed(w, c, was);
}

fn reopen(w: &mut Work, c: MsgId, by: ReopenCause) {
    let Some(rec) = w.st.claims.get(c) else {
        return;
    };
    let (was, epoch) = (rec.status, rec.epoch + 1);
    w.emit(Event::ClaimReopened {
        claim: c,
        by,
        epoch,
    });
    changed(w, c, was);
}

fn void(w: &mut Work, c: MsgId, by: MsgId) {
    let Some(was) = status(&w.st, c) else {
        return;
    };
    w.emit(Event::ClaimVoided { claim: c, by });
    changed(w, c, was);
}

// ---------------------------------------------------------------- dependencies

/// `c` went from `was` to its current status: carry the change to its
/// dependents (Dependencies). Recursion reaches the fixed point; the graph is
/// acyclic because deps name earlier claims only.
fn changed(w: &mut Work, c: MsgId, was: ClaimStatus) {
    let Some(now) = status(&w.st, c) else {
        return;
    };
    if now == was {
        return;
    }
    let dependents = w.st.claims.dependents(c);
    match now {
        ClaimStatus::Accepted => {
            for e in dependents {
                evaluate(w, e);
            }
        }
        ClaimStatus::Dropped | ClaimStatus::Void | ClaimStatus::Superseded => {
            for e in dependents {
                collapse(w, e, c);
            }
        }
        ClaimStatus::Open | ClaimStatus::Escalated if was == ClaimStatus::Accepted => {
            for e in dependents {
                left_accepted(w, e, c);
            }
        }
        ClaimStatus::Open | ClaimStatus::Escalated => {}
    }
}

/// Dep `d` closed (dropped, void or superseded: all alike, owner 2026-11-02).
/// An unacted dependent is voided; an acted-on one escalates and is never
/// auto-voided, whatever its own status.
fn collapse(w: &mut Work, e: MsgId, d: MsgId) {
    let acted = acted(&w.st, e);
    match status(&w.st, e) {
        Some(ClaimStatus::Open | ClaimStatus::Accepted) if acted => {
            escalate(w, e, ClaimEscalation::DepCollapsed)
        }
        Some(ClaimStatus::Open | ClaimStatus::Escalated) if !acted => void(w, e, d),
        Some(ClaimStatus::Accepted) => {
            reopen(w, e, ReopenCause::Dep(d));
            void(w, e, d);
        }
        _ => {}
    }
}

/// Dep `d` left `accepted` but is not closed (reopened or escalated).
fn left_accepted(w: &mut Work, e: MsgId, d: MsgId) {
    if status(&w.st, e) != Some(ClaimStatus::Accepted) {
        return; // open dependents wait for the dep
    }
    if acted(&w.st, e) {
        escalate(w, e, ClaimEscalation::DepCollapsed);
    } else {
        reopen(w, e, ReopenCause::Dep(d));
        budget_line(w, e);
    }
}

// ---------------------------------------------------------------- runner

pub(crate) fn runner(
    w: &mut Work,
    run: RunId,
    secret: RunSecret,
    blob: BlobRef,
    payload: &ArtifactPayload,
) -> R<Response> {
    let kind = payload.kind();
    if !matches!(kind, ArtifactKind::TestReport | ArtifactKind::Log) {
        return Err(rej(
            Code::KindForbidden,
            "runner writes reports and logs only",
        ));
    }
    let r =
        w.st.claims
            .runs
            .get(&run)
            .cloned()
            .ok_or_else(|| rej(Code::StaleRun, "no such run"))?;
    if r.revoked {
        return Err(rej(Code::StaleRun, "run revoked"));
    }
    if w.st.secret_for(run) != secret {
        return Err(rej(Code::BadRunSecret, "bad run secret"));
    }
    if blob.len > MAX_ARTIFACT_BYTES {
        return Err(rej(Code::TooLarge, "artifact exceeds max_artifact_bytes"));
    }
    let dup = match kind {
        ArtifactKind::TestReport => r.report.is_some(),
        _ => r.log.is_some(),
    };
    if dup {
        return Err(rej(Code::DuplicateReport, "run already has this artifact"));
    }
    if let ArtifactPayload::TestReport(rep) = payload {
        check_report(&w.st, run, &r, rep)?;
    }
    let id = w.st.next_art_id();
    w.emit(Event::ArtifactStored(ArtifactRecord {
        id,
        author: PrincipalId(RUNNER.into()),
        kind,
        blob,
        payload: payload.clone(),
        spec: None,
        candidate: Some(r.candidate),
        run: Some(run),
    }));
    if let ArtifactPayload::TestReport(rep) = payload {
        w.emit(Event::TestReportStored {
            candidate: r.candidate,
            run,
            status: rep.status,
            report: id,
        });
        on_report(w, r.candidate, rep);
    }
    Ok(Response::Stored(id))
}

/// R16 as in stage mode, against the claims-mode run.
fn check_report(st: &TaskState, run: RunId, r: &ClaimRun, rep: &Report) -> R<()> {
    if rep.candidate != r.candidate || rep.run != run {
        return Err(rej(
            Code::InconsistentReport,
            "report names another candidate or run",
        ));
    }
    let suite = manifest(st, r.candidate).map_or(Suite::Build, |k| k.suite);
    let digest = match suite {
        Suite::Build => Some(st.profile_digest),
        Suite::Holdout => st.claims.holdout_digest,
    };
    if Some(rep.profile_digest) != digest {
        return Err(rej(
            Code::ProfileMismatch,
            format!("profile digest is not the task's {suite:?} suite"),
        ));
    }
    if let Some(p) = rep.profiles.iter().find(|p| !suite.owns_profile(&p.name)) {
        return Err(rej(
            Code::InconsistentReport,
            format!("profile {:?} is not a {suite:?} suite profile", p.name),
        ));
    }
    if !rep.consistent() {
        return Err(rej(
            Code::InconsistentReport,
            "status disagrees with profile results",
        ));
    }
    let log = resolve_kind(st, rep.log, ArtifactKind::Log)?;
    if log.run != Some(run) {
        return Err(rej(Code::InconsistentReport, "log belongs to another run"));
    }
    Ok(())
}

/// Failing runs block: a final report of the claim's current candidate
/// updates `failing(c)`; then an open claim is evaluated.
fn on_report(w: &mut Work, k: ArtId, rep: &Report) {
    let Some(c) = w.st.claims.candidates.get(&k).copied() else {
        return;
    };
    if !w
        .st
        .claims
        .get(c)
        .is_some_and(|r| builds_current(&w.st, r, k))
    {
        return; // a replaced candidate cannot block
    }
    match rep.status {
        RunStatus::Failed => {
            let failed = rep.failed_profiles();
            set_failing(w, c, |f| f.extend(failed));
        }
        RunStatus::Passed if !on_tested_tree(&w.st, c, k, rep) => {}
        RunStatus::Passed if rep.tests_changed.is_empty() || endorsed_code(&w.st, c, k) => {
            let listed = rep.listed_profiles();
            set_failing(w, c, |f| {
                for n in &listed {
                    f.remove(n);
                }
            });
        }
        RunStatus::Passed | RunStatus::Error => {}
    }
    evaluate(w, c);
}

// ---------------------------------------------------------------- human

pub(crate) fn human(w: &mut Work, a: &HumanAction, now: Time) -> R<Response> {
    match a {
        HumanAction::Post(d) => {
            let human = w.st.human.clone();
            post(w, &human, None, d)
        }
        HumanAction::ClaimDecision { claim, outcome } => decide(w, *claim, *outcome),
        HumanAction::Choose { question, chosen } => choose(w, *question, *chosen),
        HumanAction::ClaimBudgetExtended { claim } => extend(w, *claim),
        HumanAction::Rerun {
            candidate,
            expected_rev,
        } => {
            check_rev(&w.st, *expected_rev)?;
            rerun(w, *candidate)
        }
        HumanAction::WithdrawRerun { candidate } => {
            if !w.st.claims.held.contains_key(candidate) {
                return Err(rej(Code::NotOpen, format!("no rerun held on {candidate}")));
            }
            w.emit(Event::RerunWithdrawn {
                candidate: *candidate,
            });
            Ok(Response::Ok)
        }
        HumanAction::ClaimActionResult { mark, outcome } => action_result(w, *mark, *outcome),
        HumanAction::Takeover {
            claim,
            session,
            client,
            minutes,
            reason,
        } => takeover_start(w, *claim, session, client, *minutes, reason, now),
        HumanAction::TakeoverRelease { takeover } => {
            takeover_end(w, *takeover, TakeoverEnd::Released, now)
        }
        HumanAction::HoldoutUpdated { digest } => {
            new_holdout(w, *digest, Event::HoldoutUpdated { digest: *digest })
        }
        HumanAction::Close { expected_rev } => {
            check_rev(&w.st, *expected_rev)?;
            transition(w, State::Closed, now);
            Ok(Response::Ok)
        }
        HumanAction::Approve { .. }
        | HumanAction::Reject { .. }
        | HumanAction::MergeReceipt { .. } => Err(rej(Code::WrongMode, "claims mode has no gates")),
        HumanAction::Fenced { .. }
        | HumanAction::Resume { .. }
        | HumanAction::BudgetExtended { .. }
        | HumanAction::Cancel { .. } => Err(rej(Code::Forbidden, "handled by the engine")),
    }
}

/// C-R8 on one claim.
fn decide(w: &mut Work, c: MsgId, outcome: ClaimOutcome) -> R<Response> {
    let rec = claim_of(&w.st, c)?.clone();
    let cfg = w.st.claims.config;
    let allowed = match outcome {
        ClaimOutcome::Accept => {
            !matches!(rec.status, ClaimStatus::Accepted | ClaimStatus::Superseded)
        }
        ClaimOutcome::Drop => !matches!(rec.status, ClaimStatus::Dropped | ClaimStatus::Superseded),
        ClaimOutcome::Reopen => !matches!(rec.status, ClaimStatus::Open | ClaimStatus::Superseded),
    };
    if !allowed {
        return Err(rej(
            Code::WrongState,
            format!("cannot {outcome:?} a claim that is {}", rec.status.name()),
        ));
    }
    if let (ClaimOutcome::Reopen, Some(d)) = (outcome, closed_dep(&w.st, c)) {
        return Err(rej(
            Code::DepClosed,
            format!(
                "{c} rests on {d}, which is {}: reopen {d} first",
                status(&w.st, d).map_or("closed", |s| s.name())
            ),
        ));
    }
    if outcome == ClaimOutcome::Reopen && rec.exhausted(&cfg) {
        return Err(rej(
            Code::WrongState,
            format!("{c} has no budget left: record budget_extended first"),
        ));
    }
    w.emit(Event::ClaimDecided { claim: c, outcome });
    match outcome {
        ClaimOutcome::Accept => accept(w, c, true),
        ClaimOutcome::Drop => human_drop(w, c),
        ClaimOutcome::Reopen => {
            // Dormant evidence now scores in the new epoch, so evaluate it.
            reopen(w, c, ReopenCause::Human);
            evaluate(w, c);
        }
    }
    Ok(Response::Ok)
}

fn human_drop(w: &mut Work, c: MsgId) {
    let Some(rec) = w.st.claims.get(c) else {
        return;
    };
    let (was, score, evidence) = (rec.status, rec.score(), rec.items.len() as u64);
    w.emit(Event::ClaimDropped {
        claim: c,
        score,
        evidence,
    });
    changed(w, c, was);
}

/// C-R8 `choose`: accept `chosen`, drop the question's other live candidates.
fn choose(w: &mut Work, q: MsgId, chosen: MsgId) -> R<Response> {
    claim_of(&w.st, q)?;
    let rec = claim_of(&w.st, chosen)?;
    if rec.body.answers != Some(q) {
        return Err(rej(Code::BadClaim, format!("{chosen} does not answer {q}")));
    }
    if matches!(rec.status, ClaimStatus::Superseded) {
        return Err(rej(Code::WrongState, format!("{chosen} is superseded")));
    }
    let others: Vec<MsgId> =
        w.st.claims
            .candidates_of(q)
            .into_iter()
            .filter(|c| *c != chosen)
            .collect();
    w.emit(Event::ClaimDecided {
        claim: chosen,
        outcome: ClaimOutcome::Accept,
    });
    if status(&w.st, chosen) != Some(ClaimStatus::Accepted) {
        accept(w, chosen, true);
    }
    for c in others {
        if status(&w.st, c).is_some_and(|s| {
            !matches!(
                s,
                ClaimStatus::Dropped | ClaimStatus::Void | ClaimStatus::Superseded
            )
        }) {
            w.emit(Event::ClaimDecided {
                claim: c,
                outcome: ClaimOutcome::Drop,
            });
            human_drop(w, c);
        }
    }
    Ok(Response::Ok)
}

/// `budget_extended(claim)`: one more `base(c)`, then any held rerun starts.
fn extend(w: &mut Work, c: MsgId) -> R<Response> {
    let rec = claim_of(&w.st, c)?;
    // Refuse rather than wrap: the extended budget must be representable.
    let next = rec
        .extensions
        .checked_add(2)
        .and_then(|n| rec.base(&w.st.claims.config).checked_mul(n));
    if next.is_none() {
        return Err(rej(
            Code::ClaimBudget,
            format!("{c}: budget would overflow"),
        ));
    }
    w.emit(Event::ClaimBudgetExtended { claim: c });
    let held: Vec<ArtId> =
        w.st.claims
            .held
            .iter()
            .filter(|(_, claim)| **claim == c)
            .map(|(k, _)| *k)
            .collect();
    let cfg = w.st.claims.config;
    for k in held {
        // Held reruns pass the same admission as a fresh `rerun`.
        if !w.st.claims.get(c).is_some_and(|r| rerun_admits(r.status)) {
            w.emit(Event::RerunWithdrawn { candidate: k });
        } else if w.st.claims.get(c).is_some_and(|r| !r.exhausted(&cfg)) {
            start_run(w, c, k);
        }
    }
    evaluate(w, c);
    Ok(Response::Ok)
}

/// A human rerun: debit and start, or hold it at the budget limit.
fn rerun(w: &mut Work, k: ArtId) -> R<Response> {
    let c = w.st.claims.candidates.get(&k).copied().ok_or_else(|| {
        rej(
            Code::RefUnresolved,
            format!("{k} is not a claims candidate"),
        )
    })?;
    let rec = claim_of(&w.st, c)?;
    if !rerun_admits(rec.status) {
        return Err(rej(
            Code::ClaimNotOpen,
            format!("{c} is {}", rec.status.name()),
        ));
    }
    if w.st.claims.held.contains_key(&k) {
        return Ok(Response::Ok); // the held one stands
    }
    if rec.exhausted(&w.st.claims.config) {
        w.emit(Event::RerunHeld {
            claim: c,
            candidate: k,
        });
        return Ok(Response::Ok);
    }
    w.emit(Event::Rerun { candidate: k });
    start_run(w, c, k);
    evaluate(w, c);
    Ok(Response::Ok)
}

// ---------------------------------------------------------------- host: action marks

pub(crate) fn host(w: &mut Work, a: &HostAction, now: Time) -> R<Response> {
    match a {
        HostAction::Acting {
            claim,
            epoch,
            reference,
        } => {
            let rec = claim_of(&w.st, *claim)?;
            if rec.status != ClaimStatus::Accepted || rec.epoch != *epoch {
                return Err(rej(
                    Code::WrongState,
                    format!(
                        "{claim} is {} in epoch {}, not accepted in epoch {epoch}",
                        rec.status.name(),
                        rec.epoch
                    ),
                ));
            }
            let mark = w.st.claims.next_mark;
            w.emit(Event::ClaimActing {
                claim: *claim,
                epoch: *epoch,
                mark,
                reference: reference.clone(),
                at: now,
            });
            Ok(Response::Marked(mark))
        }
        HostAction::ActionResult { mark, outcome } => action_result(w, *mark, *outcome),
        HostAction::HoldoutExtended { digest } => {
            new_holdout(w, *digest, Event::HoldoutExtended { digest: *digest })
        }
        HostAction::TakeoverInput {
            takeover,
            seq,
            text,
            sha256,
        } => takeover_input(w, *takeover, *seq, text, *sha256, now),
        HostAction::TakeoverEnded { takeover, reason } => {
            if *reason == TakeoverEnd::Released {
                return Err(rej(Code::Forbidden, "only the human releases a takeover"));
            }
            takeover_end(w, *takeover, *reason, now)
        }
    }
}

// ---------------------------------------------------------------- takeover (D82)

/// The longest takeover one `takeover` grants.
pub const MAX_TAKEOVER_MINUTES: u32 = 60;
/// The largest input batch, in bytes of text.
pub const MAX_TAKEOVER_BATCH: usize = 4096;
/// The longest session or client name.
const MAX_TAKEOVER_NAME: usize = 128;
/// The longest reason.
const MAX_TAKEOVER_REASON: usize = 500;

fn bounded(field: &str, value: &str, max: usize) -> R<()> {
    if value.trim().is_empty() {
        return Err(rej(Code::BadHumanInput, format!("{field} is empty")));
    }
    if value.len() > max {
        return Err(rej(Code::TooLarge, format!("{field} is over {max} bytes")));
    }
    Ok(())
}

/// D82: written before any input is possible; one live takeover per session.
fn takeover_start(
    w: &mut Work,
    claim: MsgId,
    session: &str,
    client: &str,
    minutes: u32,
    reason: &str,
    now: Time,
) -> R<Response> {
    bounded("session", session, MAX_TAKEOVER_NAME)?;
    bounded("client", client, MAX_TAKEOVER_NAME)?;
    bounded("reason", reason, MAX_TAKEOVER_REASON)?;
    if minutes == 0 || minutes > MAX_TAKEOVER_MINUTES {
        return Err(rej(
            Code::BadHumanInput,
            format!("minutes must be 1 to {MAX_TAKEOVER_MINUTES}"),
        ));
    }
    let rec = claim_of(&w.st, claim)?;
    if rec.closed() {
        return Err(rej(
            Code::WrongState,
            format!("{claim} is {}", rec.status.name()),
        ));
    }
    let epoch = rec.epoch;
    let book = &w.st.claims;
    if book
        .takeovers
        .values()
        .any(|t| t.live() && t.session == session)
    {
        return Err(rej(
            Code::AlreadyOpen,
            format!("session {session} is already taken over"),
        ));
    }
    let takeover = book.takeovers.len() as u64 + 1;
    w.emit(Event::TakeoverStarted {
        takeover,
        claim,
        epoch,
        session: session.to_owned(),
        client: client.to_owned(),
        until: now.plus(u64::from(minutes) * 60_000),
        reason: reason.to_owned(),
    });
    Ok(Response::TakenOver(takeover))
}

fn live_takeover(w: &Work, id: u64) -> R<&Takeover> {
    let t =
        w.st.claims
            .takeovers
            .get(&id)
            .ok_or_else(|| rej(Code::RefUnresolved, format!("no takeover {id}")))?;
    if !t.live() {
        return Err(rej(Code::NotOpen, format!("takeover {id} has ended")));
    }
    Ok(t)
}

/// D82: batches arrive in order and extend the chain; none after `until`.
fn takeover_input(
    w: &mut Work,
    id: u64,
    seq: u64,
    text: &str,
    sha256: Sha256,
    now: Time,
) -> R<Response> {
    let t = live_takeover(w, id)?;
    if now > t.until {
        return Err(rej(
            Code::TokenExpired,
            format!("takeover {id} expired; end it"),
        ));
    }
    if seq != t.batches + 1 {
        return Err(rej(
            Code::OpConflict,
            format!("takeover {id} expects batch {}, not {seq}", t.batches + 1),
        ));
    }
    if text.is_empty() {
        return Err(rej(Code::BadHumanInput, "empty input batch"));
    }
    if text.len() > MAX_TAKEOVER_BATCH {
        return Err(rej(
            Code::TooLarge,
            format!("input batch is over {MAX_TAKEOVER_BATCH} bytes"),
        ));
    }
    if input_chain(t.last, text) != sha256 {
        return Err(rej(
            Code::PayloadMismatch,
            format!("batch {seq} does not extend the chain of takeover {id}"),
        ));
    }
    w.emit(Event::TakeoverInput {
        takeover: id,
        seq,
        text: text.to_owned(),
        sha256,
    });
    Ok(Response::Ok)
}

/// D82: the totals come from the fold, not the caller.
fn takeover_end(w: &mut Work, id: u64, reason: TakeoverEnd, now: Time) -> R<Response> {
    let t = live_takeover(w, id)?;
    if reason == TakeoverEnd::Expired && now < t.until {
        return Err(rej(
            Code::WrongState,
            format!("takeover {id} has not expired"),
        ));
    }
    let (batches, last) = (t.batches, t.last);
    w.emit(Event::TakeoverEnded {
        takeover: id,
        reason,
        batches,
        last,
    });
    Ok(Response::Ok)
}

/// A new holdout suite version (#597 G3). Holdout failures of the old one are
/// retired: every open claim with a candidate needs a fresh holdout pass under
/// the new digest anyway, which re-runs whatever the new suite keeps.
fn new_holdout(w: &mut Work, digest: Sha256, e: Event) -> R<Response> {
    match w.st.claims.holdout_digest {
        None => return Err(rej(Code::HoldoutMismatch, "the task has no holdout suite")),
        Some(d) if d == digest => {
            return Err(rej(
                Code::Duplicate,
                "that holdout suite is already current",
            ))
        }
        Some(_) if w.st.claims.holdout_retired.contains(&digest) => {
            return Err(rej(
                Code::Duplicate,
                "that holdout suite was retired; adopt a new version instead",
            ))
        }
        Some(_) => {}
    }
    w.emit(e);
    let blocked: Vec<MsgId> =
        w.st.claims
            .claims
            .values()
            .filter(|c| c.failing.iter().any(|p| Suite::Holdout.owns_profile(p)))
            .map(|c| c.id)
            .collect();
    for c in blocked {
        set_failing(w, c, |f| f.retain(|p| !Suite::Holdout.owns_profile(p)));
    }
    Ok(Response::Ok)
}

fn action_result(w: &mut Work, mark: u64, outcome: MarkOutcome) -> R<Response> {
    let (_, m) = mark_owner(&w.st.claims, mark)
        .ok_or_else(|| rej(Code::RefUnresolved, format!("no mark {mark}")))?;
    if m.outcome.is_some() {
        return Err(rej(Code::Duplicate, format!("mark {mark} has a result")));
    }
    w.emit(Event::ClaimActionResult { mark, outcome });
    Ok(Response::Ok)
}

/// Overdue marks escalate `action_unconfirmed`, once each; the claim keeps
/// its status.
pub(crate) fn fire_marks(w: &mut Work, now: Time) {
    let due: Vec<(MsgId, u64)> =
        w.st.claims
            .pending_marks()
            .filter(|(_, _, t)| *t <= now)
            .map(|(c, m, _)| (c, m))
            .collect();
    for (claim, mark) in due {
        w.emit(Event::ClaimEscalated {
            claim,
            reason: ClaimEscalation::ActionUnconfirmed(mark),
        });
    }
}
