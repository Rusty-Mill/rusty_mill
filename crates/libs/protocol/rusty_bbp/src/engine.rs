//! The processing model: validation in rule order, effects, transitions, entry
//! actions and generated events. `handle` is the only entry point and is pure.

use crate::command::*;
use crate::event::*;
use crate::ids::*;
use crate::record::*;
use crate::state::*;

pub const MAX_ARTIFACT_BYTES: u64 = 2_000_000;
const RUNNER: &str = "runner";

/// Result of handling one command: the events to append and the response to return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handled {
    pub events: Vec<Event>,
    pub response: Response,
}

type R<T> = Result<T, Rejection>;

fn rej(code: Code, detail: impl Into<String>) -> Rejection {
    Rejection::new(code, detail)
}

/// Working copy: applies each emitted event immediately so later rules see it.
struct Work {
    st: TaskState,
    events: Vec<Event>,
}

impl Work {
    fn emit(&mut self, e: Event) {
        self.st.apply(&e);
        self.events.push(e);
    }
}

enum Flow {
    Done(Response),
    Replay(Response),
}

/// Handle one command against a state at time `now`.
pub fn handle(st: &TaskState, cmd: &Command, now: Time) -> Handled {
    let mut w = Work {
        st: st.clone(),
        events: Vec::new(),
    };
    if w.st.opened {
        fire_deadlines(&mut w, now);
    }
    match dispatch(&mut w, cmd, now) {
        Ok(Flow::Replay(resp)) => Handled {
            events: w.events,
            response: resp,
        },
        Ok(Flow::Done(resp)) => {
            if let Some((p, op, hash)) = op_key(&w.st, cmd) {
                w.emit(Event::OpAccepted {
                    principal: p,
                    op,
                    payload: hash,
                    response: resp.clone(),
                });
            }
            evaluate_budgets(&mut w, now);
            Handled {
                events: w.events,
                response: resp,
            }
        }
        Err(r) => {
            let principal = principal_of(&w.st, cmd);
            w.emit(Event::Rejected {
                principal,
                code: r.code,
            });
            Handled {
                events: w.events,
                response: Response::Rejected(r),
            }
        }
    }
}

fn principal_of(st: &TaskState, cmd: &Command) -> Option<PrincipalId> {
    match cmd {
        Command::Agent { principal, .. } => Some(principal.clone()),
        Command::Runner { .. } => Some(PrincipalId(RUNNER.into())),
        Command::Human { .. } => Some(st.human.clone()),
        _ => None,
    }
}

fn payload_hash<T: rusty_serde::Serialize>(v: &T) -> R<Sha256> {
    rusty_serde::json::to_string(v)
        .map(|b| Sha256::of(b.as_bytes()))
        .map_err(|e| rej(Code::Forbidden, e.to_string()))
}

/// (principal, op, payload hash) for external commands that participate in R15.
fn op_key(st: &TaskState, cmd: &Command) -> Option<(PrincipalId, OpId, Sha256)> {
    match cmd {
        Command::Agent {
            principal,
            op,
            action,
            ..
        } if !matches!(action, AgentAction::TaskCard) => {
            Some((principal.clone(), op.clone(), payload_hash(action).ok()?))
        }
        Command::Runner {
            op,
            run,
            payload,
            blob,
            ..
        } => Some((
            PrincipalId(RUNNER.into()),
            op.clone(),
            payload_hash(&(run, payload, blob)).ok()?,
        )),
        Command::Human { op, action } => {
            Some((st.human.clone(), op.clone(), payload_hash(action).ok()?))
        }
        _ => None,
    }
}

fn dispatch(w: &mut Work, cmd: &Command, now: Time) -> R<Flow> {
    match cmd {
        Command::Open(o) => open(w, o, now).map(Flow::Done),
        Command::Assign { role, principal } => {
            require_open(&w.st)?;
            w.emit(Event::Assigned {
                role: *role,
                principal: principal.clone(),
            });
            schedule(w, now);
            Ok(Flow::Done(Response::Ok))
        }
        Command::Tick => {
            require_open(&w.st)?;
            Ok(Flow::Done(Response::Ok))
        }
        Command::AbortTurn => {
            require_open(&w.st)?;
            end_turn(w, TurnEnd::Aborted, now);
            Ok(Flow::Done(Response::Ok))
        }
        Command::Agent {
            principal,
            token,
            op,
            action,
        } => {
            require_open(&w.st)?;
            let role =
                w.st.role_of(principal)
                    .ok_or_else(|| rej(Code::NotAssigned, "principal is not an assigned agent"))?;
            if !matches!(action, AgentAction::TaskCard) {
                if let Some(r) = replay(&w.st, principal, op, &payload_hash(action)?)? {
                    return Ok(Flow::Replay(r));
                }
                terminal(&w.st)?;
                check_token(&w.st, principal, role, *token, now)?;
            }
            agent(w, principal, role, action, now).map(Flow::Done)
        }
        Command::Runner {
            op,
            run,
            secret,
            blob,
            payload,
        } => {
            require_open(&w.st)?;
            let p = PrincipalId(RUNNER.into());
            if let Some(r) = replay(&w.st, &p, op, &payload_hash(&(run, payload, blob))?)? {
                return Ok(Flow::Replay(r));
            }
            terminal(&w.st)?;
            runner(w, *run, *secret, *blob, payload, now).map(Flow::Done)
        }
        Command::Human { op, action } => {
            require_open(&w.st)?;
            let p = w.st.human.clone();
            if let Some(r) = replay(&w.st, &p, op, &payload_hash(action)?)? {
                return Ok(Flow::Replay(r));
            }
            terminal(&w.st)?;
            human(w, action, now).map(Flow::Done)
        }
    }
}

fn require_open(st: &TaskState) -> R<()> {
    st.opened
        .then_some(())
        .ok_or_else(|| rej(Code::Forbidden, "task not opened"))
}

fn terminal(st: &TaskState) -> R<()> {
    if st.state.terminal() {
        return Err(rej(Code::Terminal, format!("task is {:?}", st.state)));
    }
    Ok(())
}

/// R15. `Ok(Some(resp))` is an exact replay.
fn replay(st: &TaskState, p: &PrincipalId, op: &OpId, hash: &Sha256) -> R<Option<Response>> {
    match st.ops.get(&(p.clone(), op.clone())) {
        None => Ok(None),
        Some((h, resp)) if h == hash => Ok(Some(resp.clone())),
        Some(_) => Err(rej(Code::OpConflict, "op reused with a different payload")),
    }
}

/// R14.
fn check_token(
    st: &TaskState,
    p: &PrincipalId,
    role: Role,
    token: Option<Token>,
    now: Time,
) -> R<()> {
    let token = token.ok_or_else(|| rej(Code::BadToken, "no token"))?;
    let turn = st
        .turn
        .as_ref()
        .ok_or_else(|| rej(Code::TokenExpired, "no live turn"))?;
    if turn.role != role || st.token_for(p, turn.id) != token {
        return Err(rej(Code::StaleTurn, "token is not the live turn's"));
    }
    if now >= turn.deadline {
        return Err(rej(Code::TokenExpired, "turn deadline passed"));
    }
    Ok(())
}

// ---------------------------------------------------------------- open

fn open(w: &mut Work, o: &OpenTask, now: Time) -> R<Response> {
    if w.st.opened {
        return Err(rej(Code::AlreadyOpen, "task already opened"));
    }
    w.emit(Event::TaskOpened {
        task: o.task.clone(),
        repo: o.repo.clone(),
        profile_digest: o.profile_digest,
        budget: o.budget,
        turn_ms: o.turn_ms,
        request_ms: o.request_ms,
        human: o.human.clone(),
    });
    let id = w.st.next_art_id();
    w.emit(Event::ArtifactStored(ArtifactRecord {
        id,
        author: o.human.clone(),
        kind: ArtifactKind::Brief,
        blob: o.brief,
        payload: ArtifactPayload::Brief,
        spec: None,
        candidate: None,
        run: None,
    }));
    entry(w, now);
    Ok(Response::Stored(id))
}

// ---------------------------------------------------------------- agent

fn agent(
    w: &mut Work,
    p: &PrincipalId,
    role: Role,
    action: &AgentAction,
    now: Time,
) -> R<Response> {
    match action {
        AgentAction::TaskCard => Ok(Response::Card(w.st.card())),
        AgentAction::Read { after, limit } => read(w, p, role, *after, *limit, now),
        AgentAction::GetArtifact { id } => get_artifact(w, p, role, *id, now),
        AgentAction::PutArtifact { blob, payload } => put_artifact(w, p, role, *blob, payload, now),
        AgentAction::Post(d) => post(w, p, role, d, now),
    }
}

fn visible(st: &TaskState, role: Role, m: &Message) -> bool {
    if role != Role::Reviewer {
        return true;
    }
    let from_human = m.from == st.human;
    let to_reviewer = m
        .draft
        .to
        .iter()
        .any(|r| matches!(r, Recipient::Role(Role::Reviewer) | Recipient::All));
    let mine = |id: MsgId| {
        st.message(id)
            .map(|x| x.from_role == Some(Role::Reviewer))
            .unwrap_or(false)
    };
    let answers_mine = m.draft.reply_to.map(mine).unwrap_or(false);
    let decides_mine = matches!(&m.draft.body, Body::Decision(d) if mine(d.subject));
    from_human && (to_reviewer || answers_mine || decides_mine)
}

fn readable(role: Role, kind: ArtifactKind) -> bool {
    role != Role::Reviewer || kind != ArtifactKind::Log
}

/// R11 per-turn read budget. Exceeding ends the turn and rejects.
fn charge_read(w: &mut Work, p: &PrincipalId, now: Time) -> R<()> {
    let turn = w.st.turn.as_ref().map(|t| t.id);
    if w.st.spent.reads_this_turn + 1 > w.st.budget.reads_per_turn {
        end_turn(w, TurnEnd::Budget, now);
        return Err(rej(Code::TurnBudgetExhausted, "reads_per_turn exceeded"));
    }
    w.emit(Event::Charged {
        principal: p.clone(),
        turn,
        messages: 0,
        bytes: 0,
        reads: 1,
    });
    Ok(())
}

fn read(
    w: &mut Work,
    p: &PrincipalId,
    role: Role,
    after: Option<MsgId>,
    limit: usize,
    now: Time,
) -> R<Response> {
    charge_read(w, p, now)?;
    let limit = limit.clamp(1, READ_PAGE);
    let start = after.map(|a| a.0 as usize).unwrap_or(0);
    let mut out: Vec<Message> = Vec::new();
    let mut more = false;
    for m in
        w.st.messages
            .iter()
            .skip(start)
            .filter(|m| visible(&w.st, role, m))
    {
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

fn get_artifact(w: &mut Work, p: &PrincipalId, role: Role, id: ArtId, now: Time) -> R<Response> {
    let rec =
        w.st.artifacts
            .get(&id)
            .cloned()
            .ok_or_else(|| rej(Code::RefUnresolved, format!("{id}")))?;
    if !readable(role, rec.kind) {
        return Err(rej(
            Code::RoleForbidden,
            "artifact kind not readable by role",
        ));
    }
    let turn =
        w.st.turn
            .as_ref()
            .map(|t| (t.id, t.read_arts.contains(&id)));
    match turn {
        Some((_, true)) => {}
        Some((tid, false)) => {
            charge_read(w, p, now)?;
            w.emit(Event::ReadDeduped { turn: tid, art: id });
        }
        None => return Err(rej(Code::TokenExpired, "no live turn")),
    }
    Ok(Response::Artifact(rec))
}

/// R16 gating table for agent writes.
fn gate_artifact(st: &TaskState, role: Role, kind: ArtifactKind) -> R<()> {
    let ok = matches!(
        (kind, role, st.state),
        (ArtifactKind::Spec, Role::Planner, State::Planning)
            | (ArtifactKind::Diff, Role::Coder, State::Build)
            | (ArtifactKind::Candidate, Role::Coder, State::Build)
    );
    if matches!(
        kind,
        ArtifactKind::Brief | ArtifactKind::TestReport | ArtifactKind::Log
    ) {
        return Err(rej(Code::KindForbidden, "agents cannot write this kind"));
    }
    if !ok {
        let writer_ok = matches!(
            (kind, role),
            (ArtifactKind::Spec, Role::Planner)
                | (ArtifactKind::Diff | ArtifactKind::Candidate, Role::Coder)
        );
        return Err(if writer_ok {
            rej(Code::WrongState, "kind not writable in this state")
        } else {
            rej(Code::KindForbidden, "role may not write this kind")
        });
    }
    if st.turn.as_ref().map(|t| t.kind) == Some(TurnKind::Consultation) {
        return Err(rej(
            Code::ConsultationOnly,
            "consultation turns do not write artifacts",
        ));
    }
    Ok(())
}

fn put_artifact(
    w: &mut Work,
    p: &PrincipalId,
    role: Role,
    blob: BlobRef,
    payload: &ArtifactPayload,
    now: Time,
) -> R<Response> {
    let kind = payload.kind();
    gate_artifact(&w.st, role, kind)?;
    if blob.len > MAX_ARTIFACT_BYTES {
        return Err(rej(Code::TooLarge, "artifact exceeds max_artifact_bytes"));
    }
    let mut spec = None;
    match payload {
        ArtifactPayload::Spec { brief } => {
            if Some(*brief) != w.st.brief {
                return Err(rej(
                    Code::RefUnresolved,
                    "spec must reference the task brief",
                ));
            }
        }
        ArtifactPayload::Candidate(c) => {
            spec = Some(
                w.st.approved_spec
                    .ok_or_else(|| rej(Code::NoApprovedSpec, "no approved spec"))?,
            );
            for d in &c.diffs {
                resolve_kind(&w.st, *d, ArtifactKind::Diff)?;
            }
        }
        _ => {}
    }
    let id = w.st.next_art_id();
    w.emit(Event::ArtifactStored(ArtifactRecord {
        id,
        author: p.clone(),
        kind,
        blob,
        payload: payload.clone(),
        spec,
        candidate: None,
        run: None,
    }));
    w.emit(Event::Charged {
        principal: p.clone(),
        turn: w.st.turn.as_ref().map(|t| t.id),
        messages: 0,
        bytes: blob.len,
        reads: 0,
    });
    if let (ArtifactKind::Candidate, Some(spec)) = (kind, spec) {
        if let Some(old) = w.st.candidate {
            w.emit(Event::VerdictsStaled { candidate: old });
        }
        w.emit(Event::CandidateSubmitted {
            candidate: id,
            spec,
        });
        end_turn(w, TurnEnd::Candidate, now);
        transition(w, State::Test, now);
    }
    Ok(Response::Stored(id))
}

fn resolve_kind(st: &TaskState, id: ArtId, kind: ArtifactKind) -> R<&ArtifactRecord> {
    let a = st
        .artifacts
        .get(&id)
        .ok_or_else(|| rej(Code::RefUnresolved, format!("{id}")))?;
    if a.kind != kind {
        return Err(rej(
            Code::WrongKind,
            format!("{id} is {:?}, expected {:?}", a.kind, kind),
        ));
    }
    Ok(a)
}

fn resolve_ref(st: &TaskState, r: &Ref) -> R<()> {
    match r {
        Ref::Art { id, fragment } => {
            st.artifacts
                .get(id)
                .ok_or_else(|| rej(Code::RefUnresolved, format!("{id}")))?;
            if fragment
                .as_ref()
                .map(|f| f.len() > FRAGMENT_CAP)
                .unwrap_or(false)
            {
                return Err(rej(Code::FragmentTooLong, "fragment over 128 chars"));
            }
            Ok(())
        }
        Ref::Msg(id) => st
            .message(*id)
            .map(|_| ())
            .ok_or_else(|| rej(Code::RefUnresolved, format!("{id}"))),
    }
}

fn role_may_post(role: Role, kind: MessageKind) -> bool {
    use MessageKind::*;
    match role {
        Role::Planner | Role::Coder => matches!(
            kind,
            Ask | Answer | Propose | Finding | RequestDecision | Pass
        ),
        Role::Tester => matches!(
            kind,
            Ask | Answer | Finding | Verdict | RequestDecision | Pass
        ),
        Role::Reviewer => matches!(kind, Answer | Finding | Verdict | RequestDecision | Pass),
    }
}

fn state_accepts(state: State, kind: MessageKind) -> bool {
    use MessageKind::*;
    match state {
        State::Planning | State::Build => matches!(
            kind,
            Ask | Answer | Propose | Finding | RequestDecision | Pass
        ),
        State::Test => matches!(
            kind,
            Ask | Answer | Finding | Verdict | RequestDecision | Pass
        ),
        State::Review => matches!(kind, Answer | Finding | Verdict | RequestDecision | Pass),
        State::PlanGate | State::MergeGate | State::Approved | State::Escalated => {
            matches!(kind, Answer | Pass)
        }
        State::Closed | State::Cancelled => false,
    }
}

fn body_len(b: &Body) -> R<usize> {
    match b {
        Body::Prose(s) => Ok(s.len()),
        Body::Verdict(v) => {
            for i in v.blocking.iter().chain(v.non_blocking.iter()) {
                if i.issue.len() > BODY_CAP || i.fix.len() > BODY_CAP {
                    return Err(rej(Code::BodyTooLong, "issue or fix over 1,200 chars"));
                }
            }
            Ok(0)
        }
        Body::Decision(d) => Ok(d.note.len()),
    }
}

fn check_common(st: &TaskState, d: &Draft) -> R<()> {
    use MessageKind::*;
    // R1: body shape matches kind.
    let shape_ok = matches!(
        (d.kind, &d.body),
        (Verdict, Body::Verdict(_))
            | (Decision, Body::Decision(_))
            | (
                Ask | Answer | Propose | Finding | RequestDecision | Pass,
                Body::Prose(_)
            )
    );
    if !shape_ok {
        return Err(rej(Code::UnknownKind, "body shape does not match kind"));
    }
    // R2.
    if matches!(
        d.kind,
        Propose | Finding | Verdict | RequestDecision | Decision
    ) && d.refs.is_empty()
    {
        return Err(rej(Code::RefsRequired, "refs required"));
    }
    // R3 presence.
    if matches!(d.kind, Finding | Verdict) && d.evidence.is_empty() {
        return Err(rej(Code::EvidenceRequired, "evidence required"));
    }
    // R4.
    for r in d.refs.iter().chain(d.evidence.iter()) {
        resolve_ref(st, r)?;
    }
    // R8.
    if body_len(&d.body)? > BODY_CAP {
        return Err(rej(Code::BodyTooLong, "body over 1,200 chars"));
    }
    // R9.
    for r in &d.to {
        if let Recipient::Role(role) = r {
            if !st.assigned.contains_key(role) {
                return Err(rej(Code::BadRecipient, format!("{role:?} not assigned")));
            }
        }
    }
    if d.kind == RequestDecision
        && !d
            .to
            .iter()
            .any(|r| matches!(r, Recipient::Human | Recipient::All))
    {
        return Err(rej(
            Code::HumanRequired,
            "request_decision must address human",
        ));
    }
    // R10.
    if let Some(id) = d.reply_to {
        st.message(id)
            .ok_or_else(|| rej(Code::BadReplyTo, format!("{id}")))?;
    }
    Ok(())
}

fn check_verdict(st: &TaskState, role: Role, v: &Verdict, d: &Draft) -> R<()> {
    let ok_role = matches!(
        (role, st.state),
        (Role::Tester, State::Test) | (Role::Reviewer, State::Review)
    );
    if !ok_role {
        return Err(rej(Code::RoleForbidden, "verdict role/state"));
    }
    let cand = st
        .current_candidate()
        .ok_or_else(|| rej(Code::StaleCandidate, "no current candidate"))?;
    if v.subject != cand.id {
        return Err(rej(
            Code::StaleCandidate,
            "subject is not the current candidate",
        ));
    }
    if cand.spec != st.approved_spec {
        return Err(rej(
            Code::SpecMismatch,
            "candidate spec is not the approved spec",
        ));
    }
    if !d
        .refs
        .iter()
        .any(|r| matches!(r, Ref::Art { id, .. } if *id == v.subject))
    {
        return Err(rej(Code::SubjectMissing, "refs must include the subject"));
    }
    let run = st
        .selected_run()
        .ok_or_else(|| rej(Code::RunPending, "no selected run"))?;
    if run.id != v.run {
        return Err(rej(Code::StaleRun, "run is not the selected run"));
    }
    let (status, report) = match (run.status, run.report) {
        (Some(s), Some(r)) => (s, r),
        _ => return Err(rej(Code::RunPending, "selected run has no final report")),
    };
    if !d
        .evidence
        .iter()
        .any(|r| matches!(r, Ref::Art { id, .. } if *id == report))
    {
        return Err(rej(
            Code::EvidenceUnbound,
            "evidence must cite the run's report",
        ));
    }
    match v.verdict {
        VerdictKind::Approve if status != RunStatus::Passed => {
            Err(rej(Code::NotPassed, "approve requires a passed run"))
        }
        VerdictKind::Revise if v.blocking.is_empty() => Err(rej(
            Code::BlockingRequired,
            "revise requires a blocking item",
        )),
        _ => Ok(()),
    }
}

fn post(w: &mut Work, p: &PrincipalId, role: Role, d: &Draft, now: Time) -> R<Response> {
    use MessageKind::*;
    if d.kind == Decision {
        return Err(rej(Code::DecisionForbidden, "decisions are human-only"));
    }
    if !role_may_post(role, d.kind) {
        return Err(rej(
            Code::RoleForbidden,
            format!("{role:?} may not post {:?}", d.kind),
        ));
    }
    if !state_accepts(w.st.state, d.kind) {
        return Err(rej(
            Code::WrongState,
            format!("{:?} not accepted in {:?}", d.kind, w.st.state),
        ));
    }
    let turn =
        w.st.turn
            .clone()
            .ok_or_else(|| rej(Code::TokenExpired, "no live turn"))?;
    if turn.kind == TurnKind::Consultation && !matches!(d.kind, Ask | Answer | Finding | Pass) {
        return Err(rej(Code::ConsultationOnly, "consultation turn"));
    }
    check_common(&w.st, d)?;
    if d.kind == Answer {
        let target = d
            .reply_to
            .and_then(|id| w.st.message(id))
            .ok_or_else(|| rej(Code::BadReplyTo, "answer needs reply_to"))?;
        if target.draft.kind != Ask {
            return Err(rej(Code::BadReplyTo, "answer must reply to an ask"));
        }
    }
    if role == Role::Reviewer
        && d.kind == Answer
        && !d.to.iter().all(|r| matches!(r, Recipient::Human))
    {
        return Err(rej(Code::BadRecipient, "reviewer answers only the human"));
    }
    if let Body::Verdict(v) = &d.body {
        check_verdict(&w.st, role, v, d)?;
    }
    let gate_spec = if d.kind == RequestDecision {
        check_request(&w.st, role, d)?
    } else {
        None
    };
    if d.kind == Pass {
        if let Some(target) = d.yield_to {
            check_yield(&w.st, &turn, target)?;
        }
    }
    // Effects.
    let id = w.st.next_msg_id();
    w.emit(Event::MessageAppended(Message {
        id,
        from: p.clone(),
        from_role: Some(role),
        draft: d.clone(),
    }));
    w.emit(Event::Charged {
        principal: p.clone(),
        turn: Some(turn.id),
        messages: 1,
        bytes: body_len(&d.body).unwrap_or(0) as u64,
        reads: 0,
    });
    match d.kind {
        Pass => match d.yield_to {
            Some(target) => {
                end_turn_only(w, TurnEnd::Yield);
                grant(w, target, TurnKind::Consultation, Some(role), now);
            }
            None => end_turn(w, TurnEnd::Pass, now),
        },
        RequestDecision => {
            let deadline = (gate_spec.is_none()).then(|| now.plus(w.st.request_ms));
            w.emit(Event::RequestOpened {
                msg: id,
                requester: role,
                gate: gate_spec.is_some(),
                deadline,
            });
            end_turn_only(w, TurnEnd::Request);
            if gate_spec.is_some() {
                transition(w, State::PlanGate, now);
            }
        }
        Verdict => {
            let kind = match &d.body {
                Body::Verdict(v) => v.verdict,
                _ => VerdictKind::Revise,
            };
            end_turn_only(w, TurnEnd::Verdict);
            let to = match (kind, w.st.state) {
                (VerdictKind::Approve, State::Test) => State::Review,
                (VerdictKind::Approve, _) => State::MergeGate,
                (VerdictKind::Revise, _) => State::Build,
            };
            transition(w, to, now);
        }
        _ => {}
    }
    Ok(Response::Posted(id))
}

/// Returns the spec id when this request opens the plan gate.
fn check_request(st: &TaskState, role: Role, d: &Draft) -> R<Option<ArtId>> {
    if st.pending_request.is_some() {
        return Err(rej(Code::AlreadyOpen, "a request is already open"));
    }
    if !d.gate {
        return Ok(None);
    }
    if role != Role::Planner || st.state != State::Planning {
        return Err(rej(
            Code::WrongState,
            "only the Planner opens the plan gate, in planning",
        ));
    }
    let spec = match d.refs.first() {
        Some(Ref::Art { id, .. }) => *id,
        _ => {
            return Err(rej(
                Code::RefsRequired,
                "gate request refs[0] must be the spec",
            ))
        }
    };
    let rec = resolve_kind(st, spec, ArtifactKind::Spec)?;
    match rec.payload {
        ArtifactPayload::Spec { brief } if Some(brief) == st.brief => Ok(Some(spec)),
        _ => Err(rej(
            Code::RefUnresolved,
            "spec does not reference the brief",
        )),
    }
}

fn check_yield(st: &TaskState, turn: &Turn, target: Role) -> R<()> {
    if turn.kind == TurnKind::Consultation {
        return Err(rej(Code::YieldNested, "yields do not nest"));
    }
    if !st.state.consultable(target) {
        return Err(rej(
            Code::WrongState,
            format!("{target:?} is not consultable in {:?}", st.state),
        ));
    }
    if !st.assigned.contains_key(&target) {
        return Err(rej(Code::BadRecipient, format!("{target:?} not assigned")));
    }
    Ok(())
}

// ---------------------------------------------------------------- runner

fn runner(
    w: &mut Work,
    run: RunId,
    secret: RunSecret,
    blob: BlobRef,
    payload: &ArtifactPayload,
    now: Time,
) -> R<Response> {
    let kind = payload.kind();
    if !matches!(kind, ArtifactKind::TestReport | ArtifactKind::Log) {
        return Err(rej(
            Code::KindForbidden,
            "runner writes reports and logs only",
        ));
    }
    if w.st.state != State::Test {
        return Err(rej(Code::WrongState, "runner writes only in test"));
    }
    if w.st.revoked_runs.contains(&run) {
        return Err(rej(Code::StaleRun, "run revoked"));
    }
    let sel =
        w.st.selected_run()
            .cloned()
            .ok_or_else(|| rej(Code::StaleRun, "no selected run"))?;
    if sel.id != run {
        return Err(rej(Code::StaleRun, "not the selected run"));
    }
    if sel.secret != secret {
        return Err(rej(Code::BadRunSecret, "bad run secret"));
    }
    if blob.len > MAX_ARTIFACT_BYTES {
        return Err(rej(Code::TooLarge, "artifact exceeds max_artifact_bytes"));
    }
    let dup = match kind {
        ArtifactKind::TestReport => sel.report.is_some(),
        _ => sel.log.is_some(),
    };
    if dup {
        return Err(rej(Code::DuplicateReport, "run already has this artifact"));
    }
    let mut status = None;
    if let ArtifactPayload::TestReport(r) = payload {
        if r.candidate != sel.candidate || r.run != sel.id {
            return Err(rej(
                Code::InconsistentReport,
                "report names another candidate or run",
            ));
        }
        if r.profile_digest != w.st.profile_digest {
            return Err(rej(Code::ProfileMismatch, "profile digest"));
        }
        if !r.consistent() {
            return Err(rej(
                Code::InconsistentReport,
                "status disagrees with profile results",
            ));
        }
        let log = resolve_kind(&w.st, r.log, ArtifactKind::Log)?;
        if log.run != Some(sel.id) {
            return Err(rej(Code::InconsistentReport, "log belongs to another run"));
        }
        status = Some(r.status);
    }
    let id = w.st.next_art_id();
    w.emit(Event::ArtifactStored(ArtifactRecord {
        id,
        author: PrincipalId(RUNNER.into()),
        kind,
        blob,
        payload: payload.clone(),
        spec: None,
        candidate: Some(sel.candidate),
        run: Some(sel.id),
    }));
    if let Some(status) = status {
        w.emit(Event::TestReportStored {
            candidate: sel.candidate,
            run: sel.id,
            status,
            report: id,
        });
        if status == RunStatus::Error {
            escalate(w, EscalateReason::RunError, now);
        } else {
            schedule(w, now);
        }
    }
    Ok(Response::Stored(id))
}

// ---------------------------------------------------------------- human

fn check_rev(st: &TaskState, expected: Rev) -> R<()> {
    if st.rev != expected {
        return Err(rej(
            Code::StaleRev,
            format!("card rev is {}, expected {}", st.rev.0, expected.0),
        ));
    }
    Ok(())
}

fn human(w: &mut Work, a: &HumanAction, now: Time) -> R<Response> {
    match a {
        HumanAction::Approve {
            gate: Gate::Plan,
            subject,
            ..
        } => {
            if w.st.state != State::PlanGate {
                return Err(rej(Code::WrongState, "not at plan gate"));
            }
            let (msg, spec) =
                w.st.gate_request
                    .ok_or_else(|| rej(Code::NotOpen, "no open gate request"))?;
            if spec != *subject {
                return Err(rej(Code::StaleSubject, "subject is not the requested spec"));
            }
            w.emit(Event::HumanApproval {
                gate: Gate::Plan,
                subject: *subject,
                run: None,
            });
            w.emit(Event::RequestSettled { msg, by: msg });
            transition(w, State::Build, now);
            Ok(Response::Ok)
        }
        HumanAction::Approve {
            gate: Gate::Merge,
            subject,
            run,
        } => {
            if w.st.state != State::MergeGate {
                return Err(rej(Code::WrongState, "not at merge gate"));
            }
            let cand =
                w.st.current_candidate()
                    .ok_or_else(|| rej(Code::StaleSubject, "no candidate"))?;
            if cand.id != *subject {
                return Err(rej(
                    Code::StaleSubject,
                    "subject is not the current candidate",
                ));
            }
            if cand.spec != w.st.approved_spec {
                return Err(rej(
                    Code::SpecMismatch,
                    "candidate spec is not the approved spec",
                ));
            }
            let sel =
                w.st.selected_run()
                    .ok_or_else(|| rej(Code::RunPending, "no selected run"))?;
            if Some(sel.id) != *run {
                return Err(rej(Code::StaleSubject, "run is not the selected run"));
            }
            if sel.status != Some(RunStatus::Passed) {
                return Err(rej(Code::NotPassed, "selected run is not passed"));
            }
            if w.st.verdict(Role::Reviewer, VerdictKind::Approve).is_none() {
                return Err(rej(
                    Code::NotReviewed,
                    "no Reviewer approve on this candidate and run",
                ));
            }
            w.emit(Event::HumanApproval {
                gate: Gate::Merge,
                subject: *subject,
                run: *run,
            });
            transition(w, State::Approved, now);
            Ok(Response::Ok)
        }
        HumanAction::Reject {
            expected_rev,
            target,
            reason,
        } => {
            check_rev(&w.st, *expected_rev)?;
            let from = w.st.state;
            let ok = match target {
                State::Planning => from != State::Planning,
                State::Build => matches!(from, State::MergeGate | State::Approved),
                _ => false,
            };
            if !ok {
                return Err(rej(
                    Code::WrongState,
                    format!("cannot reject from {from:?} to {target:?}"),
                ));
            }
            if let Some(c) = w.st.candidate {
                w.emit(Event::VerdictsStaled { candidate: c });
            }
            w.emit(Event::HumanRejection {
                from,
                target: *target,
                reason: reason.clone(),
            });
            transition(w, *target, now);
            Ok(Response::Ok)
        }
        HumanAction::Post(d) => human_post(w, d, now),
        HumanAction::Rerun {
            candidate,
            expected_rev,
        } => {
            check_rev(&w.st, *expected_rev)?;
            if !matches!(w.st.state, State::Test | State::Review | State::MergeGate) {
                return Err(rej(
                    Code::WrongState,
                    "rerun only in test, review or merge_gate",
                ));
            }
            if w.st.candidate != Some(*candidate) {
                return Err(rej(Code::StaleCandidate, "not the current candidate"));
            }
            if let Some(r) = w.st.selected_run().map(|r| r.id) {
                w.emit(Event::RunRevoked { run: r });
            }
            w.emit(Event::VerdictsStaled {
                candidate: *candidate,
            });
            w.emit(Event::Rerun {
                candidate: *candidate,
            });
            start_run(w, *candidate);
            if w.st.state != State::Test {
                end_turn_only(w, TurnEnd::Revoked);
                transition(w, State::Test, now);
            }
            Ok(Response::Ok)
        }
        HumanAction::MergeReceipt {
            candidate,
            revision,
        } => {
            if w.st.state != State::Approved {
                return Err(rej(Code::WrongState, "not approved"));
            }
            if w.st.merge_approval.map(|(c, _)| c) != Some(*candidate) {
                return Err(rej(Code::StaleSubject, "receipt names another candidate"));
            }
            w.emit(Event::MergeReceipt {
                candidate: *candidate,
                revision: revision.clone(),
            });
            transition(w, State::Closed, now);
            Ok(Response::Ok)
        }
        HumanAction::Resume {
            target,
            expected_rev,
        } => {
            check_rev(&w.st, *expected_rev)?;
            if w.st.state != State::Escalated {
                return Err(rej(Code::WrongState, "not escalated"));
            }
            check_resume(&w.st, *target)?;
            if let (State::Planning, Some(c)) = (target, w.st.candidate) {
                w.emit(Event::VerdictsStaled { candidate: c });
            }
            w.emit(Event::Resumed { target: *target });
            transition(w, *target, now);
            Ok(Response::Ok)
        }
        HumanAction::BudgetExtended { field, limit } => {
            w.emit(Event::BudgetExtended {
                field: *field,
                limit: *limit,
            });
            Ok(Response::Ok)
        }
        HumanAction::Cancel {
            expected_rev,
            reason,
        } => {
            check_rev(&w.st, *expected_rev)?;
            w.emit(Event::Cancelled {
                reason: reason.clone(),
            });
            transition(w, State::Cancelled, now);
            Ok(Response::Ok)
        }
    }
}

fn exhausted_fields(st: &TaskState) -> Vec<BudgetField> {
    let mut out = Vec::new();
    if st.spent.messages > st.budget.messages {
        out.push(BudgetField::Messages);
    }
    if st.spent.bytes > st.budget.bytes {
        out.push(BudgetField::Bytes);
    }
    if st.spent.reads > st.budget.reads {
        out.push(BudgetField::Reads);
    }
    if st.spent.turns > st.budget.turns {
        out.push(BudgetField::Turns);
    }
    if st.iteration > st.budget.iterations {
        out.push(BudgetField::Iterations);
    }
    out
}

fn check_resume(st: &TaskState, target: State) -> R<()> {
    if !exhausted_fields(st).is_empty() {
        return Err(rej(
            Code::BudgetExhausted,
            "extend the exhausted budget before resuming",
        ));
    }
    let from = st.escalated_from;
    let allowed = matches!(
        target,
        State::Planning | State::Build | State::Test | State::Review
    ) || Some(target) == from;
    if !allowed || target.terminal() || target == State::Escalated {
        return Err(rej(
            Code::WrongState,
            format!("cannot resume to {target:?}"),
        ));
    }
    let cand_ok = st
        .current_candidate()
        .map(|c| c.spec == st.approved_spec)
        .unwrap_or(false);
    match target {
        State::Planning => Ok(()),
        State::Build => st
            .approved_spec
            .map(|_| ())
            .ok_or_else(|| rej(Code::NoApprovedSpec, "build needs an approved spec")),
        State::Test => cand_ok.then_some(()).ok_or_else(|| {
            rej(
                Code::StaleCandidate,
                "test needs a current candidate on the approved spec",
            )
        }),
        State::Review => {
            if !cand_ok {
                return Err(rej(
                    Code::StaleCandidate,
                    "review needs a current candidate",
                ));
            }
            st.verdict(Role::Tester, VerdictKind::Approve)
                .map(|_| ())
                .ok_or_else(|| rej(Code::NotReviewed, "review needs a Tester approve"))
        }
        State::MergeGate => st
            .verdict(Role::Reviewer, VerdictKind::Approve)
            .map(|_| ())
            .ok_or_else(|| rej(Code::NotReviewed, "merge_gate needs a Reviewer approve")),
        State::Approved => st
            .merge_approval
            .map(|_| ())
            .ok_or_else(|| rej(Code::NotReviewed, "approved needs the prior human approval")),
        State::PlanGate => st
            .gate_request
            .map(|_| ())
            .ok_or_else(|| rej(Code::NotOpen, "plan_gate needs an open gate request")),
        _ => Err(rej(Code::WrongState, "unreachable target")),
    }
}

fn human_post(w: &mut Work, d: &Draft, now: Time) -> R<Response> {
    use MessageKind::*;
    if !matches!(d.kind, Ask | Answer | Decision) {
        return Err(rej(
            Code::RoleForbidden,
            "human posts ask, answer or decision",
        ));
    }
    check_common(&w.st, d)?;
    let pending = w.st.pending_request.clone();
    let mut settles: Option<MsgId> = None;
    match (&d.kind, &d.body) {
        (Answer, _) => {
            let target = d
                .reply_to
                .and_then(|id| w.st.message(id).cloned())
                .ok_or_else(|| rej(Code::BadReplyTo, "answer needs reply_to"))?;
            let open_request = pending
                .as_ref()
                .map(|p| p.msg == target.id && !p.gate)
                .unwrap_or(false);
            if target.draft.kind != Ask && !open_request {
                return Err(rej(
                    Code::BadReplyTo,
                    "answer must reply to an ask or an open informational request",
                ));
            }
            if open_request {
                settles = Some(target.id);
            }
        }
        (Decision, Body::Decision(dec)) => {
            let subj =
                w.st.message(dec.subject)
                    .ok_or_else(|| rej(Code::RefUnresolved, "decision subject"))?;
            if !matches!(subj.draft.kind, RequestDecision | Propose) {
                return Err(rej(
                    Code::WrongKind,
                    "decision subject must be a request or proposal",
                ));
            }
            if pending
                .as_ref()
                .map(|p| p.msg == dec.subject && !p.gate)
                .unwrap_or(false)
            {
                settles = Some(dec.subject);
            }
        }
        _ => {}
    }
    let id = w.st.next_msg_id();
    w.emit(Event::MessageAppended(Message {
        id,
        from: w.st.human.clone(),
        from_role: None,
        draft: d.clone(),
    }));
    if let Some(req) = settles {
        w.emit(Event::RequestSettled { msg: req, by: id });
        if let Some(p) = pending {
            if w.st.turn.is_none() {
                let kind = if w.st.state.default_role() == Some(p.requester) {
                    TurnKind::Default
                } else {
                    TurnKind::Consultation
                };
                if kind == TurnKind::Default
                    || w.st.state.consultable(p.requester)
                    || w.st.state.default_role().is_none()
                {
                    grant(w, p.requester, kind, None, now);
                }
            }
        }
    } else if d.kind == Ask && w.st.turn.is_none() {
        if let Some(Recipient::Role(role)) = d.to.first() {
            grant(w, *role, TurnKind::Consultation, None, now);
        }
    }
    Ok(Response::Posted(id))
}

// ---------------------------------------------------------------- turns and lifecycle

fn grant(w: &mut Work, role: Role, kind: TurnKind, return_to: Option<Role>, now: Time) {
    let Some(p) = w.st.assigned.get(&role).cloned() else {
        return;
    };
    if w.st.turn.is_some() {
        return;
    }
    let turn = TurnId(w.st.next_turn);
    let token = w.st.token_for(&p, turn);
    w.emit(Event::TurnGranted {
        role,
        turn,
        kind,
        token,
        deadline: now.plus(w.st.turn_ms),
        return_to,
    });
}

/// The one scheduling check.
fn schedule(w: &mut Work, now: Time) {
    if w.st.turn.is_some() || w.st.pending_request.is_some() || !w.st.opened {
        return;
    }
    let Some(role) = w.st.state.default_role() else {
        return;
    };
    if w.st.state == State::Test && w.st.selected_run().and_then(|r| r.status).is_none() {
        return;
    }
    grant(w, role, TurnKind::Default, None, now);
}

fn end_turn_only(w: &mut Work, cause: TurnEnd) -> Option<Turn> {
    let t = w.st.turn.clone()?;
    w.emit(Event::TurnEnded { turn: t.id, cause });
    Some(t)
}

/// End the live turn and run the consultation return or the schedule.
fn end_turn(w: &mut Work, cause: TurnEnd, now: Time) {
    let Some(t) = end_turn_only(w, cause) else {
        return;
    };
    let unchanged = t.state_at == w.st.state
        && t.candidate_at == w.st.candidate
        && t.run_at == w.st.run.as_ref().map(|r| r.id);
    match (t.kind, t.return_to) {
        (TurnKind::Consultation, Some(back)) if unchanged && w.st.pending_request.is_none() => {
            grant(w, back, TurnKind::Default, None, now)
        }
        _ => schedule(w, now),
    }
}

fn start_run(w: &mut Work, candidate: ArtId) {
    let run = RunId(w.st.next_run);
    let secret = w.st.secret_for(run);
    w.emit(Event::RunStarted {
        candidate,
        run,
        secret,
    });
}

fn transition(w: &mut Work, to: State, now: Time) {
    let from = w.st.state;
    if from == to {
        return;
    }
    end_turn_only(w, TurnEnd::Revoked);
    w.emit(Event::StateChanged { from, to });
    entry(w, now);
}

fn entry(w: &mut Work, now: Time) {
    match w.st.state {
        State::Test => {
            let needs_run = match w.st.selected_run() {
                None => true,
                Some(r) => r.status == Some(RunStatus::Error),
            };
            if let (true, Some(c)) = (needs_run, w.st.candidate) {
                start_run(w, c);
            }
            schedule(w, now);
        }
        State::Planning | State::Build | State::Review => schedule(w, now),
        _ => {}
    }
}

fn escalate(w: &mut Work, reason: EscalateReason, now: Time) {
    if w.st.state.terminal() || w.st.state == State::Escalated {
        if let EscalateReason::Budget(_) = reason {
            w.emit(Event::Escalated {
                reason,
                from: w.st.state,
            });
        }
        return;
    }
    w.emit(Event::Escalated {
        reason,
        from: w.st.state,
    });
    transition(w, State::Escalated, now);
}

/// Generated events: fire each newly exceeded budget once (E3).
fn evaluate_budgets(w: &mut Work, now: Time) {
    for f in exhausted_fields(&w.st) {
        if !w.st.exhausted.contains(&f) {
            escalate(w, EscalateReason::Budget(f), now);
        }
    }
}

/// A4: deadlines fire before the command is considered.
fn fire_deadlines(w: &mut Work, now: Time) {
    if w.st
        .turn
        .as_ref()
        .map(|t| t.deadline <= now)
        .unwrap_or(false)
    {
        end_turn(w, TurnEnd::Deadline, now);
    }
    if let Some(d) = w.st.pending_request.as_ref().and_then(|p| p.deadline) {
        if d <= now && !w.st.state.terminal() && w.st.state != State::Escalated {
            escalate(w, EscalateReason::RequestDeadline, now);
        }
    }
}
