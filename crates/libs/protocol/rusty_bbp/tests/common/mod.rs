#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Test fixture: an in-memory driver with a roster of agents, a human and a
//! runner, plus helpers that walk the happy path to any state. The default
//! roster is the four-role example of spec Appendix C.1, defined here and
//! not in the engine.
#![allow(dead_code)]

use rusty_bbp::*;

/// Handles for the four roles of the test-local roster ([`four_roles`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Planner,
    Coder,
    Tester,
    Reviewer,
}

impl Who {
    pub const ALL: [Who; 4] = [Who::Planner, Who::Coder, Who::Tester, Who::Reviewer];

    pub fn id(self) -> RoleId {
        RoleId(format!("{self:?}").to_lowercase())
    }

    pub fn of(id: &RoleId) -> Who {
        Who::ALL
            .into_iter()
            .find(|w| w.id() == *id)
            .expect("one of the four test roles")
    }
}

impl From<Who> for RoleId {
    fn from(w: Who) -> RoleId {
        w.id()
    }
}

/// Spec Appendix C.1.
pub fn four_roles() -> Vec<Role> {
    use ArtifactKind::*;
    use MessageKind::*;
    let doer = [Ask, Answer, Propose, Finding, RequestDecision, Pass];
    let all_kinds = [Brief, Spec, Diff, Candidate, TestReport, Log];
    vec![
        Role::new("planner", 0)
            .post(&doer)
            .write(&[Spec])
            .read_artifacts(&all_kinds)
            .consultable(),
        Role::new("coder", 1)
            .post(&doer)
            .write(&[Diff, Candidate])
            .read_artifacts(&all_kinds)
            .consultable(),
        Role::new("tester", 2)
            .post(&[Ask, Answer, Finding, Verdict, RequestDecision, Pass])
            .read_artifacts(&all_kinds)
            .verdict_at(&[Stage::Test])
            .consultable(),
        Role::new("reviewer", 3)
            .post(&[Answer, Finding, Verdict, RequestDecision, Pass])
            .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
            .scoped()
            .verdict_at(&[Stage::Review])
            .final_decision(),
    ]
}

pub struct Fx<S: Store = MemStore> {
    pub d: Driver<S>,
    pub now: Time,
    ops: u64,
    /// Every shadow record of every dispatch so far, in order.
    pub shadows: Vec<Shadow>,
}

/// The principal that holds `role`: the role id itself.
pub fn pid(role: impl Into<RoleId>) -> PrincipalId {
    PrincipalId(role.into().0)
}

pub fn vendor(role: &RoleId) -> &'static str {
    match role.0.as_str() {
        "coder" => "openai",
        "tester" => "google",
        _ => "anthropic",
    }
}

pub const DIGEST: Sha256 = Sha256([7; 32]);

/// The four test roles plus `coder2` and `coder3`, which also build.
pub fn builders() -> Vec<Role> {
    let mut roles = four_roles();
    for (name, order) in [("coder2", 4), ("coder3", 5)] {
        let base = roles
            .iter()
            .find(|r| r.id.0 == "coder")
            .expect("coder")
            .clone();
        let mut r = base;
        r.id = RoleId(name.into());
        r.turn_order = order;
        roles.push(r);
    }
    roles
}

pub fn budget(messages: u64, reads_per_turn: u64) -> Budget {
    Budget {
        messages,
        reads_per_turn,
        ..default_budget()
    }
}

/// The four roles plus a second reviewer, so `review` has two final-decision judges.
pub fn two_reviewers() -> Vec<Role> {
    let mut roles = four_roles();
    let mut second = roles
        .iter()
        .find(|r| r.id.0 == "reviewer")
        .expect("reviewer")
        .clone();
    second.id = RoleId("reviewer2".into());
    second.turn_order = 6;
    roles.push(second);
    roles
}

/// The four roles plus a second tester: `test` has two non-final judges, which
/// may hold turns together.
pub fn two_testers() -> Vec<Role> {
    let mut roles = four_roles();
    let mut second = roles
        .iter()
        .find(|r| r.id.0 == "tester")
        .expect("tester")
        .clone();
    second.id = RoleId("tester2".into());
    second.turn_order = 6;
    roles.push(second);
    roles
}

pub fn default_budget() -> Budget {
    Budget {
        messages: 40,
        bytes: 5_000_000,
        reads: 150,
        reads_per_turn: 30,
        turns: 60,
        iterations: 4,
    }
}

impl Fx<MemStore> {
    /// A task allowing `live` concurrent turns under `budget`.
    pub fn concurrent(roles: Vec<Role>, live: u32, budget: Budget) -> Fx {
        Fx::open_with(MemStore::new(), roles, &[], live, budget)
    }

    pub fn new() -> Fx {
        Fx::with_store(MemStore::new())
    }

    /// A task on a roster of the test's choosing.
    pub fn with_roles(roles: Vec<Role>) -> Fx {
        Fx::with_store_and_roles(MemStore::new(), roles)
    }
}

impl<S: Store> Fx<S> {
    /// Open a task on any store and assign the four roles.
    pub fn with_store(store: S) -> Fx<S> {
        Fx::with_store_and_roles(store, four_roles())
    }

    /// Open a task on any store with `roles`, and assign each to a principal.
    pub fn with_store_and_roles(store: S, roles: Vec<Role>) -> Fx<S> {
        Fx::with_roles_skipping(store, roles, &[])
    }

    /// As [`Fx::with_store_and_roles`], but leave the roles named in `skip` unassigned.
    pub fn with_roles_skipping(store: S, roles: Vec<Role>, skip: &[&str]) -> Fx<S> {
        Fx::open_with(store, roles, skip, 1, default_budget())
    }

    /// As [`Fx::with_roles_skipping`], with `live` concurrent turns and a budget.
    pub fn open_with(
        mut store: S,
        roles: Vec<Role>,
        skip: &[&str],
        live: u32,
        budget: Budget,
    ) -> Fx<S> {
        let task = TaskId("T1".into());
        let brief = store.blob_put(b"Add retry with backoff to the HTTP client.");
        let mut fx = Fx {
            d: Driver::new(store, task.clone()),
            now: Time(1_000),
            ops: 0,
            shadows: Vec::new(),
        };
        let open = Command::Open(OpenTask {
            task,
            roster: roles.clone(),
            repo: "github.com/example/repo".into(),
            profile_digest: DIGEST,
            budget,
            turn_ms: 60_000,
            request_ms: 3_600_000,
            max_live_turns: live,
            brief,
            human: PrincipalId("human".into()),
            policy: SchedulingPolicy::default(),
        });
        fx.run(&open);
        for r in roles.iter().filter(|r| !skip.contains(&r.id.0.as_str())) {
            let principal = Principal {
                id: pid(r.id.clone()),
                kind: PrincipalKind::Agent {
                    role: r.id.clone(),
                    vendor: vendor(&r.id).into(),
                },
            };
            fx.run(&Command::Assign {
                role: r.id.clone(),
                principal,
            });
        }
        // Assignment grants nothing; the first tick does.
        fx.run(&Command::Tick);
        fx
    }

    /// The default holder of `state` under this task's roster.
    pub fn holder(&self, state: State) -> RoleId {
        self.st()
            .roster
            .default_holder(state)
            .expect("a roster that serves every state")
            .clone()
    }

    pub fn st(&self) -> &TaskState {
        &self.d.state
    }

    pub fn run(&mut self, cmd: &Command) -> Response {
        // The pure engine's own answer, checked before the driver applies it:
        // independent of the store, so a reload after a conflict cannot blur it.
        let out = handle(&self.d.state, cmd, self.now);
        self.check_shadow(&out);
        let resp = match self.d.dispatch(cmd, self.now) {
            Ok(r) => r,
            Err(e) => panic!("store error: {e:?}"),
        };
        self.shadows.extend(self.d.shadow.iter().cloned());
        resp
    }

    /// Shadow mode never decides anything. For every command of every test:
    /// the recorded grantees are exactly the roles in the `turn_granted` events
    /// the command produced, every record's rev is within the command's revs,
    /// and nothing is recorded above one live turn. With `BBP_SHADOW_TRACE`
    /// set, each record is printed as a tab-separated line for the report.
    fn check_shadow(&self, out: &Handled) {
        let from = self.d.state.rev;
        let to = Rev(from.0 + out.events.len() as u64);
        let mut granted: Vec<String> = out
            .events
            .iter()
            .filter_map(|e| match e {
                Event::TurnGranted { role, .. } => Some(role.0.clone()),
                _ => None,
            })
            .collect();
        let mut recorded: Vec<String> = Vec::new();
        for sh in &out.shadow {
            assert!(
                sh.rev > from && sh.rev <= to,
                "shadow rev {:?} outside ({from:?}, {to:?}]",
                sh.rev
            );
            // The record sits beside its own `turn_granted`, and its state is
            // the state just before that event (a grant can escalate).
            let at = (sh.rev.0 - from.0 - 1) as usize;
            let who = match &sh.kind {
                ShadowKind::Auto { granted, .. } => granted.first().expect("a grantee"),
                ShadowKind::Explicit { granted, .. } => granted,
            };
            assert!(
                matches!(&out.events[at], Event::TurnGranted { role, .. } if role == who),
                "shadow rev {:?} is not the turn_granted for {who:?}: {:?}",
                sh.rev,
                out.events[at]
            );
            let mut pre = self.d.state.clone();
            for e in &out.events[..at] {
                pre.apply(e);
            }
            assert_eq!(
                sh.state, pre.state,
                "shadow state vs the state before the grant"
            );
            match &sh.kind {
                ShadowKind::Auto { granted, .. } => {
                    recorded.extend(granted.iter().map(|r| r.0.clone()))
                }
                ShadowKind::Explicit { granted, .. } => recorded.push(granted.0.clone()),
            }
            if std::env::var_os("BBP_SHADOW_TRACE").is_some() {
                eprintln!("{}", trace_line(sh));
            }
        }
        let cap = out
            .events
            .iter()
            .find_map(|e| match e {
                Event::TaskOpened { max_live_turns, .. } => Some(*max_live_turns),
                _ => None,
            })
            .unwrap_or(self.d.state.max_live_turns);
        if cap <= 1 {
            granted.sort();
            recorded.sort();
            assert_eq!(recorded, granted, "shadow records vs turn_granted events");
        } else {
            assert!(out.shadow.is_empty(), "no shadow above one live turn");
        }
    }

    pub fn op(&mut self) -> OpId {
        self.ops += 1;
        OpId(format!("op{}", self.ops))
    }

    pub fn advance(&mut self, ms: u64) {
        self.now = self.now.plus(ms);
    }

    pub fn tick(&mut self) -> Response {
        self.run(&Command::Tick)
    }

    /// Live token for `role`, or a bogus one if it does not hold the turn.
    pub fn tok(&self, role: impl Into<RoleId>) -> Token {
        let role = role.into();
        match self.st().turn_of(&role) {
            Some(t) => self.st().token_for(&pid(role), t.id),
            None => Token(Sha256([0; 32])),
        }
    }

    /// The live turn's holder among the four test roles.
    pub fn turn_role(&self) -> Option<Who> {
        self.st().turn().map(|t| Who::of(&t.role))
    }

    /// The live turn's holder, whatever the roster.
    pub fn turn_holder(&self) -> Option<RoleId> {
        self.st().turn().map(|t| t.role.clone())
    }

    pub fn agent(&mut self, role: impl Into<RoleId>, action: AgentAction) -> Response {
        let role = role.into();
        let cmd = Command::Agent {
            principal: pid(role.clone()),
            token: Some(self.tok(role)),
            op: self.op(),
            action,
        };
        self.run(&cmd)
    }

    pub fn agent_with(
        &mut self,
        role: impl Into<RoleId>,
        token: Option<Token>,
        op: OpId,
        action: AgentAction,
    ) -> Response {
        self.run(&Command::Agent {
            principal: pid(role.into()),
            token,
            op,
            action,
        })
    }

    pub fn post(&mut self, role: impl Into<RoleId>, d: Draft) -> Response {
        self.agent(role, AgentAction::Post(d))
    }

    /// Store `body` encoded for `payload` and submit it. `body` is the raw
    /// content for brief, diff and log, the Markdown for a spec, ignored otherwise.
    pub fn put(
        &mut self,
        role: impl Into<RoleId>,
        payload: ArtifactPayload,
        body: &[u8],
    ) -> Response {
        let bytes = encode_artifact(&payload, body).expect("encode");
        let blob = self.d.store.blob_put(&bytes);
        self.agent(role, AgentAction::PutArtifact { blob, payload })
    }

    /// Submit `payload` claiming `blob` without encoding: for boundary tests.
    pub fn put_raw(
        &mut self,
        role: impl Into<RoleId>,
        payload: ArtifactPayload,
        blob: BlobRef,
    ) -> Response {
        self.agent(role, AgentAction::PutArtifact { blob, payload })
    }

    pub fn human(&mut self, action: HumanAction) -> Response {
        let cmd = Command::Human {
            op: self.op(),
            action,
        };
        self.run(&cmd)
    }

    pub fn rev(&self) -> Rev {
        self.st().rev
    }

    pub fn selected_run(&self) -> (RunId, RunSecret) {
        let r = self.st().selected_run().expect("selected run");
        (r.id, self.st().secret_for(r.id))
    }

    /// Runner stores a log then a report for the selected run.
    pub fn report(&mut self, status: RunStatus) -> Response {
        let (run, secret) = self.selected_run();
        self.report_as(run, secret, status)
    }

    pub fn report_as(&mut self, run: RunId, secret: RunSecret, status: RunStatus) -> Response {
        let cand = self.st().candidate.expect("candidate");
        let blob = self.d.store.blob_put(b"test output");
        let op = self.op();
        let log = self.run(&Command::Runner {
            op,
            run,
            secret,
            blob,
            payload: ArtifactPayload::Log,
        });
        let log = match log {
            Response::Stored(id) => id,
            other => return other,
        };
        let (exit, failed) = match status {
            RunStatus::Passed => (0, 0),
            RunStatus::Failed => (101, 2),
            RunStatus::Error => (1, 0),
        };
        let rep = Report {
            candidate: cand,
            run,
            profile_digest: DIGEST,
            status,
            profiles: vec![ProfileResult {
                name: "test".into(),
                exit_code: exit,
                failed,
            }],
            tree: Some(Sha256([9; 32])),
            sandbox: "test".into(),
            log,
        };
        let payload = ArtifactPayload::TestReport(rep);
        let bytes = encode_artifact(&payload, b"").expect("encode");
        let blob = self.d.store.blob_put(&bytes);
        let op = self.op();
        self.run(&Command::Runner {
            op,
            run,
            secret,
            blob,
            payload,
        })
    }

    pub fn report_id(&self) -> ArtId {
        self.st()
            .selected_run()
            .and_then(|r| r.report)
            .expect("report")
    }

    // ---- happy path

    pub fn spec(&mut self) -> ArtId {
        let brief = self.st().brief.expect("brief");
        stored(self.put(
            self.holder(State::Planning),
            ArtifactPayload::Spec { brief },
            b"# Spec\nAC1 ...",
        ))
    }

    pub fn reach_plan_gate(&mut self) -> ArtId {
        let spec = self.spec();
        let mut d = Draft::new(MessageKind::RequestDecision, "Please approve the plan.")
            .to(Recipient::Human)
            .refs(vec![Ref::art(spec)]);
        d.gate = true;
        posted(self.post(self.holder(State::Planning), d));
        assert_eq!(self.st().state, State::PlanGate);
        spec
    }

    pub fn reach_build(&mut self) -> ArtId {
        let spec = self.reach_plan_gate();
        ok(self.human(HumanAction::Approve {
            gate: Gate::Plan,
            subject: spec,
            run: None,
        }));
        assert_eq!(self.st().state, State::Build);
        spec
    }

    pub fn candidate(&mut self) -> ArtId {
        let builder = self.holder(State::Build);
        let diff = stored(self.put(builder.clone(), ArtifactPayload::Diff, b"--- a\n+++ b\n"));
        stored(self.put(
            builder,
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![diff],
            }),
            b"{}",
        ))
    }

    pub fn reach_test(&mut self) -> ArtId {
        self.reach_build();
        let c = self.candidate();
        assert_eq!(self.st().state, State::Test);
        c
    }

    pub fn verdict(&mut self, role: impl Into<RoleId>, kind: VerdictKind) -> Response {
        let cand = self.st().candidate.expect("candidate");
        let (run, _) = self.selected_run();
        let report = self.report_id();
        let blocking = match kind {
            VerdictKind::Approve => vec![],
            VerdictKind::Revise => vec![BlockingItem {
                id: "B1".into(),
                reference: Ref::art(cand),
                issue: "x".into(),
                fix: "y".into(),
            }],
        };
        let d = Draft {
            kind: MessageKind::Verdict,
            to: vec![Recipient::All],
            body: Body::Verdict(Verdict {
                subject: cand,
                run,
                verdict: kind,
                blocking,
                non_blocking: vec![],
            }),
            refs: vec![Ref::art(cand)],
            evidence: vec![Ref::art(report)],
            reply_to: None,
            yield_to: None,
            gate: false,
        };
        self.post(role, d)
    }

    pub fn reach_review(&mut self) -> ArtId {
        let c = self.reach_test();
        stored(self.report(RunStatus::Passed));
        posted(self.verdict(self.holder(State::Test), VerdictKind::Approve));
        assert_eq!(self.st().state, State::Review);
        c
    }

    pub fn reach_merge_gate(&mut self) -> ArtId {
        let c = self.reach_review();
        posted(self.verdict(self.holder(State::Review), VerdictKind::Approve));
        assert_eq!(self.st().state, State::MergeGate);
        c
    }

    pub fn reach_approved(&mut self) -> ArtId {
        let c = self.reach_merge_gate();
        let (run, _) = self.selected_run();
        ok(self.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: c,
            run: Some(run),
        }));
        assert_eq!(self.st().state, State::Approved);
        c
    }
}

pub fn stored(r: Response) -> ArtId {
    match r {
        Response::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

pub fn posted(r: Response) -> MsgId {
    match r {
        Response::Posted(id) => id,
        other => panic!("expected Posted, got {other:?}"),
    }
}

pub fn ok(r: Response) {
    assert_eq!(r, Response::Ok);
}

pub fn rejected(r: Response) -> Code {
    match r {
        Response::Rejected(rej) => rej.code,
        other => panic!("expected rejection, got {other:?}"),
    }
}

pub fn rejection(r: Response) -> Rejection {
    match r {
        Response::Rejected(rej) => rej,
        other => panic!("expected rejection, got {other:?}"),
    }
}

pub fn ask(to: impl Into<RoleId>, text: &str) -> Draft {
    Draft::new(MessageKind::Ask, text).to(Recipient::Role(to.into()))
}

pub fn pass() -> Draft {
    Draft::new(MessageKind::Pass, "")
}

pub fn yield_to(role: impl Into<RoleId>) -> Draft {
    let mut d = pass();
    d.yield_to = Some(role.into());
    d
}

/// One report line:
/// `SHADOW<TAB>test<TAB>state<TAB>auto|explicit<TAB>granted<TAB>would<TAB>outcome`,
/// where `would` is the whole `wake_set`, best first (`role:reason,...`).
fn trace_line(sh: &Shadow) -> String {
    let test = std::thread::current().name().unwrap_or("?").to_string();
    let state = sh.state;
    match &sh.kind {
        ShadowKind::Auto { granted, would } => format!(
            "SHADOW\t{test}\t{state:?}\tauto\t{}\t{}\t{:?}",
            granted.first().map_or("-", |r| r.0.as_str()),
            would
                .iter()
                .map(|(r, why)| format!("{}:{why:?}", r.0))
                .collect::<Vec<_>>()
                .join(","),
            sh.comparison(),
        ),
        ShadowKind::Explicit { granted, kind } => {
            format!(
                "SHADOW\t{test}\t{state:?}\texplicit\t{}\t{kind:?}\t-",
                granted.0
            )
        }
    }
}
