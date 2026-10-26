//! The processing model: validation in rule order, effects, transitions, entry
//! actions and generated events. `handle` is the only entry point and is pure.

use crate::coedit::{self, Doc, SpecEdit};
use crate::command::*;
use crate::event::*;
use crate::ids::*;
use crate::policy::SchedulingPolicy;
use crate::record::*;
use crate::role::*;
use crate::shadow::{Shadow, ShadowKind};
use crate::state::*;
use crate::wake::{pick_from, wake_set, WAKE_RULES};

pub const MAX_ARTIFACT_BYTES: u64 = 2_000_000;
const RUNNER: &str = "runner";

/// Result of handling one command: the events to append and the response to return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handled {
    pub events: Vec<Event>,
    pub response: Response,
    /// What an event-driven policy would have done beside each real grant.
    /// Not logged; empty unless the task has one live turn.
    pub shadow: Vec<Shadow>,
}

type R<T> = Result<T, Rejection>;

fn rej(code: Code, detail: impl Into<String>) -> Rejection {
    Rejection::new(code, detail)
}

/// Working copy: applies each emitted event immediately so later rules see it.
struct Work {
    st: TaskState,
    events: Vec<Event>,
    shadow: Vec<Shadow>,
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
        shadow: Vec::new(),
    };
    if w.st.opened {
        fire_deadlines(&mut w, now);
    }
    match dispatch(&mut w, cmd, now) {
        Ok(Flow::Replay(resp)) => Handled {
            events: w.events,
            response: resp,
            shadow: w.shadow,
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
                shadow: w.shadow,
            }
        }
        // A command refused before the task exists writes nothing: there is no
        // log to record against, and a refused `open` (a bad roster) must leave
        // the directory free for a corrected one.
        Err(r) if !w.st.opened => Handled {
            events: Vec::new(),
            response: Response::Rejected(r),
            shadow: Vec::new(),
        },
        Err(r) => {
            let principal = principal_of(&w.st, cmd);
            w.emit(Event::Rejected {
                principal,
                code: r.code,
            });
            Handled {
                events: w.events,
                response: Response::Rejected(r),
                shadow: w.shadow,
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
            if w.st.roster.get(role).is_none() {
                return Err(rej(
                    Code::UnknownRole,
                    format!("role {role} is not in the roster"),
                ));
            }
            w.emit(Event::Assigned {
                role: role.clone(),
                principal: principal.clone(),
            });
            // No grant here: a turn needs a host ready to launch its harness,
            // and assignment happens before any moderator runs. The first
            // `Tick` grants it.
            Ok(Flow::Done(Response::Ok))
        }
        Command::Tick => {
            require_open(&w.st)?;
            schedule(w, now);
            Ok(Flow::Done(Response::Ok))
        }
        Command::AbortTurn { turn } => {
            require_open(&w.st)?;
            let ids = match turn {
                Some(id) => vec![*id],
                None => live_ids(&w.st),
            };
            for id in ids {
                end_turn(w, id, TurnEnd::Aborted, now);
            }
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
                check_token(&w.st, principal, &role, *token, now)?;
            }
            agent(w, principal, &role, action, now).map(Flow::Done)
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
    role: &RoleId,
    token: Option<Token>,
    now: Time,
) -> R<()> {
    let token = token.ok_or_else(|| rej(Code::BadToken, "no token"))?;
    let Some(turn) = st.turn_of(role) else {
        // Nobody live: the token expired. Others live: this one is stale.
        return Err(if st.turns.is_empty() {
            rej(Code::TokenExpired, "no live turn")
        } else {
            rej(Code::StaleTurn, "role holds no live turn")
        });
    };
    if st.token_for(p, turn.id) != token {
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
    let roster = Roster::new(o.roster.clone()).map_err(|e| rej(Code::BadRoster, e.to_string()))?;
    if o.max_live_turns == 0 || o.max_live_turns as usize > roster.roles().len() {
        return Err(rej(
            Code::BadLiveTurns,
            format!(
                "max_live_turns {} is not 1 to the roster size {}",
                o.max_live_turns,
                roster.roles().len()
            ),
        ));
    }
    if o.policy == SchedulingPolicy::EventDriven && o.max_live_turns != 1 {
        return Err(rej(
            Code::BadLiveTurns,
            format!(
                "event-driven scheduling needs max_live_turns 1, got {}",
                o.max_live_turns
            ),
        ));
    }
    w.emit(Event::TaskOpened {
        schema: schema_for(o.max_live_turns, o.policy),
        roster: o.roster.clone(),
        task: o.task.clone(),
        repo: o.repo.clone(),
        profile_digest: o.profile_digest,
        budget: o.budget,
        turn_ms: o.turn_ms,
        request_ms: o.request_ms,
        max_live_turns: o.max_live_turns,
        human: o.human.clone(),
        policy: o.policy,
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

/// The capability record for a rostered role.
fn role_data<'a>(st: &'a TaskState, id: &RoleId) -> R<&'a Role> {
    st.roster
        .get(id)
        .ok_or_else(|| rej(Code::NotAssigned, format!("role {id} is not in the roster")))
}

fn agent(
    w: &mut Work,
    p: &PrincipalId,
    role: &RoleId,
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

/// Does the role's `read_messages` allow it to see `m`?
pub(crate) fn visible(st: &TaskState, role: &Role, m: &Message) -> bool {
    if role.read_messages == ReadScope::All {
        return true;
    }
    let from_human = m.from == st.human;
    let to_me = m.draft.to.iter().any(|r| match r {
        Recipient::Role(id) => *id == role.id,
        Recipient::All => true,
        Recipient::Human => false,
    });
    let mine = |id: MsgId| {
        st.message(id)
            .map(|x| x.from_role.as_ref() == Some(&role.id))
            .unwrap_or(false)
    };
    let answers_mine = m.draft.reply_to.map(mine).unwrap_or(false);
    let decides_mine = matches!(&m.draft.body, Body::Decision(d) if mine(d.subject));
    from_human && (to_me || answers_mine || decides_mine)
}

/// R11 per-turn read budget. Exceeding ends the turn and rejects.
fn charge_read(w: &mut Work, p: &PrincipalId, role: &RoleId, now: Time) -> R<()> {
    let (turn, reads) =
        w.st.turn_of(role)
            .map(|t| (t.id, t.reads))
            .ok_or_else(|| rej(Code::TokenExpired, "no live turn"))?;
    if reads + 1 > w.st.budget.reads_per_turn {
        end_turn(w, turn, TurnEnd::Budget, now);
        return Err(rej(Code::TurnBudgetExhausted, "reads_per_turn exceeded"));
    }
    w.emit(Event::Charged {
        principal: p.clone(),
        turn: Some(turn),
        messages: 0,
        bytes: 0,
        reads: 1,
    });
    Ok(())
}

fn read(
    w: &mut Work,
    p: &PrincipalId,
    role: &RoleId,
    after: Option<MsgId>,
    limit: usize,
    now: Time,
) -> R<Response> {
    charge_read(w, p, role, now)?;
    let reader = role_data(&w.st, role)?.clone();
    let limit = limit.clamp(1, READ_PAGE);
    let start = after.map(|a| a.0 as usize).unwrap_or(0);
    let mut out: Vec<Message> = Vec::new();
    let mut more = false;
    for m in
        w.st.messages
            .iter()
            .skip(start)
            .filter(|m| visible(&w.st, &reader, m))
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

fn get_artifact(w: &mut Work, p: &PrincipalId, role: &RoleId, id: ArtId, now: Time) -> R<Response> {
    let rec =
        w.st.artifacts
            .get(&id)
            .cloned()
            .ok_or_else(|| rej(Code::RefUnresolved, format!("{id}")))?;
    let reader = role_data(&w.st, role)?;
    if !reader.holds(Cap::Read(rec.kind)) {
        return Err(rej(Code::RoleForbidden, reader.lacks(Cap::Read(rec.kind))));
    }
    let turn =
        w.st.turn_of(role)
            .map(|t| (t.id, t.read_arts.contains(&id)));
    match turn {
        Some((_, true)) => {}
        Some((tid, false)) => {
            charge_read(w, p, role, now)?;
            w.emit(Event::ReadDeduped { turn: tid, art: id });
        }
        None => return Err(rej(Code::TokenExpired, "no live turn")),
    }
    Ok(Response::Artifact(rec))
}

/// The state each agent-writable artifact kind is stored in (R16).
fn write_state(kind: ArtifactKind) -> Option<State> {
    match kind {
        ArtifactKind::Spec => Some(State::Planning),
        ArtifactKind::Diff | ArtifactKind::Candidate => Some(State::Build),
        ArtifactKind::Brief | ArtifactKind::TestReport | ArtifactKind::Log => None,
    }
}

/// R16 gating table for agent writes: the role's `write` capability, then the state.
fn gate_artifact(st: &TaskState, role: &Role, kind: ArtifactKind) -> R<()> {
    let Some(state) = write_state(kind) else {
        return Err(rej(Code::KindForbidden, "agents cannot write this kind"));
    };
    if !role.holds(Cap::Write(kind)) {
        return Err(rej(Code::KindForbidden, role.lacks(Cap::Write(kind))));
    }
    if st.state != state {
        return Err(rej(Code::WrongState, "kind not writable in this state"));
    }
    if st.turn_of(&role.id).map(|t| t.kind) == Some(TurnKind::Consultation) {
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
    role: &RoleId,
    blob: BlobRef,
    payload: &ArtifactPayload,
    now: Time,
) -> R<Response> {
    let kind = payload.kind();
    gate_artifact(&w.st, role_data(&w.st, role)?, kind)?;
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
        ArtifactPayload::SpecEdit(e) => check_edit(&w.st, e)?,
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
    charge_slice(w, role, 0, blob.len, now)?;
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
        turn: w.st.turn_of(role).map(|t| t.id),
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
        // End the turn without scheduling: the task leaves `build` on the
        // next line, and scheduling here would grant a Coder turn only for
        // the transition to revoke it (one turn of budget, a skipped id).
        // `transition` runs the Test entry, which schedules once.
        if let Some(t) = w.st.turn_of(role).map(|t| t.id) {
            end_turn_only(w, t, TurnEnd::Candidate);
        }
        transition(w, State::Test, now);
    }
    Ok(Response::Stored(id))
}

/// A co-edited spec edit: it references the brief, is well formed against
/// its base, and is anchored on no line edited since that base. Its ops then
/// apply to the head, which may be later than the base (step 4).
fn check_edit(st: &TaskState, e: &SpecEdit) -> R<()> {
    if Some(e.brief) != st.brief {
        return Err(rej(
            Code::RefUnresolved,
            "spec must reference the task brief",
        ));
    }
    let Some(base) = e.base else {
        return coedit::validate(&Doc::default(), &e.ops);
    };
    resolve_kind(st, base, ArtifactKind::Spec)?;
    let (Some(doc), Some((head, since))) = (st.doc(base), st.since(base)) else {
        return Err(rej(
            Code::WrongKind,
            format!("{base} is a snapshot spec, not a co-edited one"),
        ));
    };
    coedit::validate(doc, &e.ops)?;
    let clash = coedit::overlap(&e.ops, &since);
    if clash.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = clash
        .iter()
        .map(|a| a.map_or("the start".into(), |l| format!("{}#{}", l.art, l.n)))
        .collect();
    Err(rej(
        Code::StaleBase,
        format!(
            "lines edited between {base} and the head {head}: {}",
            names.join(", ")
        ),
    ))
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

pub(crate) fn state_accepts(state: State, kind: MessageKind) -> bool {
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
                return Err(rej(Code::BadRecipient, format!("{role} not assigned")));
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

/// The stage a verdict is judged at, if `role` may judge it (R6).
fn verdict_stage(state: State, role: &Role) -> R<Stage> {
    let stage = Stage::of(state).ok_or_else(|| {
        rej(
            Code::RoleForbidden,
            "verdicts are judged in test and review only",
        )
    })?;
    if !role.holds(Cap::Verdict(stage)) {
        return Err(rej(Code::RoleForbidden, role.lacks(Cap::Verdict(stage))));
    }
    Ok(stage)
}

/// May `role` post a `kind` message in `state`? The one admission rule shared
/// by `post` and by `wake_set`'s `can_act`, so scheduling and enforcement
/// cannot drift: the post capability, the by-state table, and for a verdict
/// the stage capability. Turn, content and budget checks are not part of it.
pub(crate) fn admits(state: State, role: &Role, kind: MessageKind) -> R<()> {
    if !role.holds(Cap::Post(kind)) {
        return Err(rej(Code::RoleForbidden, role.lacks(Cap::Post(kind))));
    }
    if !state_accepts(state, kind) {
        return Err(rej(
            Code::WrongState,
            format!("{kind:?} not accepted in {state:?}"),
        ));
    }
    if kind == MessageKind::Verdict {
        verdict_stage(state, role)?;
    }
    Ok(())
}

fn check_verdict(st: &TaskState, role: &Role, v: &Verdict, d: &Draft) -> R<()> {
    verdict_stage(st.state, role)?;
    if let Some(n) = st
        .turn_of(&role.id)
        .map(|t| t.unseen.len())
        .filter(|n| *n > 0)
    {
        return Err(rej(
            Code::StaleSubject,
            format!(
                "{n} item(s) appended in this state have not been read by this turn; read them \
                 before judging (reads_per_turn is {}; a read returns at most {READ_PAGE} \
                 messages, an artifact costs one read)",
                st.budget.reads_per_turn
            ),
        ));
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

fn post(w: &mut Work, p: &PrincipalId, role_id: &RoleId, d: &Draft, now: Time) -> R<Response> {
    use MessageKind::*;
    if d.kind == Decision {
        return Err(rej(Code::DecisionForbidden, "decisions are human-only"));
    }
    let role = role_data(&w.st, role_id)?.clone();
    admits(w.st.state, &role, d.kind)?;
    let turn =
        w.st.turn_of(role_id)
            .cloned()
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
    if role.read_messages == ReadScope::Scoped
        && d.kind == Answer
        && !d.to.iter().all(|r| matches!(r, Recipient::Human))
    {
        return Err(rej(
            Code::BadRecipient,
            format!(
                "role {} reads scoped messages and answers only the human",
                role.id
            ),
        ));
    }
    if let Body::Verdict(v) = &d.body {
        check_verdict(&w.st, &role, v, d)?;
    }
    let gate_spec = if d.kind == RequestDecision {
        check_request(&w.st, &role, d)?
    } else {
        None
    };
    if d.kind == Pass {
        if let Some(target) = &d.yield_to {
            check_yield(&w.st, &turn, &role, target)?;
        }
    }
    charge_slice(w, role_id, 1, body_len(&d.body).unwrap_or(0) as u64, now)?;
    // Effects.
    let id = w.st.next_msg_id();
    w.emit(Event::MessageAppended(Message {
        id,
        from: p.clone(),
        from_role: Some(role.id.clone()),
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
        Pass => match &d.yield_to {
            Some(target) => {
                end_turn_only(w, turn.id, TurnEnd::Yield);
                grant_explicit(
                    w,
                    target,
                    TurnKind::Consultation,
                    Some(role.id.clone()),
                    now,
                );
            }
            None => end_turn(w, turn.id, TurnEnd::Pass, now),
        },
        RequestDecision => {
            let deadline = (gate_spec.is_none()).then(|| now.plus(w.st.request_ms));
            w.emit(Event::RequestOpened {
                msg: id,
                requester: role.id.clone(),
                gate: gate_spec.is_some(),
                deadline,
            });
            end_turn_only(w, turn.id, TurnEnd::Request);
            if gate_spec.is_some() {
                transition(w, State::PlanGate, now);
            }
        }
        Verdict => {
            let kind = match &d.body {
                Body::Verdict(v) => v.verdict,
                _ => VerdictKind::Revise,
            };
            // T11: only a `final_decision` role's approve opens the merge gate.
            // Another judge's approve at `review` is recorded and moves nothing,
            // so the finalizer still gets its review (R17 needs its approve and
            // no one could schedule the gate).
            let to = match (kind, w.st.state) {
                (VerdictKind::Approve, State::Test) => Some(State::Review),
                (VerdictKind::Approve, State::Review) if !w.st.roster.is_final(role_id) => None,
                (VerdictKind::Approve, _) => Some(State::MergeGate),
                (VerdictKind::Revise, _) => Some(State::Build),
            };
            match to {
                Some(to) => {
                    end_turn_only(w, turn.id, TurnEnd::Verdict);
                    transition(w, to, now);
                }
                None => end_turn(w, turn.id, TurnEnd::Verdict, now),
            }
        }
        _ => {}
    }
    Ok(Response::Posted(id))
}

/// Returns the spec id when this request opens the plan gate.
fn check_request(st: &TaskState, role: &Role, d: &Draft) -> R<Option<ArtId>> {
    if st.pending_request.is_some() {
        return Err(rej(Code::AlreadyOpen, "a request is already open"));
    }
    if !d.gate {
        return Ok(None);
    }
    if !role.holds(Cap::Write(ArtifactKind::Spec)) || st.state != State::Planning {
        return Err(rej(
            Code::WrongState,
            format!(
                "only a role with {} opens the plan gate, in planning",
                Cap::Write(ArtifactKind::Spec)
            ),
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
        ArtifactPayload::SpecEdit(ref e) if Some(e.brief) == st.brief => {
            check_head(st, spec)?;
            Ok(Some(spec))
        }
        _ => Err(rej(
            Code::RefUnresolved,
            "spec does not reference the brief",
        )),
    }
}

/// A co-edited version binds the plan gate only as the head of its spec:
/// approving an older one would drop every edit accepted after it (2b, d).
fn check_head(st: &TaskState, version: ArtId) -> R<()> {
    match st.head(version) {
        Some(head) if head == version => Ok(()),
        Some(head) => Err(rej(
            Code::StaleSubject,
            format!("{version} is not the head of its spec; the head is {head}"),
        )),
        None => Err(rej(
            Code::RefUnresolved,
            format!("{version} is not an indexed co-edited version"),
        )),
    }
}

/// Derived yield rule (spec, Turns): the holder reads every message, and the
/// target is assigned, consultable and not the state's default holder.
fn check_yield(st: &TaskState, turn: &Turn, holder: &Role, target: &RoleId) -> R<()> {
    if turn.kind == TurnKind::Consultation {
        return Err(rej(Code::YieldNested, "yields do not nest"));
    }
    if holder.read_messages == ReadScope::Scoped {
        return Err(rej(
            Code::WrongState,
            format!(
                "role {} reads scoped messages and cannot read an answer, so it may not yield",
                holder.id
            ),
        ));
    }
    if st.roster.get(target).is_none() {
        return Err(rej(
            Code::BadRecipient,
            format!("{target} is not in the roster"),
        ));
    }
    if !st.roster.yieldable_in(st.state, target) {
        return Err(rej(
            Code::WrongState,
            format!("{target} is not consultable in {:?}", st.state),
        ));
    }
    if !st.assigned.contains_key(target) {
        return Err(rej(Code::BadRecipient, format!("{target} not assigned")));
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
    if w.st.secret_for(run) != secret {
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
        HumanAction::Fenced {
            expected_rev,
            action,
        } => {
            if w.st.rev != *expected_rev {
                return Err(rej(Code::StaleRev, "human view changed"));
            }
            if matches!(**action, HumanAction::Fenced { .. }) {
                return Err(rej(Code::Forbidden, "nested fence"));
            }
            human(w, action, now)
        }
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
            if !w.st.final_approved() {
                return Err(rej(
                    Code::NotReviewed,
                    "no approve from a final_decision role on this candidate and run",
                ));
            }
            if w.st.final_blocked() {
                return Err(rej(
                    Code::NotReviewed,
                    "a final_decision role asked for revision on this candidate and run",
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
            st.verdict_at(Stage::Test, VerdictKind::Approve)
                .map(|_| ())
                .ok_or_else(|| rej(Code::NotReviewed, "review needs an approve at stage test"))
        }
        State::MergeGate if st.final_approved() && !st.final_blocked() => Ok(()),
        State::MergeGate => Err(rej(
            Code::NotReviewed,
            "merge_gate needs an approve from a final_decision role and no revise from one",
        )),
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
            let holder = w.st.roster.default_holder(w.st.state).cloned();
            let kind = if holder.as_ref() == Some(&p.requester) {
                TurnKind::Default
            } else {
                TurnKind::Consultation
            };
            if kind == TurnKind::Default
                || w.st.roster.yieldable_in(w.st.state, &p.requester)
                || holder.is_none()
            {
                grant_explicit(w, &p.requester, kind, None, now);
            }
        }
    } else if d.kind == Ask {
        if let Some(Recipient::Role(role)) = d.to.first() {
            grant_explicit(w, role, TurnKind::Consultation, None, now);
        }
    }
    Ok(Response::Posted(id))
}

// ---------------------------------------------------------------- turns and lifecycle

/// Grant `role` a turn unless it already holds one or `max_live_turns` are
/// live. A `final_decision` role is serialized: it is granted only when no turn
/// is live, and while it holds one nobody else is granted, so a finalizer's
/// verdict is never racing another live turn and cannot silently revoke a
/// second finalizer's veto.
///
/// This is the only place a turn is minted, so it is also the only place
/// `turns` is charged and checked: a crossing escalates here, whichever path
/// reached it (a refused write, a deadline, a command refused afterwards).
fn grant(w: &mut Work, role: &RoleId, kind: TurnKind, return_to: Option<RoleId>, now: Time) {
    let final_role = w.st.roster.is_final(role);
    let final_live = w.st.turns.iter().any(|t| w.st.roster.is_final(&t.role));
    if !w.st.assigned.contains_key(role)
        || w.st.turn_of(role).is_some()
        || w.st.turns.len() >= w.st.max_live_turns as usize
        || (final_role && !w.st.turns.is_empty())
        || final_live
    {
        return;
    }
    let turn = TurnId(w.st.next_turn);
    let slice = w.st.next_slice(final_role);
    w.emit(Event::TurnGranted {
        role: role.clone(),
        turn,
        kind,
        deadline: now.plus(w.st.turn_ms),
        return_to,
        slice,
    });
    let turns = BudgetField::Turns;
    if w.st.spent.turns > w.st.budget.turns && !w.st.exhausted.contains(&turns) {
        escalate(w, EscalateReason::Budget(turns), now);
    }
}

/// Shadow mode runs only where `wake_set` is defined: a turn-based task with
/// one live turn.
fn shadowing(st: &TaskState) -> bool {
    st.policy == SchedulingPolicy::TurnBased && st.max_live_turns == 1
}

/// `grant` for a forced grantee (yield, human `ask`, settlement, consultation
/// return). It bypasses `pick_next`, so shadow mode records it as explicit and
/// never compares it.
fn grant_explicit(
    w: &mut Work,
    role: &RoleId,
    kind: TurnKind,
    return_to: Option<RoleId>,
    now: Time,
) {
    // Read before the grant: it can cross the turn budget and escalate, which
    // would relabel the grant with the state it escalated to.
    let (state, rev0, from) = (w.st.state, w.st.rev, w.events.len());
    grant(w, role, kind, return_to, now);
    let Some((_, rev)) = grants_since(w, from, rev0).into_iter().next() else {
        return;
    };
    if shadowing(&w.st) {
        let granted = role.clone();
        w.shadow.push(Shadow {
            rev,
            rules: WAKE_RULES,
            state,
            kind: ShadowKind::Explicit { granted, kind },
        });
    }
}

/// The grants made by the events emitted since `from`, each with the rev of its
/// own `turn_granted` event (`rev0` is the rev before `from`). Read from the
/// events, not the live turns: a grant that crosses the turn budget escalates
/// and ends its turn in the same command.
fn grants_since(w: &Work, from: usize, rev0: Rev) -> Vec<(RoleId, Rev)> {
    let new = w.events.get(from..).unwrap_or_default();
    new.iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::TurnGranted { role, .. } => Some((role.clone(), Rev(rev0.0 + i as u64 + 1))),
            _ => None,
        })
        .collect()
}

/// The one scheduling check. Fills the free turn slots with the roles that
/// serve the state, in `turn_order`; at `max_live_turns == 1` that is the
/// default holder alone. An unassigned default holder blocks the rest.
fn schedule(w: &mut Work, now: Time) {
    if w.st.pending_request.is_some() || !w.st.opened {
        return;
    }
    let serving = pick_next(&w.st);
    let Some(first) = serving.first() else {
        return;
    };
    if !w.st.assigned.contains_key(first) {
        return;
    }
    if w.st.state == State::Test && w.st.selected_run().and_then(|r| r.status).is_none() {
        return;
    }
    // Taken before any grant: `wake_set` is defined on a board with no live turn.
    let would = shadowing(&w.st).then(|| wake_set(&w.st));
    let (rev0, from) = (w.st.rev, w.events.len());
    let state = w.st.state;
    for role in &serving {
        grant(w, role, TurnKind::Default, None, now);
        if w.st.state != state {
            break; // a grant escalated the task; the rest of the list is stale
        }
    }
    let Some(would) = would else {
        return;
    };
    let grants = grants_since(w, from, rev0);
    if let Some(&(_, rev)) = grants.first() {
        let granted = grants.into_iter().map(|(r, _)| r).collect();
        w.shadow.push(Shadow {
            rev,
            rules: WAKE_RULES,
            state,
            kind: ShadowKind::Auto { granted, would },
        });
    }
}

/// Who may take the next turns, under the task's policy. `TurnBased`: the roles
/// that serve the state, in `turn_order`. `EventDriven`: the first selectable
/// role `wake_set` names, else the `TurnBased` list, so a quiet board still
/// moves. Not capped here: `grant` admits at most `max_live_turns` of it, the
/// one place the cap lives.
pub(crate) fn pick_next(st: &TaskState) -> Vec<RoleId> {
    let serving = || st.roster.serving(st.state).into_iter().cloned().collect();
    match st.policy {
        SchedulingPolicy::TurnBased => serving(),
        SchedulingPolicy::EventDriven => pick_from(&wake_set(st), st.max_live_turns)
            .map_or_else(serving, |(role, _)| vec![role.clone()]),
    }
}

/// Ids of the live turns, in grant order.
fn live_ids(st: &TaskState) -> Vec<TurnId> {
    st.turns.iter().map(|t| t.id).collect()
}

/// R11 slice: a write past the turn's reserved share ends the turn. Only
/// tasks with `max_live_turns > 1` carry a slice.
fn charge_slice(w: &mut Work, role: &RoleId, messages: u64, bytes: u64, now: Time) -> R<()> {
    let Some((id, s, used)) =
        w.st.turn_of(role)
            .and_then(|t| t.slice.map(|s| (t.id, s, (t.messages, t.bytes))))
    else {
        return Ok(());
    };
    if used.0 + messages > s.messages || used.1 + bytes > s.bytes {
        end_turn(w, id, TurnEnd::Budget, now);
        return Err(rej(
            Code::TurnBudgetExhausted,
            "turn slice of messages or bytes exceeded",
        ));
    }
    Ok(())
}

fn end_turn_only(w: &mut Work, turn: TurnId, cause: TurnEnd) -> Option<Turn> {
    let t = w.st.turns.iter().find(|t| t.id == turn).cloned()?;
    w.emit(Event::TurnEnded { turn: t.id, cause });
    Some(t)
}

/// End `turn` and run the consultation return or the schedule.
fn end_turn(w: &mut Work, turn: TurnId, cause: TurnEnd, now: Time) {
    let Some(t) = end_turn_only(w, turn, cause) else {
        return;
    };
    let unchanged = t.state_at == w.st.state
        && t.candidate_at == w.st.candidate
        && t.run_at == w.st.run.as_ref().map(|r| r.id);
    match (t.kind, t.return_to) {
        (TurnKind::Consultation, Some(back)) if unchanged && w.st.pending_request.is_none() => {
            grant_explicit(w, &back, TurnKind::Default, None, now)
        }
        _ => schedule(w, now),
    }
}

fn start_run(w: &mut Work, candidate: ArtId) {
    let run = RunId(w.st.next_run);
    w.emit(Event::RunStarted { candidate, run });
}

fn transition(w: &mut Work, to: State, now: Time) {
    let from = w.st.state;
    if from == to {
        return;
    }
    for id in live_ids(&w.st) {
        end_turn_only(w, id, TurnEnd::Revoked);
    }
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
    let expired: Vec<TurnId> =
        w.st.turns
            .iter()
            .filter(|t| t.deadline <= now)
            .map(|t| t.id)
            .collect();
    for id in expired {
        end_turn(w, id, TurnEnd::Deadline, now);
    }
    if w.st.request_deadline().is_some_and(|d| d <= now) {
        escalate(w, EscalateReason::RequestDeadline, now);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn roster() -> Roster {
        use ArtifactKind::{Candidate, Diff, Spec};
        Roster::new(vec![
            Role::new("planner", 0)
                .write(&[Spec])
                .post(&[MessageKind::RequestDecision]),
            Role::new("coder", 1).write(&[Diff, Candidate]),
            Role::new("tester", 2)
                .post(&[MessageKind::Verdict])
                .verdict_at(&[Stage::Test]),
            Role::new("reviewer", 3)
                .post(&[MessageKind::Verdict])
                .verdict_at(&[Stage::Review])
                .final_decision(),
        ])
        .expect("valid roster")
    }

    fn at(state: State) -> TaskState {
        let mut st = TaskState::new(TaskId("T".into()));
        st.roster = roster();
        st.state = state;
        st
    }

    fn grant(st: &mut TaskState, role: &str, turn: u64, deadline: u64) {
        st.apply(&Event::TurnGranted {
            role: RoleId::from(role),
            turn: TurnId(turn),
            kind: TurnKind::Default,
            deadline: Time(deadline),
            return_to: None,
            slice: None,
        });
    }

    /// The deadline a waiting driver must wake for is the one `fire_deadlines`
    /// would act on, and nothing else: waking for a request that can no longer
    /// escalate would spin.
    #[test]
    fn next_deadline_is_the_earliest_deadline_a_tick_would_fire() {
        let mut st = at(State::Build);
        assert_eq!(st.next_deadline(), None);
        grant(&mut st, "coder", 1, 900);
        grant(&mut st, "tester", 2, 700);
        assert_eq!(st.next_deadline(), Some(Time(700)));
        st.pending_request = Some(PendingRequest {
            msg: MsgId(1),
            requester: RoleId::from("planner"),
            gate: false,
            deadline: Some(Time(500)),
        });
        assert_eq!(st.next_deadline(), Some(Time(500)));
        st.state = State::Escalated;
        assert_eq!(st.next_deadline(), Some(Time(700)));
        st.turns.clear();
        assert_eq!(st.next_deadline(), None);
    }

    #[test]
    fn turn_based_picks_the_roles_that_serve_the_state() {
        for (state, holder) in [
            (State::Planning, "planner"),
            (State::Build, "coder"),
            (State::Test, "tester"),
            (State::Review, "reviewer"),
        ] {
            let got = pick_next(&at(state));
            assert_eq!(got, vec![RoleId::from(holder)], "{state:?}");
        }
    }

    /// `pick_next` as written before the pick rule became `wake::pick_from`,
    /// kept here as the oracle for the test below.
    fn pick_next_before(st: &TaskState) -> Vec<RoleId> {
        let serving = || st.roster.serving(st.state).into_iter().cloned().collect();
        match st.policy {
            SchedulingPolicy::TurnBased => serving(),
            SchedulingPolicy::EventDriven => wake_set(st)
                .into_iter()
                .find(|(_, why)| why.selectable(st.max_live_turns))
                .map_or_else(serving, |(role, _)| vec![role]),
        }
    }

    /// Over every state of the checked-in logs (the one-turn happy path of
    /// `tests/schema.rs` and the four event-driven runs of `tests/pick_golden.rs`),
    /// under both policies, `pick_next` returns what it returned before. The
    /// states include ones where `wake_set` picks a role and ones where it
    /// falls back; the counts below keep the test from passing on neither.
    #[test]
    fn pick_next_is_what_it_was_over_every_state_of_the_golden_logs() {
        let logs = [
            ("n1_happy", include_str!("../tests/golden/n1_happy.log")),
            ("ed_happy", include_str!("../tests/golden/ed_happy.log")),
            (
                "ed_question",
                include_str!("../tests/golden/ed_question.log"),
            ),
            (
                "ed_named_verdict",
                include_str!("../tests/golden/ed_named_verdict.log"),
            ),
            ("ed_quiet", include_str!("../tests/golden/ed_quiet.log")),
        ];
        let (mut states, mut picked, mut fell_back) = (0, 0, 0);
        for (name, text) in logs {
            let events: Vec<Event> = text
                .lines()
                .map(|l| rusty_serde::json::from_str(l).expect("a logged event"))
                .collect();
            for n in 0..=events.len() {
                for policy in [SchedulingPolicy::TurnBased, SchedulingPolicy::EventDriven] {
                    let mut st = TaskState::fold(TaskId("T1".into()), &events[..n]);
                    st.policy = policy;
                    assert_eq!(
                        pick_next(&st),
                        pick_next_before(&st),
                        "{name} after {n} events, {policy:?}"
                    );
                    states += 1;
                    if policy == SchedulingPolicy::EventDriven {
                        match pick_from(&wake_set(&st), st.max_live_turns) {
                            Some(_) => picked += 1,
                            None => fell_back += 1,
                        }
                    }
                }
            }
        }
        assert!(states > 200, "{states} states");
        assert!(
            picked > 20 && fell_back > 20,
            "{picked} picked, {fell_back} fell back"
        );
    }

    #[test]
    fn nobody_is_picked_where_no_role_serves_the_state() {
        for state in [
            State::PlanGate,
            State::MergeGate,
            State::Approved,
            State::Escalated,
            State::Closed,
            State::Cancelled,
        ] {
            assert!(pick_next(&at(state)).is_empty(), "{state:?}");
        }
    }
}
