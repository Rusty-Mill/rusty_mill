#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Claims mode: the trace tests of `docs/spec/bbp-claims-addendum.md`, one
//! test per row, named `cl_NN_*`. Builders: lens principals `a`, `b`, `c`, a
//! scoped role `s`, the runner, the human. Each test drives the pure
//! `handle` and folds what it returns, so the log is the whole truth.

use rusty_bbp::*;

fn lens(id: &str, order: u16) -> Role {
    use ArtifactKind::*;
    use MessageKind::*;
    Role::new(id, order)
        .post(&[Claim, Evidence])
        .write(&[Diff, Candidate, Source])
        .read_artifacts(&[Brief, Diff, Candidate, TestReport, Log, Source])
}

fn roster() -> Vec<Role> {
    vec![
        lens("a", 0),
        lens("b", 1),
        lens("c", 2),
        lens("s", 3).scoped(),
    ]
}

fn blob(tag: &str) -> BlobRef {
    BlobRef {
        sha: Sha256::of(tag.as_bytes()),
        len: tag.len() as u64,
    }
}

fn open_task(mode: Mode, claims: Option<ClaimsConfig>, roster: Vec<Role>) -> OpenTask {
    OpenTask {
        task: TaskId::from("t"),
        roster,
        repo: "repo".into(),
        profile_digest: Sha256::of(b"profiles"),
        budget: Budget {
            messages: 400,
            bytes: 5_000_000,
            reads: 1_000,
            reads_per_turn: 30,
            turns: 60,
            iterations: 4,
        },
        turn_ms: 600_000,
        request_ms: 86_400_000,
        max_live_turns: 1,
        brief: blob("brief"),
        human: PrincipalId::from("human"),
        policy: SchedulingPolicy::TurnBased,
        mode,
        claims,
        holdout_digest: None,
    }
}

struct Cx {
    st: TaskState,
    log: Vec<Event>,
    now: Time,
    ops: u64,
    brief: ArtId,
    /// The tree the runner reports.
    tree: Option<Sha256>,
}

fn pid(s: &str) -> PrincipalId {
    PrincipalId::from(s)
}

impl Cx {
    fn with(cfg: ClaimsConfig) -> Cx {
        let mut cx = Cx {
            st: TaskState::new(TaskId::from("t")),
            log: Vec::new(),
            now: Time(1_000),
            ops: 0,
            brief: ArtId(0),
            tree: Some(Sha256::of(b"tree")),
        };
        let r = cx.run(Command::Open(open_task(Mode::Claims, Some(cfg), roster())));
        let Response::Stored(brief) = r else {
            panic!("open: {r:?}")
        };
        cx.brief = brief;
        for who in ["a", "b", "c", "s"] {
            cx.assign(who);
        }
        cx
    }

    fn new() -> Cx {
        Cx::with(ClaimsConfig::default())
    }

    fn assign(&mut self, who: &str) {
        ok(self.run(Command::Assign {
            role: RoleId::from(who),
            principal: Principal {
                id: pid(who),
                kind: PrincipalKind::Agent {
                    role: RoleId::from(who),
                    vendor: "test".into(),
                },
            },
        }));
    }

    fn run(&mut self, cmd: Command) -> Response {
        let h = handle(&self.st, &cmd, self.now);
        for e in &h.events {
            self.st.apply(e);
        }
        self.log.extend(h.events);
        h.response
    }

    fn op(&mut self) -> OpId {
        self.ops += 1;
        OpId(format!("op{}", self.ops))
    }

    fn cred(&self, who: &str) -> Option<Token> {
        self.st.credential_for(&pid(who))
    }

    fn agent_with(
        &mut self,
        who: &str,
        token: Option<Token>,
        op: OpId,
        a: AgentAction,
    ) -> Response {
        self.run(Command::Agent {
            principal: pid(who),
            token,
            op,
            action: a,
        })
    }

    fn agent(&mut self, who: &str, a: AgentAction) -> Response {
        let op = self.op();
        let t = self.cred(who);
        self.agent_with(who, t, op, a)
    }

    fn human(&mut self, a: HumanAction) -> Response {
        let op = self.op();
        self.run(Command::Human { op, action: a })
    }

    fn host(&mut self, a: HostAction) -> Response {
        let op = self.op();
        self.run(Command::Host { op, action: a })
    }

    fn claim_with(&mut self, who: &str, body: ClaimBody) -> Response {
        let d = Draft {
            kind: MessageKind::Claim,
            to: vec![Recipient::All],
            body: Body::Claim(body),
            refs: vec![Ref::art(self.brief)],
            evidence: vec![],
            reply_to: None,
            yield_to: None,
            gate: false,
        };
        self.agent(who, AgentAction::Post(d))
    }

    fn claim(&mut self, who: &str, body: ClaimBody) -> MsgId {
        posted(self.claim_with(who, body))
    }

    fn low(&mut self) -> MsgId {
        self.claim("a", body("x"))
    }

    fn evidence_draft(
        &self,
        c: MsgId,
        dir: Direction,
        kind: EvidenceKind,
        strength: u8,
        ev: Vec<Ref>,
    ) -> Draft {
        Draft {
            kind: MessageKind::Evidence,
            to: vec![Recipient::All],
            body: Body::Evidence(EvidenceBody {
                claim: c,
                direction: dir,
                kind,
                strength,
                note: String::new(),
            }),
            refs: vec![Ref::Msg(c)],
            evidence: ev,
            reply_to: None,
            yield_to: None,
            gate: false,
        }
    }

    fn ev(
        &mut self,
        who: &str,
        c: MsgId,
        dir: Direction,
        kind: EvidenceKind,
        strength: u8,
        ev: Vec<Ref>,
    ) -> Response {
        let d = self.evidence_draft(c, dir, kind, strength, ev);
        self.agent(who, AgentAction::Post(d))
    }

    fn opinion(&mut self, who: &str, c: MsgId, dir: Direction) -> Response {
        self.ev(who, c, dir, EvidenceKind::Opinion, 1, vec![])
    }

    fn cited(&mut self, who: &str, c: MsgId, dir: Direction, strength: u8) -> Response {
        let b = self.brief;
        self.ev(
            who,
            c,
            dir,
            EvidenceKind::Cited,
            strength,
            vec![Ref::art(b)],
        )
    }

    fn executed(&mut self, who: &str, c: MsgId, dir: Direction, report: ArtId) -> Response {
        self.ev(
            who,
            c,
            dir,
            EvidenceKind::Executed,
            1,
            vec![Ref::art(report)],
        )
    }

    fn put(&mut self, who: &str, tag: &str, payload: ArtifactPayload) -> Response {
        let b = blob(tag);
        self.agent(who, AgentAction::PutArtifact { blob: b, payload })
    }

    /// A diff and a candidate naming `c`, by `who`. Returns the candidate.
    fn candidate(&mut self, who: &str, c: MsgId) -> ArtId {
        self.ops += 1;
        let diff = stored(self.put(who, &format!("diff{}", self.ops), ArtifactPayload::Diff));
        stored(self.put(
            who,
            &format!("cand{}", self.ops),
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![diff],
                claim: Some(c),
                suite: Suite::Build,
            }),
        ))
    }

    fn live_run(&self, k: ArtId) -> RunId {
        self.st.claims.live_run(k).expect("live run")
    }

    /// The runner stores a log and a report for `k`'s live run.
    fn report_profiles(
        &mut self,
        k: ArtId,
        profiles: &[(&str, u32)],
        tests_changed: Vec<TestChange>,
    ) -> Response {
        let run = self.live_run(k);
        let secret = self.st.secret_for(run);
        self.report_as(k, run, secret, profiles, tests_changed)
    }

    fn report_as(
        &mut self,
        k: ArtId,
        run: RunId,
        secret: RunSecret,
        profiles: &[(&str, u32)],
        tests_changed: Vec<TestChange>,
    ) -> Response {
        let digest = self.st.profile_digest;
        self.report_full(k, run, secret, digest, profiles, tests_changed)
    }

    /// `k`'s live run reports under `digest` (a holdout run passes the
    /// task's holdout digest).
    fn report_under(&mut self, k: ArtId, digest: Sha256, profiles: &[(&str, u32)]) -> Response {
        let run = self.live_run(k);
        let secret = self.st.secret_for(run);
        self.report_full(k, run, secret, digest, profiles, vec![])
    }

    fn report_full(
        &mut self,
        k: ArtId,
        run: RunId,
        secret: RunSecret,
        digest: Sha256,
        profiles: &[(&str, u32)],
        tests_changed: Vec<TestChange>,
    ) -> Response {
        let op = self.op();
        let log = self.run(Command::Runner {
            op,
            run,
            secret,
            blob: blob(&format!("log{}", self.ops)),
            payload: ArtifactPayload::Log,
        });
        let log = match log {
            Response::Stored(id) => id,
            other => return other,
        };
        let failed = profiles.iter().any(|(_, f)| *f > 0);
        let rep = Report {
            candidate: k,
            run,
            profile_digest: digest,
            status: if failed {
                RunStatus::Failed
            } else {
                RunStatus::Passed
            },
            profiles: profiles
                .iter()
                .map(|(n, f)| ProfileResult {
                    name: (*n).into(),
                    exit_code: if *f > 0 { 1 } else { 0 },
                    failed: *f,
                })
                .collect(),
            tree: self.tree,
            sandbox: "landlock+seccomp:no-net".into(),
            log,
            tests_changed,
        };
        let op = self.op();
        self.run(Command::Runner {
            op,
            run,
            secret,
            blob: blob(&format!("rep{}", self.ops)),
            payload: ArtifactPayload::TestReport(rep),
        })
    }

    /// A candidate on `c` by `who` whose run reports `status`; returns the report.
    fn tested(&mut self, who: &str, c: MsgId, pass: bool) -> ArtId {
        let k = self.candidate(who, c);
        let profiles: &[(&str, u32)] = if pass { &[("unit", 0)] } else { &[("unit", 1)] };
        stored(self.report_profiles(k, profiles, vec![]))
    }

    fn rec(&self, c: MsgId) -> &rusty_bbp::claims::ClaimRecord {
        self.st.claims.get(c).expect("claim")
    }

    fn status(&self, c: MsgId) -> ClaimStatus {
        self.rec(c).status
    }

    fn score(&self, c: MsgId) -> i64 {
        self.rec(c).score()
    }

    fn rev(&self) -> Rev {
        self.st.rev
    }

    fn count(&self, f: impl Fn(&Event) -> bool) -> usize {
        self.log.iter().filter(|e| f(e)).count()
    }

    fn mark(&mut self, c: MsgId, reference: &str) -> u64 {
        let epoch = self.rec(c).epoch;
        match self.host(HostAction::Acting {
            claim: c,
            epoch,
            reference: reference.into(),
        }) {
            Response::Marked(m) => m,
            other => panic!("mark: {other:?}"),
        }
    }

    fn decide(&mut self, c: MsgId, outcome: ClaimOutcome) -> Response {
        let rev = self.rev();
        self.human(HumanAction::Fenced {
            expected_rev: rev,
            action: Box::new(HumanAction::ClaimDecision { claim: c, outcome }),
        })
    }
}

fn body(text: &str) -> ClaimBody {
    ClaimBody {
        text: text.into(),
        domains: vec!["http".into()],
        deps: vec![],
        answers: None,
        supersedes: None,
        stakes: Stakes::Low,
        reversible: true,
    }
}

fn ok(r: Response) {
    assert!(!matches!(r, Response::Rejected(_)), "rejected: {r:?}");
}

fn posted(r: Response) -> MsgId {
    match r {
        Response::Posted(id) => id,
        other => panic!("not posted: {other:?}"),
    }
}

fn stored(r: Response) -> ArtId {
    match r {
        Response::Stored(id) => id,
        other => panic!("not stored: {other:?}"),
    }
}

fn rejected(r: Response) -> Code {
    match r {
        Response::Rejected(x) => x.code,
        other => panic!("not rejected: {other:?}"),
    }
}

fn detail(r: Response) -> (Code, String) {
    match r {
        Response::Rejected(x) => (x.code, x.detail),
        other => panic!("not rejected: {other:?}"),
    }
}

use Direction::{Counter, Support};

// ---------------------------------------------------------------- mode

#[test]
fn cl_01_stage_open_without_mode_is_unchanged() {
    // The serialized `task_opened` of a stage task carries neither `mode`
    // nor `claims`, so its bytes match a pre-addendum build's.
    let mut o = open_task(Mode::Stages, None, stage_roster());
    o.claims = None;
    let h = handle(
        &TaskState::new(TaskId::from("t")),
        &Command::Open(o),
        Time(0),
    );
    let opened = h
        .events
        .iter()
        .find(|e| matches!(e, Event::TaskOpened { .. }))
        .expect("opened");
    let json = rusty_serde::json::to_string(opened).expect("json");
    assert!(!json.contains("mode"), "{json}");
    assert!(!json.contains("claims"), "{json}");
    assert!(json.contains("\"schema\":2"), "{json}");
}

fn stage_roster() -> Vec<Role> {
    use ArtifactKind::*;
    use MessageKind::*;
    let doer = [Ask, Answer, Propose, Finding, RequestDecision, Pass];
    vec![
        Role::new("planner", 0).post(&doer).write(&[Spec]),
        Role::new("coder", 1).post(&doer).write(&[Diff, Candidate]),
        Role::new("tester", 2)
            .post(&[Verdict, Pass])
            .verdict_at(&[Stage::Test]),
        Role::new("reviewer", 3)
            .post(&[Verdict, Pass])
            .verdict_at(&[Stage::Review])
            .final_decision(),
    ]
}

#[test]
fn cl_01b_claims_open_writes_schema_5() {
    let cx = Cx::new();
    assert_eq!(cx.st.schema, LOG_SCHEMA_CLAIMS);
    assert_eq!(cx.st.state, State::Open);
    assert_eq!(cx.st.mode, Mode::Claims);
}

#[test]
fn cl_02_bad_config_refused_naming_the_field() {
    let cfg = ClaimsConfig {
        accept_at: 0,
        ..ClaimsConfig::default()
    };
    let h = handle(
        &TaskState::new(TaskId::from("t")),
        &Command::Open(open_task(Mode::Claims, Some(cfg), roster())),
        Time(0),
    );
    assert!(h.events.is_empty());
    let (code, d) = detail(h.response);
    assert_eq!(code, Code::BadClaimsConfig);
    assert!(d.contains("accept_at"), "{d}");
    // A claims open that names live turns is refused too.
    let mut o = open_task(Mode::Claims, None, roster());
    o.max_live_turns = 2;
    let h = handle(
        &TaskState::new(TaskId::from("t")),
        &Command::Open(o),
        Time(0),
    );
    assert_eq!(rejected(h.response), Code::BadClaimsConfig);
}

#[test]
fn cl_03_claims_roster_needs_an_evidence_poster() {
    let roles = vec![Role::new("a", 0).post(&[MessageKind::Claim])];
    let h = handle(
        &TaskState::new(TaskId::from("t")),
        &Command::Open(open_task(Mode::Claims, None, roles)),
        Time(0),
    );
    let (code, d) = detail(h.response);
    assert_eq!(code, Code::BadRoster);
    assert!(d.contains("Unservable(claims)"), "{d}");
}

#[test]
fn cl_04_evidence_needs_no_turn() {
    let mut cx = Cx::new();
    let c = cx.low();
    ok(cx.opinion("b", c, Support));
    assert_eq!(cx.count(|e| matches!(e, Event::TurnGranted { .. })), 0);
    ok(cx.run(Command::Tick));
    assert_eq!(cx.count(|e| matches!(e, Event::TurnGranted { .. })), 0);
}

#[test]
fn cl_05_foreign_or_stale_credential_refused() {
    let mut cx = Cx::new();
    let c = cx.low();
    let d = cx.evidence_draft(c, Support, EvidenceKind::Opinion, 1, vec![]);
    let op = cx.op();
    let b = cx.cred("b");
    assert_eq!(
        rejected(cx.agent_with("a", b, op, AgentAction::Post(d.clone()))),
        Code::BadCredential
    );
    let old = cx.cred("a");
    cx.assign("a");
    let op = cx.op();
    assert_eq!(
        rejected(cx.agent_with("a", old, op, AgentAction::Post(d.clone()))),
        Code::BadCredential
    );
    let op = cx.op();
    assert_eq!(
        rejected(cx.agent_with("a", None, op, AgentAction::Post(d))),
        Code::BadCredential
    );
}

#[test]
fn cl_06_claim_in_stage_task_is_wrong_mode() {
    let mut st = TaskState::new(TaskId::from("t"));
    let roles = {
        let mut r = stage_roster();
        r[0].post.push(MessageKind::Claim);
        r
    };
    let h = handle(
        &st,
        &Command::Open(open_task(Mode::Stages, None, roles)),
        Time(0),
    );
    for e in &h.events {
        st.apply(e);
    }
    let h = handle(
        &st,
        &Command::Assign {
            role: RoleId::from("planner"),
            principal: Principal {
                id: pid("planner"),
                kind: PrincipalKind::Agent {
                    role: RoleId::from("planner"),
                    vendor: "t".into(),
                },
            },
        },
        Time(0),
    );
    for e in &h.events {
        st.apply(e);
    }
    let d = Draft {
        kind: MessageKind::Claim,
        to: vec![Recipient::All],
        body: Body::Claim(body("x")),
        refs: vec![Ref::art(ArtId(0))],
        evidence: vec![],
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    let h = handle(
        &st,
        &Command::Agent {
            principal: pid("planner"),
            token: None,
            op: OpId::from("o"),
            action: AgentAction::Post(d),
        },
        Time(0),
    );
    assert_eq!(rejected(h.response), Code::WrongMode);
}

// ---------------------------------------------------------------- update rule

#[test]
fn cl_07_two_executed_supports_accept() {
    let mut cx = Cx::new();
    let c = cx.low();
    let r = cx.tested("a", c, true);
    ok(cx.executed("a", c, Support, r));
    assert_eq!(cx.status(c), ClaimStatus::Open);
    ok(cx.executed("b", c, Support, r));
    assert_eq!(cx.status(c), ClaimStatus::Accepted);
    assert!(matches!(
        cx.log
            .iter()
            .rev()
            .find(|e| matches!(e, Event::ClaimAccepted { .. })),
        Some(Event::ClaimAccepted { score: 8, .. })
    ));
}

#[test]
fn cl_08_one_author_never_accepts_and_executed_strength_is_one() {
    let mut cx = Cx::new();
    let c = cx.low();
    let r = cx.tested("a", c, true);
    for _ in 0..3 {
        ok(cx.executed("a", c, Support, r));
    }
    assert_eq!(cx.score(c), 12);
    assert_eq!(cx.status(c), ClaimStatus::Open);
    let r2 = cx.ev(
        "b",
        c,
        Support,
        EvidenceKind::Executed,
        2,
        vec![Ref::art(r)],
    );
    assert_eq!(rejected(r2), Code::BadClaim);
}

#[test]
fn cl_09_opinions_alone_never_accept() {
    let mut cx = Cx::new();
    let c = cx.low();
    for i in 0..4 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.ev(who, c, Support, EvidenceKind::Opinion, 2, vec![]));
    }
    assert_eq!(cx.score(c), 8);
    assert_eq!(cx.status(c), ClaimStatus::Open);
}

#[test]
fn cl_10_drop_needs_two_counter_authors() {
    let mut cx = Cx::new();
    let c = cx.low();
    let r = cx.tested("c", c, false);
    ok(cx.executed("a", c, Counter, r));
    assert_eq!(cx.score(c), -8);
    assert_eq!(cx.status(c), ClaimStatus::Open);
    ok(cx.executed("b", c, Counter, r));
    assert_eq!(cx.status(c), ClaimStatus::Dropped);
    assert!(cx
        .log
        .iter()
        .any(|e| matches!(e, Event::ClaimDropped { score: -16, .. })));
}

#[test]
fn cl_11_executed_must_cite_a_runner_report() {
    let mut cx = Cx::new();
    let c = cx.low();
    let k = cx.candidate("a", c);
    let diff = match &cx.st.artifacts[&k].payload {
        ArtifactPayload::Candidate(x) => x.diffs[0],
        _ => unreachable!(),
    };
    assert_eq!(
        rejected(cx.executed("a", c, Support, diff)),
        Code::WrongKind
    );
    assert_eq!(
        rejected(cx.ev("a", c, Support, EvidenceKind::Executed, 1, vec![])),
        Code::EvidenceRequired
    );
}

#[test]
fn cl_12_irreversible_escalates_then_human_accepts() {
    let mut cx = Cx::new();
    let c = cx.claim(
        "a",
        ClaimBody {
            reversible: false,
            ..body("irreversible")
        },
    );
    let other = cx.low();
    let r = cx.tested("a", c, true);
    ok(cx.executed("a", c, Support, r));
    ok(cx.executed("b", c, Support, r));
    ok(cx.executed("c", c, Support, r));
    assert_eq!(cx.status(c), ClaimStatus::Escalated);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated {
            reason: ClaimEscalation::Irreversible,
            ..
        }
    )));
    assert_eq!(cx.count(|e| matches!(e, Event::ClaimAccepted { .. })), 0);
    ok(cx.opinion("b", other, Support));
    ok(cx.decide(c, ClaimOutcome::Accept));
    assert_eq!(cx.status(c), ClaimStatus::Accepted);
}

#[test]
fn cl_13_irreversible_drops_without_a_human() {
    let mut cx = Cx::new();
    let c = cx.claim(
        "a",
        ClaimBody {
            reversible: false,
            ..body("irreversible")
        },
    );
    let r = cx.tested("c", c, false);
    ok(cx.executed("a", c, Counter, r));
    ok(cx.executed("b", c, Counter, r));
    assert_eq!(cx.status(c), ClaimStatus::Dropped);
}

fn accept(cx: &mut Cx, c: MsgId) {
    let r = cx.tested("c", c, true);
    ok(cx.executed("a", c, Support, r));
    ok(cx.executed("b", c, Support, r));
    assert_eq!(cx.status(c), ClaimStatus::Accepted, "accept {c}");
}

fn drop_claim(cx: &mut Cx, c: MsgId) {
    let b = cx.brief;
    ok(cx.ev("a", c, Counter, EvidenceKind::Cited, 3, vec![Ref::art(b)]));
    ok(cx.ev("b", c, Counter, EvidenceKind::Cited, 1, vec![Ref::art(b)]));
    assert_eq!(cx.status(c), ClaimStatus::Dropped, "drop {c}");
}

fn dep_on(text: &str, deps: Vec<MsgId>) -> ClaimBody {
    ClaimBody { deps, ..body(text) }
}

#[test]
fn cl_14_dependent_voided_and_stays_void() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    // The run is made while B is open: a candidate on a void claim is refused.
    let r = cx.tested("c", b, true);
    drop_claim(&mut cx, a);
    assert_eq!(cx.status(b), ClaimStatus::Void);
    assert!(cx
        .log
        .iter()
        .any(|e| matches!(e, Event::ClaimVoided { claim, by } if *claim == b && *by == a)));
    ok(cx.executed("a", b, Support, r));
    ok(cx.executed("c", b, Support, r));
    assert_eq!(cx.status(b), ClaimStatus::Void);
    assert_eq!(cx.score(b), 0);
}

#[test]
fn cl_15_void_cascades_to_a_fixed_point() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let c = cx.claim("c", dep_on("c", vec![b]));
    drop_claim(&mut cx, a);
    assert_eq!(cx.status(b), ClaimStatus::Void);
    assert_eq!(cx.status(c), ClaimStatus::Void);
}

#[test]
fn cl_16_dependent_waits_for_its_dep() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let r = cx.tested("c", b, true);
    ok(cx.executed("a", b, Support, r));
    ok(cx.executed("b", b, Support, r));
    assert_eq!(cx.status(b), ClaimStatus::Open);
    accept(&mut cx, a);
    assert_eq!(cx.status(b), ClaimStatus::Accepted);
}

#[test]
fn cl_17_acted_on_dependent_escalates_when_dep_falls() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    accept(&mut cx, a);
    accept(&mut cx, b);
    let m = cx.mark(b, "pr-1");
    ok(cx.host(HostAction::ActionResult {
        mark: m,
        outcome: MarkOutcome::Done,
    }));
    ok(cx.decide(a, ClaimOutcome::Reopen));
    assert_eq!(cx.status(b), ClaimStatus::Escalated);
    ok(cx.decide(a, ClaimOutcome::Drop));
    assert_eq!(cx.status(b), ClaimStatus::Escalated);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated { claim, reason: ClaimEscalation::DepCollapsed } if *claim == b
    )));
}

#[test]
fn cl_18_flat_evidence_escalates_stuck() {
    let mut cx = Cx::new();
    let c = cx.low();
    for i in 0..6 {
        let (who, dir) = if i % 2 == 0 {
            ("a", Support)
        } else {
            ("b", Counter)
        };
        ok(cx.opinion(who, c, dir));
        if i < 5 {
            assert_eq!(cx.status(c), ClaimStatus::Open, "item {i}");
        }
    }
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated {
            reason: ClaimEscalation::Stuck,
            ..
        }
    )));
}

#[test]
fn cl_19_budget_escalates_then_refuses_then_extends() {
    let mut cx = Cx::with(ClaimsConfig {
        settle_window: 50,
        ..ClaimsConfig::default()
    });
    let c = cx.low();
    for i in 0..8 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.opinion(who, c, Support));
    }
    assert_eq!(cx.status(c), ClaimStatus::Escalated);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated {
            reason: ClaimEscalation::Budget,
            ..
        }
    )));
    assert_eq!(rejected(cx.opinion("a", c, Support)), Code::ClaimBudget);
    ok(cx.human(HumanAction::ClaimBudgetExtended { claim: c }));
    ok(cx.decide(c, ClaimOutcome::Reopen));
    assert_eq!(cx.status(c), ClaimStatus::Open);
    ok(cx.opinion("a", c, Support));
}

#[test]
fn cl_20_budget_arithmetic() {
    let mut cx = Cx::new();
    let c = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::High,
            reversible: false,
            ..body("big")
        },
    );
    assert_eq!(cx.rec(c).budget(&cx.st.claims.config), 64);
    let m = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::Medium,
            ..body("mid")
        },
    );
    assert_eq!(cx.rec(m).budget(&cx.st.claims.config), 16);
}

#[test]
fn cl_21_sole_candidate_is_a_fact() {
    let mut cx = Cx::new();
    let q = cx.low();
    let x = cx.claim(
        "a",
        ClaimBody {
            answers: Some(q),
            ..body("x")
        },
    );
    accept(&mut cx, x);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimAccepted { claim, sole_candidate: true, .. } if *claim == x
    )));
    let q2 = cx.low();
    let y1 = cx.claim(
        "a",
        ClaimBody {
            answers: Some(q2),
            ..body("y1")
        },
    );
    let _y2 = cx.claim(
        "b",
        ClaimBody {
            answers: Some(q2),
            ..body("y2")
        },
    );
    accept(&mut cx, y1);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimAccepted { claim, sole_candidate: false, .. } if *claim == y1
    )));
    assert_eq!(cx.count(|e| matches!(e, Event::ClaimEscalated { .. })), 0);
}

#[test]
fn cl_23_replay_after_settlement_returns_the_original() {
    let mut cx = Cx::new();
    let c = cx.low();
    let r = cx.tested("c", c, true);
    ok(cx.executed("a", c, Support, r));
    let d = cx.evidence_draft(c, Support, EvidenceKind::Executed, 1, vec![Ref::art(r)]);
    let op = cx.op();
    let t = cx.cred("b");
    let first = cx.agent_with("b", t, op.clone(), AgentAction::Post(d.clone()));
    assert_eq!(cx.status(c), ClaimStatus::Accepted);
    let n = cx.log.len();
    let spent = cx.rec(c).spent();
    let again = cx.agent_with("b", t, op, AgentAction::Post(d));
    assert_eq!(first, again);
    assert_eq!(cx.log.len(), n);
    assert_eq!(cx.rec(c).spent(), spent);
}

#[test]
fn cl_25_order_dependent_example_replays_identically() {
    // a, b, c: accepted at b, reopened by c into epoch 2 at −8.
    let mut cx = Cx::new();
    let x = cx.low();
    let pass = cx.tested("a", x, true);
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    let fail = cx.tested("c", x, false);
    ok(cx.executed("c", x, Counter, fail));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert_eq!(cx.rec(x).epoch, 2);
    assert_eq!(cx.score(x), -8);
    let refold = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(refold.claims, cx.st.claims);

    // a, c, b: never settles, stays open at 0 in epoch 1.
    let mut cx = Cx::new();
    let x = cx.low();
    let pass = cx.tested("a", x, true);
    let fail = cx.tested("c", x, false);
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("c", x, Counter, fail));
    ok(cx.executed("b", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert_eq!(cx.rec(x).epoch, 1);
    assert_eq!(cx.score(x), 0);
}

#[test]
fn cl_26_delta_returns_changed_claims_and_dependents() {
    let mut cx = Cx::new();
    let h = cx.low();
    let other = cx.claim(
        "a",
        ClaimBody {
            domains: vec!["db".into()],
            ..body("db")
        },
    );
    let dep = cx.claim(
        "b",
        ClaimBody {
            domains: vec!["ui".into()],
            ..dep_on("ui", vec![h])
        },
    );
    let after = cx.rev();
    ok(cx.opinion("a", h, Support));
    ok(cx.opinion("a", other, Support));
    let r = cx.agent(
        "a",
        AgentAction::ClaimsDelta {
            after,
            domains: vec!["http".into()],
        },
    );
    let Response::Claims(v) = r else {
        panic!("{r:?}")
    };
    let ids: Vec<MsgId> = v.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![h, dep]);
}

#[test]
fn cl_27_line_matches_the_golden_format() {
    let mut cx = Cx::new();
    let c = cx.claim(
        "a",
        body("One shared HTTP client covers all 7 call sites without behaviour change."),
    );
    accept(&mut cx, c);
    let r = cx.agent("a", AgentAction::ClaimLine { claim: c });
    assert_eq!(
        r,
        Response::Line(Some(format!(
            "{c} ACCEPTED +8 (90%) 2 evd · One shared HTTP client covers all 7 call sites without beha…"
        )))
    );
}

#[test]
fn cl_28_task_budget_crossing_escalates_the_task_once() {
    let mut cx = Cx::new();
    let mut o = open_task(Mode::Claims, None, roster());
    // Escalation fires when spent exceeds the limit: the third message crosses.
    o.budget.messages = 2;
    cx.st = TaskState::new(TaskId::from("t"));
    cx.log.clear();
    let r = cx.run(Command::Open(o));
    cx.brief = stored(r);
    for who in ["a", "b", "c", "s"] {
        cx.assign(who);
    }
    let c = cx.low();
    ok(cx.opinion("a", c, Support));
    ok(cx.opinion("b", c, Support));
    assert_eq!(cx.st.state, State::Escalated);
    assert_eq!(cx.count(|e| matches!(e, Event::Escalated { .. })), 1);
    assert_eq!(rejected(cx.opinion("c", c, Support)), Code::WrongState);
    ok(cx.human(HumanAction::BudgetExtended {
        field: BudgetField::Messages,
        limit: 10,
    }));
    let rev = cx.rev();
    let r = cx.human(HumanAction::Resume {
        target: State::Planning,
        expected_rev: rev,
    });
    assert_eq!(
        rejected(r),
        Code::WrongState,
        "a claims task resumes only to open"
    );
    let rev = cx.rev();
    ok(cx.human(HumanAction::Resume {
        target: State::Open,
        expected_rev: rev,
    }));
    ok(cx.opinion("c", c, Support));
}

#[test]
fn cl_29_end_to_end_with_normal_apis() {
    let mut cx = Cx::new();
    let x = cx.low();
    let d1 = stored(cx.put("a", "d1", ArtifactPayload::Diff));
    let d2 = stored(cx.put("a", "d2", ArtifactPayload::Diff));
    let k = stored(cx.put(
        "a",
        "k",
        ArtifactPayload::Candidate(Candidate {
            base: "a".repeat(40),
            diffs: vec![d1, d2],
            claim: Some(x),
            suite: Suite::Build,
        }),
    ));
    assert!(cx
        .log
        .iter()
        .any(|e| matches!(e, Event::RunStarted { candidate, .. } if *candidate == k)));
    let rep = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    ok(cx.executed("a", x, Support, rep));
    ok(cx.executed("b", x, Support, rep));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    assert_eq!(cx.count(|e| matches!(e, Event::TurnGranted { .. })), 0);
}

#[test]
fn cl_30_writes_bind_to_the_credential_and_mode() {
    let mut cx = Cx::new();
    let x = cx.low();
    let op = cx.op();
    let b = cx.cred("b");
    assert_eq!(
        rejected(cx.agent_with(
            "a",
            b,
            op,
            AgentAction::PutArtifact {
                blob: blob("d"),
                payload: ArtifactPayload::Diff
            }
        )),
        Code::BadCredential
    );
    // A candidate without a claim is refused in claims mode.
    let d = stored(cx.put("a", "d", ArtifactPayload::Diff));
    assert_eq!(
        rejected(cx.put(
            "a",
            "k",
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![d],
                claim: None,
                suite: Suite::Build,
            })
        )),
        Code::BadClaim
    );
    let _ = x;
}

#[test]
fn cl_31_runner_writes_are_fenced() {
    let mut cx = Cx::new();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let run = cx.live_run(k);
    let bad = RunSecret(Sha256::of(b"nope"));
    assert_eq!(
        rejected(cx.report_as(k, run, bad, &[("unit", 0)], vec![])),
        Code::BadRunSecret
    );
    let old = cx.st.secret_for(run);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    assert_eq!(
        rejected(cx.report_as(k, run, old, &[("unit", 0)], vec![])),
        Code::StaleRun
    );
    stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let run2 = cx.live_run(k);
    let s2 = cx.st.secret_for(run2);
    assert_eq!(
        rejected(cx.report_as(k, run2, s2, &[("unit", 0)], vec![])),
        Code::DuplicateReport
    );
}

#[test]
fn cl_32_executed_evidence_binds_to_its_claim() {
    let mut cx = Cx::new();
    let x = cx.low();
    let y = cx.low();
    let rx = cx.tested("a", x, true);
    assert_eq!(
        rejected(cx.executed("a", y, Support, rx)),
        Code::EvidenceUnbound
    );
    let fail = cx.tested("a", y, false);
    assert_eq!(
        rejected(cx.executed("a", y, Support, fail)),
        Code::NotPassed
    );
}

#[test]
fn cl_33_new_claim_may_not_rest_on_a_closed_dep() {
    let mut cx = Cx::new();
    let a = cx.low();
    let v = cx.claim("a", dep_on("v", vec![a]));
    drop_claim(&mut cx, a);
    assert_eq!(
        rejected(cx.claim_with("b", dep_on("x", vec![a]))),
        Code::DepClosed
    );
    assert_eq!(
        rejected(cx.claim_with("b", dep_on("x", vec![v]))),
        Code::DepClosed
    );
    let e = cx.claim(
        "a",
        ClaimBody {
            reversible: false,
            ..body("e")
        },
    );
    let r = cx.tested("c", e, true);
    for who in ["a", "b", "c"] {
        ok(cx.executed(who, e, Support, r));
    }
    assert_eq!(cx.status(e), ClaimStatus::Escalated);
    ok(cx.claim_with("b", dep_on("x", vec![e])));
}

#[test]
fn cl_35_scoped_role_sees_only_what_it_may_read() {
    let mut cx = Cx::new();
    let hidden = cx.low();
    // The human addresses a claim to `s`; it depends on `hidden` and has
    // evidence from `a`.
    let d = Draft {
        kind: MessageKind::Claim,
        to: vec![Recipient::Role(RoleId::from("s"))],
        body: Body::Claim(dep_on("for s", vec![hidden])),
        refs: vec![Ref::art(cx.brief)],
        evidence: vec![],
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    let mine = posted(cx.human(HumanAction::Post(d)));
    ok(cx.opinion("a", mine, Support));
    let r = cx.agent("s", AgentAction::ClaimCard { claim: hidden });
    assert_eq!(r, Response::Claims(vec![]));
    let r = cx.agent("s", AgentAction::ClaimLine { claim: hidden });
    assert_eq!(r, Response::Line(None));
    let r = cx.agent("s", AgentAction::ClaimCard { claim: mine });
    let Response::Claims(v) = r else {
        panic!("{r:?}")
    };
    assert_eq!(v.len(), 1);
    assert!(v[0].evidence.is_empty(), "other agents' evidence hidden");
    assert!(v[0].deps.is_empty(), "hidden dep omitted");
    let r = cx.agent(
        "s",
        AgentAction::ClaimsDelta {
            after: Rev(0),
            domains: vec!["http".into()],
        },
    );
    let Response::Claims(v) = r else {
        panic!("{r:?}")
    };
    assert_eq!(v.iter().map(|c| c.id).collect::<Vec<_>>(), vec![mine]);
    let op = cx.op();
    assert_eq!(
        rejected(cx.agent_with("s", None, op, AgentAction::ClaimCard { claim: mine })),
        Code::BadCredential
    );
}

#[test]
fn cl_36_all_reader_sees_everything() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    ok(cx.opinion("c", b, Support));
    let r = cx.agent("a", AgentAction::ClaimCard { claim: b });
    let Response::Claims(v) = r else {
        panic!("{r:?}")
    };
    assert_eq!(v[0].evidence.len(), 1);
    assert_eq!(v[0].deps, vec![a]);
}

#[test]
fn cl_37_weak_evidence_on_a_settled_claim_is_recorded_only() {
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let score = cx.rec(x).score();
    let n = cx.rec(x).items.len();
    ok(cx.cited("c", x, Counter, 3));
    ok(cx.opinion("c", x, Counter));
    let pass = cx.tested("c", x, true);
    ok(cx.executed("c", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    assert_eq!(cx.rec(x).score(), score);
    assert_eq!(cx.rec(x).items.len(), n + 3);
    assert!(cx.rec(x).items[n..].iter().all(|i| !i.scoring));
}

#[test]
fn cl_38_failing_recheck_reopens_into_a_new_epoch() {
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let spent = cx.rec(x).spent();
    let fail = cx.tested("c", x, false);
    assert_eq!(cx.rec(x).spent(), spent + 1);
    ok(cx.executed("c", x, Counter, fail));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert_eq!(cx.rec(x).epoch, 2);
    assert_eq!(cx.score(x), -8);
    ok(cx.cited("a", x, Support, 3));
    assert_eq!(cx.status(x), ClaimStatus::Open);
}

#[test]
fn cl_39_acted_on_claim_escalates_instead_of_reopening() {
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    cx.mark(x, "pr");
    let fail = cx.tested("c", x, false);
    ok(cx.executed("c", x, Counter, fail));
    assert_eq!(cx.status(x), ClaimStatus::Escalated);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated {
            reason: ClaimEscalation::ReopenedAfterAction,
            ..
        }
    )));
    ok(cx.decide(x, ClaimOutcome::Reopen));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert_eq!(cx.rec(x).epoch, 2);
}

#[test]
fn cl_40_dropped_claim_reopens_on_passing_support() {
    let mut cx = Cx::new();
    let x = cx.low();
    drop_claim(&mut cx, x);
    let pass = cx.tested("c", x, true);
    ok(cx.executed("c", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert_eq!(cx.rec(x).epoch, 2);
    assert_eq!(cx.score(x), 4);
}

#[test]
fn cl_41_unacted_dependent_reopens_with_its_dep() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    accept(&mut cx, a);
    accept(&mut cx, b);
    let fail = cx.tested("c", a, false);
    ok(cx.executed("c", a, Counter, fail));
    assert_eq!(cx.status(a), ClaimStatus::Open);
    assert_eq!(cx.status(b), ClaimStatus::Open);
    assert_eq!(cx.rec(b).epoch, 2);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimReopened { claim, by: ReopenCause::Dep(d), .. } if *claim == b && *d == a
    )));
}

#[test]
fn cl_42_candidate_debits() {
    // (a) 7 items, then a candidate: spent 8, budget escalation.
    let mut cx = Cx::with(ClaimsConfig {
        settle_window: 50,
        ..ClaimsConfig::default()
    });
    let x = cx.low();
    for i in 0..7 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.opinion(who, x, Support));
    }
    cx.candidate("a", x);
    assert_eq!(cx.rec(x).spent(), 8);
    assert_eq!(cx.status(x), ClaimStatus::Escalated);
    // (c) a candidate naming an escalated claim.
    let d = stored(cx.put("a", "dd", ArtifactPayload::Diff));
    assert_eq!(
        rejected(cx.put(
            "a",
            "kk",
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![d],
                claim: Some(x),
                suite: Suite::Build,
            })
        )),
        Code::ClaimNotOpen
    );
}

#[test]
fn cl_43_mark_rules() {
    let mut cx = Cx::new();
    let x = cx.low();
    assert_eq!(
        rejected(cx.host(HostAction::Acting {
            claim: x,
            epoch: 1,
            reference: "pr".into()
        })),
        Code::WrongState
    );
    accept(&mut cx, x);
    assert_eq!(
        rejected(cx.host(HostAction::Acting {
            claim: x,
            epoch: 9,
            reference: "pr".into()
        })),
        Code::WrongState
    );
    assert_eq!(
        rejected(cx.host(HostAction::ActionResult {
            mark: 99,
            outcome: MarkOutcome::Done
        })),
        Code::RefUnresolved
    );
    let m = cx.mark(x, "pr");
    ok(cx.host(HostAction::ActionResult {
        mark: m,
        outcome: MarkOutcome::Done,
    }));
    assert_eq!(
        rejected(cx.host(HostAction::ActionResult {
            mark: m,
            outcome: MarkOutcome::Done
        })),
        Code::Duplicate
    );
}

#[test]
fn cl_45_marks_bind_to_their_epoch() {
    // (b) Contrary evidence reopens first: the mark is refused.
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let fail = cx.tested("c", x, false);
    ok(cx.executed("c", x, Counter, fail));
    assert_eq!(
        rejected(cx.host(HostAction::Acting {
            claim: x,
            epoch: 1,
            reference: "pr".into()
        })),
        Code::WrongState
    );
    // (c) A late result binds to its own mark.
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let m = cx.mark(x, "pr");
    let fail = cx.tested("c", x, false);
    ok(cx.executed("c", x, Counter, fail));
    ok(cx.decide(x, ClaimOutcome::Reopen));
    ok(cx.host(HostAction::ActionResult {
        mark: m,
        outcome: MarkOutcome::Done,
    }));
    let mk = &cx.rec(x).marks[0];
    assert_eq!(mk.epoch, 1);
    assert_eq!(mk.outcome, Some(MarkOutcome::Done));
    assert!(cx.rec(x).acted_on());
}

#[test]
fn cl_46_reruns_past_budget_are_held_for_the_human() {
    let mut cx = Cx::with(ClaimsConfig {
        settle_window: 50,
        ..ClaimsConfig::default()
    });
    let x = cx.low();
    let k = cx.candidate("a", x); // 1 unit
    for i in 0..5 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.opinion(who, x, Support));
    } // 6 units
    let run0 = cx.live_run(k);
    // (a) one unit left... two left here; rerun uses one.
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    assert_eq!(cx.rec(x).spent(), 7);
    assert_ne!(cx.live_run(k), run0);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    assert_eq!(cx.rec(x).spent(), 8);
    // (b) at the limit: held, status unchanged, old run still live.
    let live = cx.live_run(k);
    let before = cx.status(x);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    assert!(cx.log.iter().any(|e| matches!(e, Event::RerunHeld { .. })));
    assert_eq!(cx.status(x), before);
    assert_eq!(cx.live_run(k), live);
    // (e) an agent's new candidate at the limit is refused.
    let d = stored(cx.put("a", "dx", ArtifactPayload::Diff));
    let r = cx.put(
        "a",
        "kx",
        ArtifactPayload::Candidate(Candidate {
            base: "a".repeat(40),
            diffs: vec![d],
            claim: Some(x),
            suite: Suite::Build,
        }),
    );
    assert!(matches!(
        rejected(r),
        Code::ClaimBudget | Code::ClaimNotOpen
    ));
    // (d) approve the held rerun.
    ok(cx.human(HumanAction::ClaimBudgetExtended { claim: x }));
    assert_ne!(cx.live_run(k), live);
    assert_eq!(cx.rec(x).spent(), 9);
    assert!(cx.st.claims.held.is_empty());
}

#[test]
fn cl_47_collapse_reopens_unacted_and_escalates_acted() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let c = cx.claim("c", dep_on("c", vec![a]));
    accept(&mut cx, a);
    accept(&mut cx, b);
    accept(&mut cx, c);
    let m = cx.mark(c, "pr");
    ok(cx.host(HostAction::ActionResult {
        mark: m,
        outcome: MarkOutcome::Done,
    }));
    let fail = cx.tested("c", a, false);
    ok(cx.executed("c", a, Counter, fail));
    ok(cx.executed("b", a, Counter, fail));
    assert_eq!(cx.status(a), ClaimStatus::Dropped);
    assert_eq!(cx.status(b), ClaimStatus::Void);
    assert_eq!(cx.status(c), ClaimStatus::Escalated);
}

#[test]
fn cl_48_reopen_cascades_to_a_fixed_point() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let d = cx.claim("c", dep_on("d", vec![b]));
    accept(&mut cx, a);
    accept(&mut cx, b);
    accept(&mut cx, d);
    ok(cx.decide(a, ClaimOutcome::Reopen));
    assert_eq!(cx.status(b), ClaimStatus::Open);
    assert_eq!(cx.status(d), ClaimStatus::Open);
}

#[test]
fn cl_49_supersedes_acted_needs_a_human() {
    let mut cx = Cx::new();
    let x = cx.claim(
        "a",
        ClaimBody {
            reversible: false,
            ..body("x")
        },
    );
    let r = cx.tested("c", x, true);
    for who in ["a", "b", "c"] {
        ok(cx.executed(who, x, Support, r));
    }
    ok(cx.decide(x, ClaimOutcome::Accept));
    cx.mark(x, "deploy");
    let y = cx.claim(
        "b",
        ClaimBody {
            supersedes: Some(x),
            ..body("y")
        },
    );
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    accept_any(&mut cx, y);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated { claim, reason: ClaimEscalation::SupersedesActed } if *claim == y
    )));
    ok(cx.decide(y, ClaimOutcome::Accept));
    assert_eq!(cx.status(y), ClaimStatus::Accepted);
    assert_eq!(cx.status(x), ClaimStatus::Superseded);
    assert_eq!(cx.rec(x).marks.len(), 1);
    let fail = cx.tested("c", y, false);
    let _ = fail;
    ok(cx.cited("c", x, Counter, 3));
    assert_eq!(cx.status(x), ClaimStatus::Superseded);
    assert_eq!(
        rejected(cx.claim_with(
            "c",
            ClaimBody {
                supersedes: Some(x),
                ..body("z")
            }
        )),
        Code::BadClaim
    );
}

/// Two executed supports from `a` and `b` on a passing run, whatever the outcome.
fn accept_any(cx: &mut Cx, c: MsgId) {
    let r = cx.tested("c", c, true);
    ok(cx.executed("a", c, Support, r));
    ok(cx.executed("b", c, Support, r));
}

#[test]
fn cl_50_dropped_claim_reopens_by_executed_support() {
    let mut cx = Cx::new();
    let x = cx.low();
    drop_claim(&mut cx, x);
    let pass = cx.tested("c", x, true);
    ok(cx.executed("c", x, Support, pass));
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimReopened { claim, by: ReopenCause::Evidence(_), .. } if *claim == x
    )));
}

#[test]
fn cl_51_overdue_marks_escalate() {
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let m = cx.mark(x, "pr");
    cx.now = cx.now.plus(3_600_000);
    ok(cx.run(Command::Tick));
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated { reason: ClaimEscalation::ActionUnconfirmed(mm), .. } if *mm == m
    )));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    ok(cx.human(HumanAction::ClaimActionResult {
        mark: m,
        outcome: MarkOutcome::Aborted,
    }));
    assert!(!cx.rec(x).acted_on());
    // (c) a result before the timeout: no escalation.
    let mut cx = Cx::new();
    let x = cx.low();
    accept(&mut cx, x);
    let m = cx.mark(x, "pr");
    ok(cx.host(HostAction::ActionResult {
        mark: m,
        outcome: MarkOutcome::Done,
    }));
    cx.now = cx.now.plus(3_600_000);
    ok(cx.run(Command::Tick));
    assert_eq!(cx.count(|e| matches!(e, Event::ClaimEscalated { .. })), 0);
}

#[test]
fn cl_52_source_artifacts_cite_at_weight_two() {
    let mut cx = Cx::new();
    let x = cx.low();
    let excerpt = "quoted text".to_string();
    let src = stored(cx.put(
        "a",
        "src",
        ArtifactPayload::Source(Source {
            origin: "https://example.org".into(),
            retrieved_at: "2026-11-02".into(),
            digest: Sha256::of(excerpt.as_bytes()),
            excerpt: excerpt.clone(),
        }),
    ));
    ok(cx.ev("b", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(src)]));
    assert_eq!(cx.score(x), 2);
    let r = cx.put(
        "a",
        "src2",
        ArtifactPayload::Source(Source {
            origin: "o".into(),
            retrieved_at: "t".into(),
            digest: Sha256::of(b"other"),
            excerpt,
        }),
    );
    assert_eq!(rejected(r), Code::BadClaim);
}

#[test]
fn cl_53_supersession_takes_effect_only_on_acceptance() {
    let mut cx = Cx::new();
    let x = cx.low();
    let d = cx.claim("b", dep_on("d", vec![x]));
    let y = cx.claim(
        "b",
        ClaimBody {
            supersedes: Some(x),
            ..body("y")
        },
    );
    assert_eq!(cx.status(x), ClaimStatus::Open);
    accept(&mut cx, y);
    assert_eq!(cx.status(x), ClaimStatus::Superseded);
    assert_eq!(cx.status(d), ClaimStatus::Void);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimSuperseded { claim, by, prior: ClaimStatus::Open } if *claim == x && *by == y
    )));
    // Dropped instead: X untouched.
    let mut cx = Cx::new();
    let x = cx.low();
    let y = cx.claim(
        "b",
        ClaimBody {
            supersedes: Some(x),
            ..body("y")
        },
    );
    drop_claim(&mut cx, y);
    assert_eq!(cx.status(x), ClaimStatus::Open);
}

#[test]
fn cl_54_high_stakes_accepts_at_twelve() {
    let mut cx = Cx::new();
    let x = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::High,
            ..body("x")
        },
    );
    let r = cx.tested("c", x, true);
    ok(cx.executed("a", x, Support, r));
    ok(cx.executed("b", x, Support, r));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    ok(cx.executed("c", x, Support, r));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

#[test]
fn cl_55_choose_settles_a_question_in_one_transaction() {
    let mut cx = Cx::new();
    let q = cx.low();
    let ans = |t: &str| ClaimBody {
        answers: Some(q),
        ..body(t)
    };
    let a = cx.claim("a", ans("a"));
    let b = cx.claim("b", ans("b"));
    let c = cx.claim("c", ans("c"));
    for i in 0..6 {
        let (who, dir) = if i % 2 == 0 {
            ("a", Support)
        } else {
            ("c", Counter)
        };
        ok(cx.opinion(who, b, dir));
    }
    assert_eq!(cx.status(b), ClaimStatus::Escalated);
    accept(&mut cx, c);
    cx.mark(c, "pr");
    let rev = cx.rev();
    ok(cx.human(HumanAction::Fenced {
        expected_rev: rev,
        action: Box::new(HumanAction::Choose {
            question: q,
            chosen: b,
        }),
    }));
    assert_eq!(cx.status(b), ClaimStatus::Accepted);
    assert_eq!(cx.status(a), ClaimStatus::Dropped);
    assert_eq!(cx.status(c), ClaimStatus::Dropped);
    assert_eq!(cx.rec(c).marks.len(), 1);
    let n = cx.log.len();
    let r = cx.human(HumanAction::Fenced {
        expected_rev: Rev(1),
        action: Box::new(HumanAction::Choose {
            question: q,
            chosen: a,
        }),
    });
    assert_eq!(rejected(r), Code::StaleRev);
    assert!(cx.log[n..]
        .iter()
        .all(|e| matches!(e, Event::Rejected { .. })));
}

#[test]
fn cl_56_reopened_dependent_on_its_last_unit_escalates_budget() {
    let mut cx = Cx::with(ClaimsConfig {
        settle_window: 50,
        ..ClaimsConfig::default()
    });
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    accept(&mut cx, a);
    for i in 0..5 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.opinion(who, b, Support));
    }
    accept(&mut cx, b); // 5 opinions + 1 run + 2 executed = 8
    assert_eq!(cx.rec(b).spent(), 8);
    let fail = cx.tested("c", a, false);
    ok(cx.executed("c", a, Counter, fail));
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimReopened { claim, .. } if *claim == b
    )));
    assert_eq!(cx.status(b), ClaimStatus::Escalated);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimEscalated { claim, reason: ClaimEscalation::Budget } if *claim == b
    )));
}

#[test]
fn cl_57_replacements_cannot_rest_on_what_they_retire() {
    let mut cx = Cx::new();
    let x = cx.low();
    let w = cx.claim("a", dep_on("w", vec![x]));
    assert_eq!(
        rejected(cx.claim_with(
            "b",
            ClaimBody {
                supersedes: Some(x),
                ..dep_on("z", vec![w])
            }
        )),
        Code::BadClaim
    );
    let y = cx.claim(
        "b",
        ClaimBody {
            supersedes: Some(x),
            ..body("y")
        },
    );
    accept(&mut cx, y);
    assert_eq!(
        rejected(cx.claim_with("c", dep_on("b", vec![x]))),
        Code::DepClosed
    );
}

#[test]
fn cl_58_competing_replacements_lapse() {
    let mut cx = Cx::new();
    let x = cx.low();
    let rep = |t: &str| ClaimBody {
        supersedes: Some(x),
        ..body(t)
    };
    let y1 = cx.claim("b", rep("y1"));
    let y2 = cx.claim("c", rep("y2"));
    accept(&mut cx, y1);
    assert_eq!(cx.status(x), ClaimStatus::Superseded);
    accept(&mut cx, y2);
    assert!(cx.log.iter().any(|e| matches!(
        e,
        Event::ClaimAccepted { claim, supersedes_lapsed: Some(l), .. } if *claim == y2 && *l == x
    )));
}

// ---------------------------------------------------------------- failing runs

#[test]
fn cl_59_failing_run_blocks_until_the_failed_profiles_pass() {
    let mut cx = Cx::new();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let pass = stored(cx.report_profiles(k, &[("unit", 0), ("lint", 0)], vec![]));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k, &[("unit", 1), ("lint", 0)], vec![]));
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert!(cx.rec(x).failing.contains("unit"));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k, &[("lint", 0)], vec![]));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k, &[("unit", 0), ("lint", 0)], vec![]));
    assert!(cx.rec(x).failing.is_empty());
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

#[test]
fn cl_60_only_the_current_candidate_matters() {
    let mut cx = Cx::new();
    let x = cx.low();
    let k1 = cx.candidate("a", x);
    let pass1 = stored(cx.report_profiles(k1, &[("unit", 0)], vec![]));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k1,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k1, &[("unit", 1)], vec![]));
    let k2 = cx.candidate("b", x);
    ok(cx.executed("a", x, Support, pass1));
    ok(cx.executed("b", x, Support, pass1));
    assert_eq!(cx.status(x), ClaimStatus::Open, "K2 inherits the block");
    stored(cx.report_profiles(k2, &[("lint", 0)], vec![]));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k2,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k2, &[("unit", 0)], vec![]));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k1,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k1, &[("unit", 1), ("lint", 1)], vec![]));
    assert!(cx.rec(x).failing.is_empty());
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

#[test]
fn cl_61_a_report_on_another_profile_set_is_refused() {
    let mut cx = Cx::new();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let run = cx.live_run(k);
    let secret = cx.st.secret_for(run);
    let op = cx.op();
    let log = stored(cx.run(Command::Runner {
        op,
        run,
        secret,
        blob: blob("l"),
        payload: ArtifactPayload::Log,
    }));
    let op = cx.op();
    let r = cx.run(Command::Runner {
        op,
        run,
        secret,
        blob: blob("r"),
        payload: ArtifactPayload::TestReport(Report {
            candidate: k,
            run,
            profile_digest: Sha256::of(b"other"),
            status: RunStatus::Passed,
            profiles: vec![ProfileResult {
                name: "lint".into(),
                exit_code: 0,
                failed: 0,
            }],
            tree: None,
            sandbox: "s".into(),
            log,
            tests_changed: vec![],
        }),
    });
    assert_eq!(rejected(r), Code::ProfileMismatch);
    // (c) No agent write can carry a report.
    let r = cx.put("a", "fake", ArtifactPayload::Log);
    assert_eq!(rejected(r), Code::KindForbidden);
}

fn block(cx: &mut Cx, x: MsgId) -> ArtId {
    let k = cx.candidate("a", x);
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k, &[("unit", 1)], vec![]));
    assert!(cx.rec(x).failing.contains("unit"));
    pass
}

#[test]
fn cl_62_changed_tests_do_not_clear() {
    // The runner lists every test_paths file the candidate's diffs add,
    // modify or delete (#1386, 72a8ce8e: added files count, since an added
    // conftest can skip tests). The core only reads that list.
    let mut cx = Cx::new();
    let x = cx.low();
    block(&mut cx, x);
    // (a) modifies, (b) deletes, (c) adds a test file: the block stays.
    for changed in [
        TestChange::modified("tests/core.rs"),
        TestChange::deleted("tests/core.rs"),
        TestChange::added("tests/new.rs"),
    ] {
        let k = cx.candidate("a", x);
        stored(cx.report_profiles(k, &[("unit", 0)], vec![changed.clone()]));
        assert!(cx.rec(x).failing.contains("unit"), "{changed:?}");
    }
    // (d) an added conftest keeps the block, and support citing it is refused.
    let k = cx.candidate("a", x);
    let r = stored(cx.report_profiles(
        k,
        &[("unit", 0)],
        vec![TestChange::added("tests/conftest.py")],
    ));
    assert!(cx.rec(x).failing.contains("unit"));
    assert_eq!(
        rejected(cx.executed("b", x, Support, r)),
        Code::TestsUnendorsed
    );
    // (e) no test-path change: the passing run clears.
    let k = cx.candidate("a", x);
    stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    assert!(cx.rec(x).failing.is_empty());

    // (f) no block: a candidate that only adds tests needs no endorsement,
    // one that modifies a test still does.
    let mut cx = Cx::new();
    let y = cx.low();
    let k = cx.candidate("a", y);
    let added =
        stored(cx.report_profiles(k, &[("unit", 0)], vec![TestChange::added("tests/new.rs")]));
    ok(cx.executed("b", y, Support, added));
    let k = cx.candidate("a", y);
    let modified = stored(cx.report_profiles(
        k,
        &[("unit", 0)],
        vec![TestChange::modified("tests/core.rs")],
    ));
    assert_eq!(
        rejected(cx.executed("b", y, Support, modified)),
        Code::TestsUnendorsed
    );
}

#[test]
fn cl_63_endorsement_by_another_principal_unlocks() {
    let mut cx = Cx::new();
    let x = cx.low();
    block(&mut cx, x);
    let k2 = cx.candidate("a", x);
    let r2 = stored(cx.report_profiles(
        k2,
        &[("unit", 0)],
        vec![TestChange::modified("tests/core.rs")],
    ));
    assert_eq!(
        rejected(cx.executed("b", x, Support, r2)),
        Code::TestsUnendorsed
    );
    ok(cx.ev("a", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
    assert_eq!(
        rejected(cx.executed("b", x, Support, r2)),
        Code::TestsUnendorsed
    );
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
    assert!(cx.rec(x).failing.is_empty());
    ok(cx.executed("b", x, Support, r2));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);

    // (d) K3 stored before the endorsement: no clearing.
    let mut cx = Cx::new();
    let x = cx.low();
    block(&mut cx, x);
    let k2 = cx.candidate("a", x);
    stored(cx.report_profiles(
        k2,
        &[("unit", 0)],
        vec![TestChange::modified("tests/core.rs")],
    ));
    let _k3 = cx.candidate("b", x);
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
    assert!(cx.rec(x).failing.contains("unit"));
}

// ---------------------------------------------------------------- concurrency and properties

/// CL-22: two evidence posts race through the real `Driver`; the second
/// append hits a stale revision, reloads and is judged at its commit.
#[test]
fn cl_22_racing_posts_are_judged_in_commit_order() {
    fn drive(d: &mut Driver<MemStore>, cmd: Command) -> Response {
        d.dispatch(&cmd, Time(1_000)).expect("store")
    }
    fn post(d: &Driver<MemStore>, who: &str, op: &str, draft: Draft) -> Command {
        Command::Agent {
            principal: pid(who),
            token: d.state.credential_for(&pid(who)),
            op: OpId::from(op),
            action: AgentAction::Post(draft),
        }
    }
    fn evidence(c: MsgId, kind: EvidenceKind, strength: u8, brief: ArtId) -> Draft {
        let cx = Cx::new();
        let mut d = cx.evidence_draft(c, Support, kind, strength, vec![]);
        if kind == EvidenceKind::Cited {
            d.evidence = vec![Ref::art(brief)];
        }
        d
    }
    let setup = || {
        let mut store = MemStore::new();
        let brief = store.blob_put(b"brief");
        let mut d = Driver::new(store, TaskId::from("t"));
        let mut o = open_task(Mode::Claims, None, roster());
        o.brief = brief;
        let art = stored(drive(&mut d, Command::Open(o)));
        for who in ["a", "b", "c"] {
            ok(drive(
                &mut d,
                Command::Assign {
                    role: RoleId::from(who),
                    principal: Principal {
                        id: pid(who),
                        kind: PrincipalKind::Agent {
                            role: RoleId::from(who),
                            vendor: "test".into(),
                        },
                    },
                },
            ));
        }
        let claim = Draft {
            kind: MessageKind::Claim,
            to: vec![Recipient::All],
            body: Body::Claim(body("x")),
            refs: vec![Ref::art(art)],
            evidence: vec![],
            reply_to: None,
            yield_to: None,
            gate: false,
        };
        let cmd = post(&d, "a", "claim", claim);
        let c = posted(drive(&mut d, cmd));
        (d, c, art)
    };

    // (a) The first post does not settle: both land, summed in commit order.
    let (mut d, c, brief) = setup();
    let stale = d.state.clone();
    let cmd = post(&d, "a", "e1", evidence(c, EvidenceKind::Opinion, 1, brief));
    ok(drive(&mut d, cmd));
    let second = post(&d, "b", "e2", evidence(c, EvidenceKind::Opinion, 2, brief));
    d.state = stale; // b's moderator computed against the old revision
    ok(drive(&mut d, second));
    let rec = d.state.claims.get(c).expect("claim");
    assert_eq!((rec.items.len(), rec.score()), (2, 3));

    // (b) The first post settles: the second is recorded on recompute.
    let (mut d, c, brief) = setup();
    let cmd = post(&d, "a", "e1", evidence(c, EvidenceKind::Cited, 3, brief));
    ok(drive(&mut d, cmd));
    let stale = d.state.clone();
    let cmd = post(&d, "b", "e2", evidence(c, EvidenceKind::Cited, 1, brief));
    ok(drive(&mut d, cmd));
    assert_eq!(
        d.state.claims.get(c).map(|r| r.status),
        Some(ClaimStatus::Accepted)
    );
    let third = post(&d, "c", "e3", evidence(c, EvidenceKind::Opinion, 3, brief));
    d.state = stale;
    ok(drive(&mut d, third));
    let rec = d.state.claims.get(c).expect("claim");
    assert_eq!(rec.status, ClaimStatus::Accepted);
    assert_eq!(rec.score(), 8);
    assert!(!rec.items[2].scoring, "recorded, not scored");
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum Op {
        Claim {
            deps: Vec<usize>,
        },
        Opinion {
            claim: usize,
            who: usize,
            counter: bool,
            strength: u8,
        },
        Cited {
            claim: usize,
            who: usize,
            counter: bool,
            strength: u8,
        },
        Executed {
            claim: usize,
            who: usize,
            pass: bool,
        },
        Reopen {
            claim: usize,
        },
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            1 => prop::collection::vec(0usize..8, 0..3).prop_map(|deps| Op::Claim { deps }),
            3 => (0usize..8, 0usize..3, any::<bool>(), 1u8..=3)
                .prop_map(|(claim, who, counter, strength)| Op::Opinion { claim, who, counter, strength }),
            3 => (0usize..8, 0usize..3, any::<bool>(), 1u8..=3)
                .prop_map(|(claim, who, counter, strength)| Op::Cited { claim, who, counter, strength }),
            2 => (0usize..8, 0usize..3, any::<bool>())
                .prop_map(|(claim, who, pass)| Op::Executed { claim, who, pass }),
            1 => (0usize..8).prop_map(|claim| Op::Reopen { claim }),
        ]
    }

    const WHO: [&str; 3] = ["a", "b", "c"];

    /// Run `ops`, ignoring refusals: every refusal is a rule doing its job.
    fn run(ops: &[Op]) -> Cx {
        let mut cx = Cx::new();
        let mut claims: Vec<MsgId> = vec![cx.low()];
        for op in ops {
            let pick = |i: usize| claims[i % claims.len()];
            let dir = |counter: bool| if counter { Counter } else { Support };
            match op {
                Op::Claim { deps } => {
                    let deps: Vec<MsgId> = deps.iter().map(|i| pick(*i)).collect();
                    if let Response::Posted(id) = cx.claim_with("a", dep_on("p", deps)) {
                        claims.push(id);
                    }
                }
                Op::Opinion {
                    claim,
                    who,
                    counter,
                    strength,
                } => {
                    let c = pick(*claim);
                    let _ = cx.ev(
                        WHO[*who],
                        c,
                        dir(*counter),
                        EvidenceKind::Opinion,
                        *strength,
                        vec![],
                    );
                }
                Op::Cited {
                    claim,
                    who,
                    counter,
                    strength,
                } => {
                    let _ = cx.cited(WHO[*who], pick(*claim), dir(*counter), *strength);
                }
                Op::Executed { claim, who, pass } => {
                    let c = pick(*claim);
                    let ok = cx.st.claims.get(c).is_some_and(|r| {
                        matches!(
                            r.status,
                            ClaimStatus::Open | ClaimStatus::Accepted | ClaimStatus::Dropped
                        ) && !r.exhausted(&cx.st.claims.config)
                    });
                    if ok {
                        let r = cx.tested("c", c, *pass);
                        let _ = cx.executed(WHO[*who], c, dir(!*pass), r);
                    }
                }
                Op::Reopen { claim } => {
                    let _ = cx.decide(pick(*claim), ClaimOutcome::Reopen);
                }
            }
        }
        cx
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        /// CL-24 and CL-25: the fold equals replay, twice; the budget holds;
        /// voids are closed under deps; an accepted claim rests on accepted deps.
        #[test]
        fn cl_24_25_fold_equals_replay_and_invariants_hold(ops in prop::collection::vec(op(), 1..60)) {
            let cx = run(&ops);
            let once = TaskState::fold(TaskId::from("t"), &cx.log);
            let twice = TaskState::fold(TaskId::from("t"), &cx.log);
            prop_assert_eq!(&once.claims, &cx.st.claims);
            prop_assert_eq!(&once.claims, &twice.claims);
            let book = &cx.st.claims;
            for c in book.claims.values() {
                prop_assert!(c.spent() <= c.budget(&book.config), "{} over budget", c.id);
                for d in &c.body.deps {
                    let dep = book.get(*d).expect("dep");
                    if dep.closed() {
                        prop_assert!(c.closed(), "{} is {:?} on closed {}", c.id, c.status, d);
                    }
                    if c.status == ClaimStatus::Accepted {
                        prop_assert_eq!(dep.status, ClaimStatus::Accepted);
                    }
                }
                // Recorded items never score.
                let scored: i64 = c.items.iter().filter(|i| i.scoring && i.epoch == c.epoch).map(|i| i.delta).sum();
                prop_assert_eq!(scored, c.score());
            }
        }
    }
}

/// Found by CL-24; owner decision 2026-11-02 ("stay dead"). A dropped claim
/// whose dep is dropped records passing results only, and a human reopen is
/// refused naming that dep. Once the dep is reopened, the dependent may be
/// reopened, and the passing results recorded meanwhile count in its new
/// epoch with no rerun.
#[test]
fn cl_14b_a_claim_on_a_closed_dep_stays_dead_until_the_dep_reopens() {
    let mut cx = Cx::new();
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let pass = cx.tested("c", b, true);
    drop_claim(&mut cx, b);
    drop_claim(&mut cx, a);
    ok(cx.executed("c", b, Support, pass));
    assert_eq!(cx.status(b), ClaimStatus::Dropped, "recorded, not reopened");
    assert!(cx
        .rec(b)
        .items
        .last()
        .is_some_and(|i| i.dormant && !i.scoring));
    let (code, d) = detail(cx.decide(b, ClaimOutcome::Reopen));
    assert_eq!(code, Code::DepClosed);
    assert!(d.contains(&a.to_string()) && d.contains("DROPPED"), "{d}");

    ok(cx.decide(a, ClaimOutcome::Reopen));
    ok(cx.decide(b, ClaimOutcome::Reopen));
    assert_eq!(cx.status(b), ClaimStatus::Open);
    assert_eq!(cx.rec(b).epoch, 2);
    assert_eq!(
        cx.score(b),
        4,
        "the recorded passing result counts, no rerun"
    );
    ok(cx.executed("a", b, Support, pass));
    assert_eq!(cx.status(b), ClaimStatus::Open, "waits for its dep");
    accept(&mut cx, a);
    assert_eq!(cx.status(b), ClaimStatus::Accepted);
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(replayed.claims, cx.st.claims);
}

#[test]
fn cl_64_replay_needs_the_current_credential() {
    let mut cx = Cx::new();
    let c = cx.low();
    let d = cx.evidence_draft(c, Support, EvidenceKind::Opinion, 1, vec![]);
    let op = cx.op();
    let old = cx.cred("a");
    let first = cx.agent_with("a", old, op.clone(), AgentAction::Post(d.clone()));
    posted(first.clone());
    cx.assign("a"); // rotates a's credential
    for t in [old, None] {
        assert_eq!(
            rejected(cx.agent_with("a", t, op.clone(), AgentAction::Post(d.clone()))),
            Code::BadCredential
        );
    }
    let now = cx.cred("a");
    assert_eq!(cx.agent_with("a", now, op, AgentAction::Post(d)), first);
}

#[test]
fn cl_65_human_posts_are_not_charged_to_the_task_budget() {
    let mut cx = Cx::new();
    let mut o = open_task(Mode::Claims, None, roster());
    o.budget.messages = 1;
    cx.st = TaskState::new(TaskId::from("t"));
    cx.log.clear();
    let r = cx.run(Command::Open(o));
    cx.brief = stored(r);
    for who in ["a", "b", "c", "s"] {
        cx.assign(who);
    }
    let _ = cx.low(); // spends the whole message budget
    let d = Draft {
        kind: MessageKind::Claim,
        to: vec![Recipient::All],
        body: Body::Claim(body("human")),
        refs: vec![Ref::art(cx.brief)],
        evidence: vec![],
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    let charged = cx.count(|e| matches!(e, Event::Charged { .. }));
    let h = posted(cx.human(HumanAction::Post(d)));
    ok(cx.human(HumanAction::Post(cx.evidence_draft(
        h,
        Support,
        EvidenceKind::Opinion,
        1,
        vec![],
    ))));
    assert_eq!(cx.count(|e| matches!(e, Event::Charged { .. })), charged);
    assert_eq!(cx.st.state, State::Open);
}

#[test]
fn cl_66_held_rerun_on_a_voided_claim_is_withdrawn_not_started() {
    let mut cx = Cx::with(ClaimsConfig {
        settle_window: 50,
        ..ClaimsConfig::default()
    });
    let a = cx.low();
    let b = cx.claim("b", dep_on("b", vec![a]));
    let k = cx.candidate("a", b); // 1 unit
    for i in 0..7 {
        let who = if i % 2 == 0 { "a" } else { "b" };
        ok(cx.opinion(who, b, Support));
    } // 8 units: exhausted
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    assert!(cx.st.claims.held.contains_key(&k));
    drop_claim(&mut cx, a);
    assert_eq!(cx.status(b), ClaimStatus::Void);
    let live = cx.live_run(k);
    let started = cx.count(|e| matches!(e, Event::RunStarted { .. }));
    ok(cx.human(HumanAction::ClaimBudgetExtended { claim: b }));
    assert_eq!(cx.count(|e| matches!(e, Event::RunStarted { .. })), started);
    assert_eq!(cx.live_run(k), live);
    assert_eq!(cx.status(b), ClaimStatus::Void);
    assert!(cx.st.claims.held.is_empty());
    assert!(cx
        .log
        .iter()
        .any(|e| matches!(e, Event::RerunWithdrawn { candidate } if *candidate == k)));
}

#[test]
fn cl_67_task_end_invalidates_session_credentials() {
    for close in [true, false] {
        let mut cx = Cx::new();
        let c = cx.low();
        let op = cx.op();
        let d = cx.evidence_draft(c, Support, EvidenceKind::Opinion, 1, vec![]);
        let saved = cx.cred("a");
        posted(cx.agent_with("a", saved, op.clone(), AgentAction::Post(d.clone())));
        let rev = cx.rev();
        ok(cx.human(if close {
            HumanAction::Close { expected_rev: rev }
        } else {
            HumanAction::Cancel {
                expected_rev: rev,
                reason: "stop".into(),
            }
        }));
        assert!(cx.st.state.terminal());
        for view in [
            AgentAction::ClaimCard { claim: c },
            AgentAction::ClaimLine { claim: c },
            AgentAction::ClaimsDelta {
                after: Rev(0),
                domains: vec![],
            },
        ] {
            let op = cx.op();
            assert_eq!(
                rejected(cx.agent_with("a", saved, op, view)),
                Code::BadCredential
            );
        }
        // The credential is gone, so its cached responses are too.
        assert_eq!(
            rejected(cx.agent_with("a", saved, op, AgentAction::Post(d))),
            Code::BadCredential
        );
        // The human's own inspection is unaffected.
        let card = rusty_bbp::claims::card(&cx.st, cx.rec(c), &rusty_bbp::claims::Sight::all());
        assert_eq!(card.id, c);
    }
}

fn card_all(st: &TaskState, c: MsgId) -> rusty_bbp::claims::ClaimView {
    rusty_bbp::claims::card(st, &st.claims.claims[&c], &rusty_bbp::claims::Sight::all())
}

#[test]
fn cl_68_card_carries_the_decision_facts_and_replays() {
    use rusty_bbp::claims::FailingView;
    // (a) sole_candidate is on the card, not only on the event.
    let mut cx = Cx::new();
    let q = cx.low();
    let x = cx.claim(
        "a",
        ClaimBody {
            answers: Some(q),
            ..body("x")
        },
    );
    accept(&mut cx, x);
    assert!(card_all(&cx.st, x).sole_candidate);
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(card_all(&replayed, x), card_all(&cx.st, x));

    // (b) a block names the report that set it; the override shows the test
    // change and its endorsement.
    let mut cx = Cx::new();
    let x = cx.low();
    let k = cx.candidate("a", x);
    stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    let fail = stored(cx.report_profiles(k, &[("unit", 1)], vec![]));
    let v = card_all(&cx.st, x);
    assert_eq!(v.current, Some(k));
    assert_eq!(
        v.failing,
        vec![FailingView {
            profile: "unit".into(),
            report: Some(fail),
        }]
    );
    assert!(v.tests_changed.is_empty());
    assert_eq!(v.endorsement, None);
    let k2 = cx.candidate("a", x);
    let changed = TestChange::modified("tests/core.rs");
    stored(cx.report_profiles(k2, &[("unit", 0)], vec![changed.clone()]));
    let v = card_all(&cx.st, x);
    assert_eq!(v.current, Some(k2));
    assert_eq!(v.tests_changed, vec![changed.clone()]);
    assert_eq!(v.endorsement, None);
    assert_eq!(v.failing.len(), 1);
    let e = posted(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
    let v = card_all(&cx.st, x);
    assert!(v.failing.is_empty());
    assert_eq!(v.endorsement, Some(e));
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(card_all(&replayed, x), v);

    // (c) a reader without candidate/report access sees none of it.
    let mut o = open_task(Mode::Claims, None, roster());
    o.roster
        .push(Role::new("n", 4).post(&[MessageKind::Evidence]));
    cx.st = TaskState::new(TaskId::from("t"));
    cx.log.clear();
    cx.brief = stored(cx.run(Command::Open(o)));
    for who in ["a", "b", "c", "s", "n"] {
        cx.assign(who);
    }
    let x = cx.low();
    let k = cx.candidate("a", x);
    stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let rev = cx.rev();
    ok(cx.human(HumanAction::Rerun {
        candidate: k,
        expected_rev: rev,
    }));
    stored(cx.report_profiles(k, &[("unit", 1)], vec![changed]));
    let Response::Claims(vs) = cx.agent("n", AgentAction::ClaimCard { claim: x }) else {
        panic!("card")
    };
    assert_eq!(vs[0].current, None);
    assert_eq!(
        vs[0].failing,
        vec![FailingView {
            profile: "unit".into(),
            report: None,
        }]
    );
    assert!(vs[0].tests_changed.is_empty());
}

#[test]
fn cl_69_unrepresentable_budgets_are_refused_at_open() {
    let open = |base: u64| {
        let cfg = ClaimsConfig {
            evidence_base: base,
            ..ClaimsConfig::default()
        };
        handle(
            &TaskState::new(TaskId::from("t")),
            &Command::Open(open_task(Mode::Claims, Some(cfg), roster())),
            Time(0),
        )
        .response
    };
    for base in [ClaimsConfig::MAX_EVIDENCE_BASE + 1, 1 << 63, u64::MAX] {
        let (code, d) = detail(open(base));
        assert_eq!(code, Code::BadClaimsConfig);
        assert!(d.contains("evidence_base"), "{d}");
    }
    // The largest base still gives every claim a representable budget.
    let mut cx = Cx::with(ClaimsConfig {
        evidence_base: ClaimsConfig::MAX_EVIDENCE_BASE,
        ..ClaimsConfig::default()
    });
    let x = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::High,
            reversible: false,
            ..body("x")
        },
    );
    ok(cx.human(HumanAction::ClaimBudgetExtended { claim: x }));
    assert_eq!(
        cx.rec(x).budget(&cx.st.claims.config),
        ClaimsConfig::MAX_EVIDENCE_BASE * 4 * 2 * 2
    );
}

/// Owner, 2026-11-02: a superseded dep is treated exactly like a dropped one.
/// Unacted dependents are voided; acted-on dependents escalate and are never
/// auto-voided, whatever their own status.
#[test]
fn cl_70_superseded_dep_collapses_like_a_dropped_one() {
    fn supersede(cx: &mut Cx, x: MsgId) {
        let y = cx.claim(
            "b",
            ClaimBody {
                supersedes: Some(x),
                ..body("y")
            },
        );
        accept(cx, y);
        assert_eq!(cx.status(x), ClaimStatus::Superseded);
    }
    let collapsed = |cx: &Cx, b: MsgId| {
        cx.log.iter().any(|e| {
            matches!(e, Event::ClaimEscalated { claim, reason: ClaimEscalation::DepCollapsed } if *claim == b)
        })
    };
    for superseded in [true, false] {
        let fall = |cx: &mut Cx, x: MsgId| {
            if superseded {
                supersede(cx, x)
            } else {
                ok(cx.decide(x, ClaimOutcome::Drop))
            }
        };
        // (a) unacted accepted dependent: voided.
        let mut cx = Cx::new();
        let x = cx.low();
        let b = cx.claim("b", dep_on("b", vec![x]));
        accept(&mut cx, x);
        accept(&mut cx, b);
        fall(&mut cx, x);
        assert_eq!(cx.status(b), ClaimStatus::Void, "superseded={superseded}");
        // (b) acted-on accepted dependent: escalates.
        let mut cx = Cx::new();
        let x = cx.low();
        let b = cx.claim("b", dep_on("b", vec![x]));
        accept(&mut cx, x);
        accept(&mut cx, b);
        cx.mark(b, "pr-1");
        fall(&mut cx, x);
        assert_eq!(
            cx.status(b),
            ClaimStatus::Escalated,
            "superseded={superseded}"
        );
        assert!(collapsed(&cx, b));
        // (c) acted-on dependent the human reopened: escalates, not voided.
        let mut cx = Cx::new();
        let x = cx.low();
        let b = cx.claim("b", dep_on("b", vec![x]));
        accept(&mut cx, x);
        accept(&mut cx, b);
        cx.mark(b, "pr-1");
        ok(cx.decide(b, ClaimOutcome::Reopen));
        assert_eq!(cx.status(b), ClaimStatus::Open);
        fall(&mut cx, x);
        assert_eq!(
            cx.status(b),
            ClaimStatus::Escalated,
            "superseded={superseded}"
        );
        assert!(collapsed(&cx, b));
    }
}

// ---------------------------------------------------------------- holdouts (#597)

fn holdout_digest() -> Sha256 {
    Sha256::of(b"holdout suite")
}

/// A claims task whose roster adds the holdout lens `h`, opened with a
/// holdout suite.
fn holdout_cx() -> Cx {
    holdout_cx_with(Some(holdout_digest()))
}

fn holdout_cx_with(digest: Option<Sha256>) -> Cx {
    let mut cx = Cx::new();
    let mut roles = roster();
    roles.push(lens("h", 4).holdout());
    let mut o = open_task(Mode::Claims, None, roles);
    o.holdout_digest = digest;
    cx.st = TaskState::new(TaskId::from("t"));
    cx.log.clear();
    cx.brief = stored(cx.run(Command::Open(o)));
    for who in ["a", "b", "c", "s", "h"] {
        cx.assign(who);
    }
    cx
}

impl Cx {
    fn manifest(&self, k: ArtId) -> Candidate {
        match &self.st.artifacts[&k].payload {
            ArtifactPayload::Candidate(c) => c.clone(),
            other => panic!("not a candidate: {other:?}"),
        }
    }

    /// `who` stores a holdout candidate with `k`'s manifest.
    fn holdout_of(&mut self, who: &str, k: ArtId) -> Response {
        let m = Candidate {
            suite: Suite::Holdout,
            ..self.manifest(k)
        };
        self.put(who, "holdout", ArtifactPayload::Candidate(m))
    }

    /// The holdout lens re-runs `k`'s code; its run reports `profiles`.
    fn holdout_run(&mut self, k: ArtId, profiles: &[(&str, u32)]) -> Response {
        let h = stored(self.holdout_of("h", k));
        self.report_under(h, holdout_digest(), profiles)
    }
}

#[test]
fn cl_71_holdout_candidates_are_gated() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    // (a) Only a role with write:holdout stores one.
    let (code, why) = detail(cx.holdout_of("a", k));
    assert_eq!(code, Code::KindForbidden);
    assert!(why.contains("write:holdout"), "{why}");
    // (b) It must build exactly the current code.
    let other = Candidate {
        base: "b".repeat(40),
        suite: Suite::Holdout,
        ..cx.manifest(k)
    };
    let r = cx.put("h", "h1", ArtifactPayload::Candidate(other));
    assert_eq!(rejected(r), Code::HoldoutMismatch);
    // (c) A valid one debits a run and leaves current(c) alone.
    let runs = cx.rec(x).runs;
    let h = stored(cx.holdout_of("h", k));
    assert_eq!(cx.rec(x).runs, runs + 1);
    assert_eq!(cx.rec(x).current, Some(k));
    assert!(cx.st.claims.live_run(h).is_some());
    // (d) A task without a holdout suite refuses one, and its accept line
    // is unchanged.
    let mut plain = holdout_cx_with(None);
    let y = plain.low();
    let k = plain.candidate("a", y);
    assert_eq!(rejected(plain.holdout_of("h", k)), Code::HoldoutMismatch);
    let pass = stored(plain.report_profiles(k, &[("unit", 0)], vec![]));
    ok(plain.executed("a", y, Support, pass));
    ok(plain.executed("b", y, Support, pass));
    assert_eq!(plain.status(y), ClaimStatus::Accepted);
}

#[test]
fn cl_72_accept_waits_for_a_holdout_pass() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass));
    assert_eq!(cx.status(x), ClaimStatus::Open, "no holdout pass yet");
    assert!(card_of(&cx, x).holdout_due);
    stored(cx.holdout_run(k, &[("holdout:api", 0)]));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    assert!(!card_of(&cx, x).holdout_due);
    // A claim with no candidate (research, design) is unaffected.
    let y = cx.low();
    ok(cx.cited("a", y, Support, 2));
    ok(cx.cited("b", y, Support, 2));
    assert_eq!(cx.status(y), ClaimStatus::Accepted);
}

fn card_of(cx: &Cx, c: MsgId) -> ClaimView {
    rusty_bbp::claims::card(&cx.st, cx.rec(c), &rusty_bbp::claims::Sight::all())
}

#[test]
fn cl_73_a_holdout_failure_blocks_until_a_holdout_pass_lists_it() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let fail = stored(cx.holdout_run(k, &[("holdout:api", 2)]));
    assert_eq!(
        cx.rec(x).failing.iter().cloned().collect::<Vec<_>>(),
        ["holdout:api"]
    );
    assert_eq!(card_of(&cx, x).failing[0].report, Some(fail));
    // A new builder candidate inherits the block; its build pass cannot
    // clear a holdout profile.
    let k2 = cx.candidate("b", x);
    let pass2 = stored(cx.report_profiles(k2, &[("unit", 0)], vec![]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass2));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    // A holdout pass on the new code that lists the profile clears it.
    stored(cx.holdout_run(k2, &[("holdout:api", 0)]));
    assert!(cx.rec(x).failing.is_empty());
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

#[test]
fn cl_74_reports_bind_to_their_suite() {
    let mut cx = holdout_cx_with(Some(holdout_digest()));
    let x = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::High,
            ..body("x")
        },
    );
    let build = cx.st.profile_digest;
    // A build run reporting under the holdout digest, or a holdout profile.
    for (digest, profile, code) in [
        (holdout_digest(), "unit", Code::ProfileMismatch),
        (build, "holdout:api", Code::InconsistentReport),
        (build, "golden:cases", Code::InconsistentReport),
    ] {
        let k = cx.candidate("a", x);
        assert_eq!(rejected(cx.report_under(k, digest, &[(profile, 0)])), code);
    }
    // A holdout run reporting under the build digest, or a build profile.
    let k = cx.candidate("a", x);
    for (digest, profile, code) in [
        (build, "holdout:api", Code::ProfileMismatch),
        (holdout_digest(), "unit", Code::InconsistentReport),
    ] {
        let h = stored(cx.holdout_of("h", k));
        assert_eq!(rejected(cx.report_under(h, digest, &[(profile, 0)])), code);
    }
    let h = stored(cx.holdout_of("h", k));
    stored(cx.report_under(h, holdout_digest(), &[("golden:cases", 0)]));
}

#[test]
fn cl_75_holdout_runs_on_replaced_code_change_nothing() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    let h = stored(cx.holdout_of("h", k));
    let k2 = cx.candidate("b", x);
    // The holdout of K1 reports after K2 became current: no block, no pass.
    stored(cx.report_under(h, holdout_digest(), &[("holdout:api", 1)]));
    assert!(cx.rec(x).failing.is_empty());
    ok(cx.executed("a", x, Support, pass));
    let pass2 = stored(cx.report_profiles(k2, &[("unit", 0)], vec![]));
    ok(cx.executed("b", x, Support, pass2));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    // A human accept still overrides.
    ok(cx.decide(x, ClaimOutcome::Accept));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

#[test]
fn cl_76_holdout_suite_needs_claims_mode_and_replays() {
    let mut o = open_task(Mode::Stages, None, stage_roster());
    o.holdout_digest = Some(holdout_digest());
    let h = handle(
        &TaskState::new(TaskId::from("t")),
        &Command::Open(o),
        Time(1),
    );
    assert!(matches!(
        h.response,
        Response::Rejected(Rejection {
            code: Code::BadClaimsConfig,
            ..
        })
    ));
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    stored(cx.holdout_run(k, &[("holdout:api", 0)]));
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(replayed.claims, cx.st.claims);
    assert_eq!(replayed.claims.holdout_digest, Some(holdout_digest()));
}

#[test]
fn cl_77_suite_versions_need_a_fresh_holdout_pass() {
    let v2 = Sha256::of(b"holdout suite v2");
    let v3 = Sha256::of(b"holdout suite v3");
    let mut cx = holdout_cx();
    // Accepted on v1.
    let x = cx.low();
    let kx = cx.candidate("a", x);
    let px = stored(cx.report_profiles(kx, &[("unit", 0)], vec![]));
    stored(cx.holdout_run(kx, &[("holdout:api", 0)]));
    ok(cx.executed("a", x, Support, px));
    ok(cx.executed("b", x, Support, px));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    // Open, passed on v1 but short of support; another open claim blocked by
    // a v1 holdout failure.
    let y = cx.low();
    let ky = cx.candidate("a", y);
    let py = stored(cx.report_profiles(ky, &[("unit", 0)], vec![]));
    stored(cx.holdout_run(ky, &[("holdout:api", 0)]));
    let z = cx.low();
    let kz = cx.candidate("a", z);
    stored(cx.holdout_run(kz, &[("holdout:api", 1)]));
    assert!(!cx.rec(z).failing.is_empty());

    // (a) The host adopts a pure addition.
    ok(cx.host(HostAction::HoldoutExtended { digest: v2 }));
    assert_eq!(cx.st.claims.holdout_digest, Some(v2));
    assert_eq!(
        cx.status(x),
        ClaimStatus::Accepted,
        "accepted stays accepted"
    );
    assert!(cx.rec(z).failing.is_empty(), "v1 holdout failures retire");
    // Y's v1 pass no longer counts: support alone does not accept it.
    ok(cx.executed("a", y, Support, py));
    ok(cx.executed("b", y, Support, py));
    assert_eq!(cx.status(y), ClaimStatus::Open);
    assert!(card_of(&cx, y).holdout_due);
    // A report under the old digest is refused; a v2 pass accepts.
    let h = stored(cx.holdout_of("h", ky));
    assert_eq!(
        rejected(cx.report_under(h, holdout_digest(), &[("holdout:api", 0)])),
        Code::ProfileMismatch
    );
    let h = stored(cx.holdout_of("h", ky));
    stored(cx.report_under(h, v2, &[("holdout:api", 0), ("holdout:new", 0)]));
    assert_eq!(cx.status(y), ClaimStatus::Accepted);

    // (b) A change to existing tests is the human's: the same digest twice
    // is a duplicate, an agent has no such write, the human adopts it.
    assert_eq!(
        rejected(cx.host(HostAction::HoldoutExtended { digest: v2 })),
        Code::Duplicate
    );
    ok(cx.human(HumanAction::HoldoutUpdated { digest: v3 }));
    assert_eq!(cx.st.claims.holdout_digest, Some(v3));
    assert_eq!(
        cx.count(|e| matches!(e, Event::HoldoutUpdated { digest } if *digest == v3)),
        1
    );
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(replayed.claims, cx.st.claims);

    // (c) A task without a holdout suite has no versions.
    let mut plain = holdout_cx_with(None);
    assert_eq!(
        rejected(plain.host(HostAction::HoldoutExtended { digest: v2 })),
        Code::HoldoutMismatch
    );
    assert_eq!(
        rejected(plain.human(HumanAction::HoldoutUpdated { digest: v2 })),
        Code::HoldoutMismatch
    );
}

/// CL-81: the human's `answer` (Refine, #595) refs the brief, replies to a
/// claim or to no message, needs no turn, is not charged, changes no claim,
/// and is taken while the task is escalated. An agent's `answer` stays refused.
#[test]
fn cl_81_human_answer_clarifies_the_intent() {
    let mut o = open_task(Mode::Claims, None, roster());
    o.budget.messages = 2;
    let mut cx = Cx::new();
    cx.st = TaskState::new(TaskId::from("t"));
    cx.log.clear();
    cx.brief = stored(cx.run(Command::Open(o)));
    for who in ["a", "b", "c", "s"] {
        cx.assign(who);
    }
    let c = cx.low();
    let answer = |brief: Option<ArtId>, to: Option<MsgId>| {
        let d = Draft::new(MessageKind::Answer, "prefer the smaller change")
            .refs(brief.map(Ref::art).into_iter().collect());
        HumanAction::Post(match to {
            Some(m) => d.reply_to(m),
            None => d,
        })
    };
    let (status, spent) = (cx.status(c), cx.st.spent.messages);
    let a = posted(cx.human(answer(Some(cx.brief), Some(c))));
    assert_eq!(cx.st.message(a).map(|m| m.from_role.clone()), Some(None));
    assert_eq!(
        (cx.status(c), cx.st.spent.messages),
        (status, spent),
        "no claim change, not charged"
    );
    assert_eq!(
        rejected(cx.human(answer(None, Some(c)))),
        Code::RefsRequired
    );
    let e = posted(cx.opinion("a", c, Support));
    assert_eq!(
        rejected(cx.human(answer(Some(cx.brief), Some(e)))),
        Code::WrongKind
    );
    let agent = Draft::new(MessageKind::Answer, "x").refs(vec![Ref::art(cx.brief)]);
    assert_eq!(
        rejected(cx.agent("a", AgentAction::Post(agent))),
        Code::WrongMode
    );
    // Escalated on a task total: the human may still clarify the task.
    ok(cx.opinion("b", c, Support));
    assert_eq!(cx.st.state, State::Escalated);
    posted(cx.human(answer(Some(cx.brief), None)));
}

#[test]
fn cl_78_a_tests_claim_needs_support_from_outside_its_authors() {
    let mut cx = Cx::new();
    let x = cx.claim(
        "a",
        ClaimBody {
            domains: vec![rusty_bbp::claims::TESTS_DOMAIN.into()],
            ..body("fix the wrong assertion in tests/core.rs")
        },
    );
    // `b` writes the diff, `a` the candidate: both are its authors.
    let diff = stored(cx.put("b", "test-diff", ArtifactPayload::Diff));
    let k = stored(cx.put(
        "a",
        "test-cand",
        ArtifactPayload::Candidate(Candidate {
            base: "a".repeat(40),
            diffs: vec![diff],
            claim: Some(x),
            suite: Suite::Build,
        }),
    ));
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass));
    assert_eq!(
        cx.status(x),
        ClaimStatus::Open,
        "only its authors support it"
    );
    ok(cx.opinion("c", x, Support));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
    // Any other claim accepts on its authors' support as before.
    let y = cx.low();
    let r = cx.tested("a", y, true);
    ok(cx.executed("a", y, Support, r));
    ok(cx.executed("b", y, Support, r));
    assert_eq!(cx.status(y), ClaimStatus::Accepted);
}

/// #597 review: a manifest names its base by any ref, so a holdout pass
/// counts only on the tree the current candidate's latest report tested.
#[test]
fn cl_91_a_holdout_pass_counts_only_on_the_tree_the_build_tested() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    cx.tree = Some(Sha256::of(b"tree-1"));
    let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
    ok(cx.executed("a", x, Support, pass));
    ok(cx.executed("b", x, Support, pass));
    // The base ref moved between the runs: the holdout built another tree.
    cx.tree = Some(Sha256::of(b"tree-2"));
    stored(cx.holdout_run(k, &[("holdout:api", 0)]));
    assert_eq!(cx.status(x), ClaimStatus::Open);
    assert!(card_of(&cx, x).holdout_due);
    cx.tree = Some(Sha256::of(b"tree-1"));
    stored(cx.holdout_run(k, &[("holdout:api", 0)]));
    assert_eq!(cx.status(x), ClaimStatus::Accepted);
}

/// #597 review: the endorsement of a candidate's test change also covers
/// the holdout candidates building its code, whichever comes first.
#[test]
fn cl_92_an_endorsement_unlocks_holdout_reports_of_the_same_code() {
    let changed = || vec![TestChange::modified("tests/core.rs")];
    for endorse_first in [false, true] {
        let mut cx = holdout_cx();
        let x = cx.low();
        let k = cx.candidate("a", x);
        stored(cx.holdout_run(k, &[("holdout:api", 1)]));
        assert!(cx.rec(x).failing.contains("holdout:api"));
        let k2 = cx.candidate("a", x);
        // The build report names the tree a holdout pass must match.
        stored(cx.report_profiles(k2, &[("unit", 0)], vec![]));
        if endorse_first {
            ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
        }
        let h = stored(cx.holdout_of("h", k2));
        let run = cx.live_run(h);
        let secret = cx.st.secret_for(run);
        stored(cx.report_full(
            h,
            run,
            secret,
            holdout_digest(),
            &[("holdout:api", 0)],
            changed(),
        ));
        assert_eq!(
            cx.rec(x).failing.contains("holdout:api"),
            !endorse_first,
            "endorse first: {endorse_first}"
        );
        if !endorse_first {
            ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k2)]));
            assert!(cx.rec(x).failing.is_empty());
        }
    }
}

/// #597 review: a suite version makes every claim with a candidate due
/// again, so the delta shows them; a retired suite is never adopted again,
/// or its old passes would count once more.
#[test]
fn cl_93_suite_versions_reach_the_delta_and_never_return() {
    let v2 = Sha256::of(b"holdout suite v2");
    let mut cx = holdout_cx();
    let y = cx.low();
    let ky = cx.candidate("a", y);
    stored(cx.report_profiles(ky, &[("unit", 0)], vec![]));
    stored(cx.holdout_run(ky, &[("holdout:api", 0)]));
    let bare = cx.low();
    let before = cx.st.rev;
    ok(cx.host(HostAction::HoldoutExtended { digest: v2 }));
    let delta = rusty_bbp::claims::delta_ids(
        &cx.st.claims,
        before,
        &["http".into()],
        &rusty_bbp::claims::Sight::all(),
    );
    assert!(delta.contains(&y), "{delta:?}");
    assert!(
        !delta.contains(&bare),
        "no candidate, nothing due: {delta:?}"
    );
    // Back to v1, by the host or the human: refused.
    let v1 = holdout_digest();
    assert_eq!(
        rejected(cx.host(HostAction::HoldoutExtended { digest: v1 })),
        Code::Duplicate
    );
    assert_eq!(
        rejected(cx.human(HumanAction::HoldoutUpdated { digest: v1 })),
        Code::Duplicate
    );
    assert_eq!(cx.st.claims.holdout_digest, Some(v2));
    let replayed = TaskState::fold(TaskId::from("t"), &cx.log);
    assert_eq!(replayed.claims, cx.st.claims);
}

/// #597 review: a host rule passed to `dispatch_guarded` is judged against
/// the state the commit lands on: after a revision conflict it is judged
/// again on the reloaded state, and its refusal leaves the log alone.
#[test]
fn cl_94_a_dispatch_guard_is_rejudged_after_a_reload() {
    let mut store = MemStore::new();
    let brief = store.blob_put(b"brief");
    let mut d = Driver::new(store, TaskId::from("t"));
    let mut o = open_task(Mode::Claims, None, roster());
    o.brief = brief;
    let art = stored(d.dispatch(&Command::Open(o), Time(1_000)).expect("store"));
    for who in ["a", "b"] {
        let assign = Command::Assign {
            role: RoleId::from(who),
            principal: Principal {
                id: pid(who),
                kind: PrincipalKind::Agent {
                    role: RoleId::from(who),
                    vendor: "test".into(),
                },
            },
        };
        ok(d.dispatch(&assign, Time(1_000)).expect("store"));
    }
    let post = |d: &Driver<MemStore>, who: &str, op: &str, text: &str| Command::Agent {
        principal: pid(who),
        token: d.state.credential_for(&pid(who)),
        op: OpId::from(op),
        action: AgentAction::Post(Draft {
            kind: MessageKind::Claim,
            to: vec![Recipient::All],
            body: Body::Claim(body(text)),
            refs: vec![Ref::art(art)],
            evidence: vec![],
            reply_to: None,
            yield_to: None,
            gate: false,
        }),
    };
    let stale = d.state.clone();
    let first = post(&d, "a", "x", "x");
    posted(d.dispatch(&first, Time(1_000)).expect("store"));
    let committed = d.state.rev;
    // b's server read the old revision; the rule only fails on the new one.
    d.state = stale.clone();
    let second = post(&d, "b", "y", "y");
    let r = d
        .dispatch_guarded(&second, Time(1_000), |st| {
            (st.rev > stale.rev).then(|| Rejection::new(Code::ProtectedPath, "changed"))
        })
        .expect("store");
    assert_eq!(rejected(r), Code::ProtectedPath);
    assert_eq!(d.state.rev, committed, "nothing appended");
    // A guard that holds on the reloaded state lets the command through.
    d.state = stale;
    posted(
        d.dispatch_guarded(&second, Time(1_000), |_| None)
            .expect("store"),
    );
}

/// #597 re-review: a report without a tree proves no tree, so a holdout pass
/// counts only when both it and the build report name one.
#[test]
fn cl_95_a_holdout_pass_needs_a_tree_on_both_sides() {
    for (build, holdout) in [
        (None, None),
        (Some(b"t".as_slice()), None),
        (None, Some(b"t".as_slice())),
    ] {
        let mut cx = holdout_cx();
        let x = cx.low();
        let k = cx.candidate("a", x);
        cx.tree = build.map(Sha256::of);
        let pass = stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
        ok(cx.executed("a", x, Support, pass));
        ok(cx.executed("b", x, Support, pass));
        cx.tree = holdout.map(Sha256::of);
        stored(cx.holdout_run(k, &[("holdout:api", 0)]));
        assert_eq!(cx.status(x), ClaimStatus::Open, "{build:?} {holdout:?}");
        assert!(card_of(&cx, x).holdout_due);
    }
}

/// #597 re-review: an endorsement unlocks only a pass newer than the failure
/// that blocks the profile, and a later holdout failure supersedes a pass.
#[test]
fn cl_96_a_pass_older_than_the_block_does_not_clear_it() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let h1 = stored(cx.holdout_of("h", k));
    let run = cx.live_run(h1);
    let secret = cx.st.secret_for(run);
    let changed = vec![TestChange::modified("tests/core.rs")];
    stored(cx.report_full(
        h1,
        run,
        secret,
        holdout_digest(),
        &[("holdout:api", 0)],
        changed,
    ));
    stored(cx.holdout_run(k, &[("holdout:api", 1)]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k)]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    assert!(card_of(&cx, x).holdout_due);
}

/// #597 re-review: the current candidate's endorsement also covers executed
/// support citing a holdout report of the same code.
#[test]
fn cl_97_executed_holdout_evidence_uses_the_code_endorsement() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k)]));
    let h = stored(cx.holdout_of("h", k));
    let run = cx.live_run(h);
    let secret = cx.st.secret_for(run);
    let changed = vec![TestChange::modified("tests/core.rs")];
    let rep = stored(cx.report_full(
        h,
        run,
        secret,
        holdout_digest(),
        &[("holdout:api", 0)],
        changed,
    ));
    ok(cx.executed("a", x, Support, rep));
}

/// #597 re-review: after a suite version an endorsement does not unlock a
/// holdout pass reported under the retired suite.
#[test]
fn cl_98_an_endorsement_ignores_passes_under_a_retired_suite() {
    let v2 = Sha256::of(b"holdout suite v2");
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    let h = stored(cx.holdout_of("h", k));
    let run = cx.live_run(h);
    let secret = cx.st.secret_for(run);
    let changed = vec![TestChange::modified("tests/core.rs")];
    stored(cx.report_full(
        h,
        run,
        secret,
        holdout_digest(),
        &[("holdout:api", 0)],
        changed,
    ));
    ok(cx.host(HostAction::HoldoutExtended { digest: v2 }));
    let h2 = stored(cx.holdout_of("h", k));
    stored(cx.report_under(h2, v2, &[("holdout:api", 1)]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k)]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    assert!(card_of(&cx, x).holdout_due);
}

/// #597 re-review: `failing_from` keeps the first failure, so an endorsement
/// compares a pass with the latest failure of the profile: fail, pass with a
/// test change, fail again, endorse leaves the block.
#[test]
fn cl_99_an_endorsement_does_not_clear_a_profile_that_failed_again() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let k = cx.candidate("a", x);
    stored(cx.holdout_run(k, &[("holdout:api", 1)]));
    let h1 = stored(cx.holdout_of("h", k));
    let run = cx.live_run(h1);
    let secret = cx.st.secret_for(run);
    let changed = vec![TestChange::modified("tests/core.rs")];
    stored(cx.report_full(
        h1,
        run,
        secret,
        holdout_digest(),
        &[("holdout:api", 0)],
        changed,
    ));
    stored(cx.holdout_run(k, &[("holdout:api", 1)]));
    ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k)]));
    assert!(cx.rec(x).failing.contains("holdout:api"));
    assert!(card_of(&cx, x).holdout_due);
}

/// #597 re-review: a holdout pass on another tree than the build tested
/// clears no failure, neither when it arrives nor at an endorsement.
#[test]
fn cl_100_a_holdout_pass_on_another_tree_clears_no_failure() {
    let tree1 = Some(Sha256::of(b"tree-1"));
    let tree2 = Some(Sha256::of(b"tree-2"));
    for endorsed in [false, true] {
        let mut cx = holdout_cx();
        let x = cx.low();
        let k = cx.candidate("a", x);
        cx.tree = tree1;
        stored(cx.report_profiles(k, &[("unit", 0)], vec![]));
        stored(cx.holdout_run(k, &[("holdout:api", 1)]));
        cx.tree = tree2;
        let h = stored(cx.holdout_of("h", k));
        let run = cx.live_run(h);
        let secret = cx.st.secret_for(run);
        let changed = match endorsed {
            true => vec![TestChange::modified("tests/core.rs")],
            false => vec![],
        };
        stored(cx.report_full(
            h,
            run,
            secret,
            holdout_digest(),
            &[("holdout:api", 0)],
            changed,
        ));
        if endorsed {
            ok(cx.ev("c", x, Support, EvidenceKind::Cited, 1, vec![Ref::art(k)]));
        }
        assert!(
            cx.rec(x).failing.contains("holdout:api"),
            "endorsed: {endorsed}"
        );
        // The same pass on the tested tree clears it.
        cx.tree = tree1;
        stored(cx.holdout_run(k, &[("holdout:api", 0)]));
        assert!(cx.rec(x).failing.is_empty(), "endorsed: {endorsed}");
    }
}

/// #597 re-review: a holdout report of another claim's candidate of equal
/// code neither blocks nor decides this claim.
#[test]
fn cl_101_another_claims_holdout_of_equal_code_does_not_count() {
    let mut cx = holdout_cx();
    let x = cx.low();
    let y = cx.low();
    let ka = cx.candidate("a", x);
    stored(cx.report_profiles(ka, &[("unit", 0)], vec![]));
    stored(cx.holdout_run(ka, &[("holdout:api", 0)]));
    assert!(!card_of(&cx, x).holdout_due);
    let same = Candidate {
        claim: Some(y),
        ..cx.manifest(ka)
    };
    let kb = stored(cx.put("b", "same-code", ArtifactPayload::Candidate(same)));
    stored(cx.report_profiles(kb, &[("unit", 0)], vec![]));
    stored(cx.holdout_run(kb, &[("holdout:api", 1)]));
    assert!(cx.rec(y).failing.contains("holdout:api"));
    assert!(cx.rec(x).failing.is_empty());
    assert!(
        !card_of(&cx, x).holdout_due,
        "y's failure does not decide x"
    );
}

#[test]
fn cl_102_the_card_carries_the_claims_own_accept_line() {
    let mut cx = Cx::with(ClaimsConfig {
        accept_at: 10,
        accept_at_strict: 14,
        ..ClaimsConfig::default()
    });
    let low = cx.low();
    let high = cx.claim(
        "a",
        ClaimBody {
            stakes: Stakes::High,
            ..body("high")
        },
    );
    let once = cx.claim(
        "a",
        ClaimBody {
            reversible: false,
            ..body("once")
        },
    );
    assert_eq!(card_all(&cx.st, low).accept_at, 10, "the task's line");
    assert_eq!(
        card_all(&cx.st, high).accept_at,
        14,
        "strict for high stakes"
    );
    assert_eq!(
        card_all(&cx.st, once).accept_at,
        14,
        "strict when irreversible"
    );
}

#[test]
fn cl_103_the_card_shows_each_evidence_items_strength_and_note() {
    let mut cx = Cx::new();
    let c = cx.low();
    let b = cx.brief;
    let mut d = cx.evidence_draft(
        c,
        Direction::Support,
        EvidenceKind::Cited,
        2,
        vec![Ref::art(b)],
    );
    if let Body::Evidence(e) = &mut d.body {
        e.note = "the brief asks for this".into();
    }
    posted(cx.agent("b", AgentAction::Post(d)));
    ok(cx.opinion("c", c, Direction::Counter));
    let card = card_all(&cx.st, c);
    let shown: Vec<(u8, &str)> = card
        .evidence
        .iter()
        .map(|e| (e.strength, e.note.as_str()))
        .collect();
    assert_eq!(shown, [(2, "the brief asks for this"), (1, "")]);
}

// ---------------------------------------------------------------- replay (#599)

mod replay {
    use super::*;
    use rusty_bbp::history::{Change, History, Rule};

    fn history(cx: &Cx, c: MsgId) -> History {
        claim_history(&TaskId::from("t"), &cx.log, c, None).expect("history")
    }

    fn rules(h: &History) -> Vec<Rule> {
        h.steps
            .iter()
            .filter_map(|s| match &s.change {
                Change::Status { rule, .. } => Some(rule.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn accept_line_names_its_numbers_and_each_evidence_step() {
        let mut cx = Cx::new();
        let c = cx.low();
        accept(&mut cx, c);
        let h = history(&cx, c);
        assert_eq!(h.author, pid("a"));
        assert_eq!(h.body.text, "x");
        assert_eq!(h.steps[0].change, Change::Posted);
        let moves: Vec<i64> = h
            .steps
            .iter()
            .filter_map(|s| match s.change {
                Change::Evidence { delta, scoring, .. } => scoring.then_some(delta),
                _ => None,
            })
            .collect();
        assert_eq!(moves, vec![4, 4]);
        assert_eq!(
            rules(&h),
            vec![Rule::AcceptLine {
                accept_at: 8,
                support_authors: 2
            }]
        );
        let last = h.steps.last().expect("step");
        assert_eq!(
            (last.status, last.score, last.confidence),
            (ClaimStatus::Accepted, 8, 90)
        );
        assert_eq!(
            last.rev,
            cx.log
                .iter()
                .position(|e| matches!(e, Event::ClaimAccepted { .. }))
                .map(|i| Rev(i as u64 + 1))
                .expect("accepted")
        );
    }

    #[test]
    fn drop_line_and_evidence_after_settling_does_not_score() {
        let mut cx = Cx::new();
        let c = cx.low();
        drop_claim(&mut cx, c);
        ok(cx.opinion("c", c, Support));
        let h = history(&cx, c);
        assert_eq!(
            rules(&h),
            vec![Rule::DropLine {
                drop_at: -8,
                counter_authors: 2
            }]
        );
        let last = h.steps.last().expect("step");
        assert!(matches!(
            last.change,
            Change::Evidence {
                scoring: false,
                delta: 1,
                ..
            }
        ));
        assert_eq!(last.score, -8);
    }

    #[test]
    fn human_accept_after_an_irreversible_escalation() {
        let mut cx = Cx::new();
        let c = cx.claim(
            "a",
            ClaimBody {
                reversible: false,
                ..body("irreversible")
            },
        );
        let r = cx.tested("a", c, true);
        for who in ["a", "b", "c"] {
            ok(cx.executed(who, c, Support, r));
        }
        ok(cx.decide(c, ClaimOutcome::Accept));
        assert_eq!(
            rules(&history(&cx, c)),
            vec![
                Rule::Escalation(ClaimEscalation::Irreversible),
                Rule::Human { accept: true }
            ]
        );
    }

    /// Dormant passes count once the human reopens, and the line accepts in
    /// that same command: the accept is the line's, not the human's.
    #[test]
    fn a_human_reopen_the_line_then_settles_is_the_lines() {
        let mut cx = Cx::new();
        let a = cx.low();
        let b = cx.claim("b", dep_on("b", vec![a]));
        let pass = cx.tested("c", b, true);
        drop_claim(&mut cx, b);
        drop_claim(&mut cx, a);
        ok(cx.executed("c", b, Support, pass));
        ok(cx.executed("a", b, Support, pass));
        ok(cx.decide(a, ClaimOutcome::Reopen));
        accept(&mut cx, a);
        ok(cx.decide(b, ClaimOutcome::Reopen));
        assert_eq!(cx.status(b), ClaimStatus::Accepted);
        let got = rules(&history(&cx, b));
        assert_eq!(
            got[got.len() - 2..],
            [
                Rule::Reopen(ReopenCause::Human),
                Rule::AcceptLine {
                    accept_at: 8,
                    support_authors: 2
                }
            ],
            "{got:?}"
        );
    }

    #[test]
    fn choose_marks_both_sides() {
        let mut cx = Cx::new();
        let q = cx.low();
        let ans = |t: &str| ClaimBody {
            answers: Some(q),
            ..body(t)
        };
        let a = cx.claim("a", ans("a"));
        let b = cx.claim("b", ans("b"));
        let rev = cx.rev();
        ok(cx.human(HumanAction::Fenced {
            expected_rev: rev,
            action: Box::new(HumanAction::Choose {
                question: q,
                chosen: b,
            }),
        }));
        let chose = Rule::Choose { question: q };
        assert_eq!(rules(&history(&cx, b)), vec![chose.clone()]);
        assert_eq!(rules(&history(&cx, a)), vec![chose]);
    }

    #[test]
    fn at_stops_the_replay_and_an_unposted_claim_has_none() {
        let mut cx = Cx::new();
        let c = cx.low();
        accept(&mut cx, c);
        let posted_at = history(&cx, c).steps[0].rev;
        let h = claim_history(&TaskId::from("t"), &cx.log, c, Some(posted_at)).expect("history");
        assert_eq!(h.steps.len(), 1);
        assert_eq!(h.steps[0].status, ClaimStatus::Open);
        let before = Rev(posted_at.0 - 1);
        assert!(claim_history(&TaskId::from("t"), &cx.log, c, Some(before)).is_none());
        assert!(claim_history(&TaskId::from("t"), &cx.log, MsgId(999), None).is_none());
    }
}
