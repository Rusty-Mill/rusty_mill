#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Test fixture: an in-memory driver with four agents, a human and a runner,
//! plus helpers that walk the happy path to any state.
#![allow(dead_code)]

use rusty_bbp::*;

pub struct Fx<S: Store = MemStore> {
    pub d: Driver<S>,
    pub now: Time,
    ops: u64,
}

pub fn pid(role: Role) -> PrincipalId {
    PrincipalId(format!("{role:?}").to_lowercase())
}

pub fn vendor(role: Role) -> &'static str {
    match role {
        Role::Planner => "anthropic",
        Role::Coder => "openai",
        Role::Tester => "google",
        Role::Reviewer => "anthropic",
    }
}

pub const DIGEST: Sha256 = Sha256([7; 32]);

impl Fx<MemStore> {
    pub fn new() -> Fx {
        Fx::with_store(MemStore::new())
    }
}

impl<S: Store> Fx<S> {
    /// Open a task on any store and assign the four roles.
    pub fn with_store(mut store: S) -> Fx<S> {
        let task = TaskId("T1".into());
        let brief = store.blob_put(b"Add retry with backoff to the HTTP client.");
        let mut fx = Fx {
            d: Driver::new(store, task.clone()),
            now: Time(1_000),
            ops: 0,
        };
        let open = Command::Open(OpenTask {
            task,
            repo: "github.com/example/repo".into(),
            profile_digest: DIGEST,
            budget: Budget {
                messages: 40,
                bytes: 5_000_000,
                reads: 150,
                reads_per_turn: 30,
                turns: 60,
                iterations: 4,
            },
            turn_ms: 60_000,
            request_ms: 3_600_000,
            brief,
            human: PrincipalId("human".into()),
        });
        fx.run(&open);
        for role in Role::ALL {
            let principal = Principal {
                id: pid(role),
                kind: PrincipalKind::Agent {
                    role,
                    vendor: vendor(role).into(),
                },
            };
            fx.run(&Command::Assign { role, principal });
        }
        fx
    }

    pub fn st(&self) -> &TaskState {
        &self.d.state
    }

    pub fn run(&mut self, cmd: &Command) -> Response {
        match self.d.dispatch(cmd, self.now) {
            Ok(r) => r,
            Err(e) => panic!("store error: {e:?}"),
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
    pub fn tok(&self, role: Role) -> Token {
        match self.st().turn.as_ref() {
            Some(t) if t.role == role => self.st().token_for(&pid(role), t.id),
            _ => Token(Sha256([0; 32])),
        }
    }

    pub fn turn_role(&self) -> Option<Role> {
        self.st().turn.as_ref().map(|t| t.role)
    }

    pub fn agent(&mut self, role: Role, action: AgentAction) -> Response {
        let cmd = Command::Agent {
            principal: pid(role),
            token: Some(self.tok(role)),
            op: self.op(),
            action,
        };
        self.run(&cmd)
    }

    pub fn agent_with(
        &mut self,
        role: Role,
        token: Option<Token>,
        op: OpId,
        action: AgentAction,
    ) -> Response {
        self.run(&Command::Agent {
            principal: pid(role),
            token,
            op,
            action,
        })
    }

    pub fn post(&mut self, role: Role, d: Draft) -> Response {
        self.agent(role, AgentAction::Post(d))
    }

    /// Store `body` encoded for `payload` and submit it. `body` is the raw
    /// content for brief, diff and log, the Markdown for a spec, ignored otherwise.
    pub fn put(&mut self, role: Role, payload: ArtifactPayload, body: &[u8]) -> Response {
        let bytes = encode_artifact(&payload, body).expect("encode");
        let blob = self.d.store.blob_put(&bytes);
        self.agent(role, AgentAction::PutArtifact { blob, payload })
    }

    /// Submit `payload` claiming `blob` without encoding: for boundary tests.
    pub fn put_raw(&mut self, role: Role, payload: ArtifactPayload, blob: BlobRef) -> Response {
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
        (r.id, r.secret)
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
            Role::Planner,
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
        posted(self.post(Role::Planner, d));
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
        let diff = stored(self.put(Role::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
        stored(self.put(
            Role::Coder,
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

    pub fn verdict(&mut self, role: Role, kind: VerdictKind) -> Response {
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
        posted(self.verdict(Role::Tester, VerdictKind::Approve));
        assert_eq!(self.st().state, State::Review);
        c
    }

    pub fn reach_merge_gate(&mut self) -> ArtId {
        let c = self.reach_review();
        posted(self.verdict(Role::Reviewer, VerdictKind::Approve));
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

pub fn ask(to: Role, text: &str) -> Draft {
    Draft::new(MessageKind::Ask, text).to(Recipient::Role(to))
}

pub fn pass() -> Draft {
    Draft::new(MessageKind::Pass, "")
}

pub fn yield_to(role: Role) -> Draft {
    let mut d = pass();
    d.yield_to = Some(role);
    d
}
