#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Roles as data: roster validation, rosters of other shapes than the
//! four-role example, capability enforcement and its error messages, the
//! derived yield rule, roster immutability and the log schema.

mod common;
use common::*;
use rusty_bbp::*;

use ArtifactKind::{Brief, Candidate, Diff, Log, Spec, TestReport};
use MessageKind::{Answer, Ask, Finding, Pass, Propose, RequestDecision, Verdict};

const ALL_KINDS: [ArtifactKind; 6] = [Brief, Spec, Diff, Candidate, TestReport, Log];
const DOER: [MessageKind; 6] = [Ask, Answer, Propose, Finding, RequestDecision, Pass];
const JUDGE: [MessageKind; 6] = [Ask, Answer, Finding, Verdict, RequestDecision, Pass];

fn without(id: &str) -> Vec<Role> {
    four_roles().into_iter().filter(|r| r.id.0 != id).collect()
}

fn two_roles() -> Vec<Role> {
    vec![
        Role::new("author", 0)
            .post(&[Ask, Answer, Propose, Finding, RequestDecision, Pass])
            .write(&[Spec, Diff, Candidate])
            .read_artifacts(&ALL_KINDS)
            .consultable(),
        Role::new("judge", 1)
            .post(&JUDGE)
            .read_artifacts(&ALL_KINDS)
            .verdict_at(&[Stage::Test, Stage::Review])
            .final_decision(),
    ]
}

fn observer() -> Role {
    Role::new("watcher", 5).read_artifacts(&ALL_KINDS)
}

fn six_roles() -> Vec<Role> {
    vec![
        Role::new("lead", 0)
            .post(&DOER)
            .write(&[Spec])
            .read_artifacts(&ALL_KINDS)
            .consultable(),
        Role::new("dev1", 1)
            .post(&DOER)
            .write(&[Diff, Candidate])
            .read_artifacts(&ALL_KINDS)
            .consultable(),
        Role::new("dev2", 2)
            .post(&DOER)
            .write(&[Diff])
            .read_artifacts(&ALL_KINDS)
            .consultable(),
        Role::new("qa", 3)
            .post(&JUDGE)
            .read_artifacts(&ALL_KINDS)
            .verdict_at(&[Stage::Test])
            .consultable(),
        Role::new("sec", 4)
            .post(&JUDGE)
            .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
            .scoped()
            .verdict_at(&[Stage::Review])
            .final_decision(),
        observer(),
    ]
}

fn close(fx: &mut Fx) {
    let c = fx.reach_approved();
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "r".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
}

// ---------------------------------------------------------------- roster validation

#[test]
fn the_example_rosters_are_valid() {
    assert!(Roster::new(four_roles()).is_ok());
    assert!(Roster::new(two_roles()).is_ok());
    assert!(Roster::new(six_roles()).is_ok());
}

#[test]
fn ids_are_tokens_and_unreserved_and_unique() {
    let bad = |id: &str| {
        let mut r = four_roles();
        r[0] = Role {
            id: RoleId(id.into()),
            ..r[0].clone()
        };
        Roster::new(r)
    };
    for id in [
        "",
        "Planner",
        "has space",
        "1st",
        "x".repeat(33).as_str(),
        "a/b",
    ] {
        assert!(
            matches!(bad(id), Err(RosterError::BadId { .. })),
            "{id:?} should be a bad id"
        );
    }
    for id in ["human", "moderator", "runner"] {
        assert!(matches!(bad(id), Err(RosterError::ReservedId { .. })));
    }
    assert!(bad(&"x".repeat(32)).is_ok());
    let mut dup = four_roles();
    dup[1].id = dup[0].id.clone();
    assert!(matches!(
        Roster::new(dup),
        Err(RosterError::DuplicateId { .. })
    ));
}

#[test]
fn turn_order_is_unique() {
    let mut r = four_roles();
    r[3].turn_order = r[0].turn_order;
    let err = Roster::new(r).expect_err("duplicate turn_order");
    assert_eq!(
        err,
        RosterError::DuplicateTurnOrder {
            order: 0,
            first: "planner".into(),
            second: "reviewer".into()
        }
    );
    assert!(err.to_string().contains("share turn_order 0"));
}

#[test]
fn every_working_state_must_have_a_serving_role() {
    for (gone, state) in [
        ("planner", State::Planning),
        ("coder", State::Build),
        ("tester", State::Test),
    ] {
        match Roster::new(without(gone)) {
            Err(RosterError::Unservable { state: s, .. }) => assert_eq!(s, state, "{gone}"),
            other => panic!("expected Unservable({state:?}), got {other:?}"),
        }
    }
    // No role can judge `review` at all: unservable, not "no finalizer".
    let mut r = four_roles();
    r[3].verdict_at.clear();
    assert!(matches!(
        Roster::new(r),
        Err(RosterError::Unservable {
            state: State::Review,
            ..
        })
    ));
    assert!(matches!(
        Roster::new(vec![]),
        Err(RosterError::Unservable {
            state: State::Planning,
            ..
        })
    ));
}

#[test]
fn a_roster_with_no_finalizer_is_rejected_with_its_own_error() {
    let mut r = four_roles();
    r[3].final_decision = false;
    let err = Roster::new(r.clone()).expect_err("no finalizer");
    assert_eq!(err, RosterError::NoFinalizer);
    assert!(err.to_string().contains("final_decision"));
    // A final_decision role that cannot judge review does not count.
    r.push(Role::new("boss", 9).post(&JUDGE).final_decision());
    assert_eq!(Roster::new(r), Err(RosterError::NoFinalizer));
}

#[test]
fn opening_a_task_with_an_invalid_roster_is_refused_and_no_task_exists() {
    let mut r = four_roles();
    r[3].final_decision = false;
    let mut store = MemStore::new();
    let brief = store.blob_put(b"brief");
    let task = TaskId("T9".into());
    let mut d = Driver::new(store, task.clone());
    let cmd = Command::Open(OpenTask {
        task,
        roster: r,
        repo: "r".into(),
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
        max_live_turns: 1,
        brief,
        human: PrincipalId("human".into()),
    });
    let rej = rejection(d.dispatch(&cmd, Time(1)).expect("dispatch"));
    assert_eq!(rej.code, Code::BadRoster);
    assert!(rej.detail.contains("no finalizer"), "{}", rej.detail);
    assert!(!d.state.opened);
    let events = d.store.events(&d.state.task, 0).unwrap_or_default();
    assert!(
        events.is_empty(),
        "a refused open writes nothing: {events:?}"
    );
    // The same task opens afterwards with a corrected roster.
    let Command::Open(refused) = cmd else {
        unreachable!("built above")
    };
    let mut fixed = four_roles();
    fixed[3].final_decision = true;
    let again = Command::Open(OpenTask {
        roster: fixed,
        ..refused
    });
    assert!(matches!(
        d.dispatch(&again, Time(2)).expect("dispatch"),
        Response::Stored(_)
    ));
    assert!(d.state.opened);
}

// ---------------------------------------------------------------- other roster shapes

#[test]
fn a_two_role_roster_runs_the_whole_lifecycle() {
    let mut fx = Fx::with_roles(two_roles());
    assert_eq!(fx.holder(State::Planning), "author".into());
    assert_eq!(fx.holder(State::Test), "judge".into());
    close(&mut fx);
}

#[test]
fn a_six_role_roster_runs_the_whole_lifecycle() {
    let mut fx = Fx::with_roles(six_roles());
    for (state, who) in [
        (State::Planning, "lead"),
        (State::Build, "dev1"),
        (State::Test, "qa"),
        (State::Review, "sec"),
    ] {
        assert_eq!(fx.holder(state), who.into(), "{state:?}");
    }
    close(&mut fx);
}

#[test]
fn turn_order_decides_between_two_serving_roles() {
    // dev2 serves build only when it can write both diff and candidate.
    let mut roles = six_roles();
    roles[2] = roles[2].clone().write(&[Diff, Candidate]);
    assert_eq!(
        Roster::new(roles.clone())
            .unwrap()
            .default_holder(State::Build),
        Some(&"dev1".into())
    );
    roles[2].turn_order = 0;
    roles[0].turn_order = 2;
    assert_eq!(
        Roster::new(roles).unwrap().default_holder(State::Build),
        Some(&"dev2".into())
    );
}

#[test]
fn a_role_with_no_write_capability_reads_but_cannot_write_or_post() {
    let mut fx = Fx::with_roles(six_roles());
    // A human ask is the only way an observer holds a turn; at a gate none is live.
    fx.reach_plan_gate();
    posted(fx.human(HumanAction::Post(ask("watcher", "anything to add?"))));
    assert_eq!(fx.turn_holder(), Some("watcher".into()));
    assert!(matches!(
        fx.agent(
            "watcher",
            AgentAction::Read {
                after: None,
                limit: 5
            }
        ),
        Response::Read { .. }
    ));
    let brief = fx.st().brief.expect("brief");
    assert!(matches!(
        fx.agent("watcher", AgentAction::GetArtifact { id: brief }),
        Response::Artifact(_)
    ));
    let r = rejection(fx.post("watcher", Draft::new(Answer, "x")));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (Code::RoleForbidden, "role watcher lacks post:answer")
    );
    let r = rejection(fx.put("watcher", ArtifactPayload::Diff, b"d"));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (Code::KindForbidden, "role watcher lacks write:diff")
    );
    let r = rejection(fx.post("watcher", pass()));
    assert_eq!(r.code, Code::RoleForbidden, "even pass is a capability");
}

// ---------------------------------------------------------------- capability errors

#[test]
fn a_capability_violation_names_the_role_and_the_missing_capability() {
    let mut fx = Fx::new();
    // post
    let r = rejection(fx.post(Who::Planner, Draft::new(Verdict, "x")));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (Code::RoleForbidden, "role planner lacks post:verdict")
    );
    // write
    fx.reach_build();
    let r = rejection(fx.put(
        Who::Coder,
        ArtifactPayload::Spec {
            brief: fx.st().brief.unwrap(),
        },
        b"s",
    ));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (Code::KindForbidden, "role coder lacks write:spec")
    );
    // read_artifacts
    let mut fx = Fx::new();
    fx.reach_review();
    let log = fx.st().selected_run().and_then(|r| r.log).expect("log");
    let r = rejection(fx.agent(Who::Reviewer, AgentAction::GetArtifact { id: log }));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (
            Code::RoleForbidden,
            "role reviewer lacks read_artifacts:log"
        )
    );
    // scoped readers answer only the human
    let q = posted(fx.human(HumanAction::Post(ask(Who::Reviewer, "q?"))));
    let a = Draft::new(Answer, "a")
        .to(Recipient::Role(Who::Planner.id()))
        .reply_to(q);
    let r = rejection(fx.post(Who::Reviewer, a));
    assert_eq!(
        (r.code, r.detail.as_str()),
        (
            Code::BadRecipient,
            "role reviewer reads scoped messages and answers only the human"
        )
    );
}

// ---------------------------------------------------------------- the derived yield rule

fn yield_code(fx: &mut Fx, from: impl Into<RoleId>, to: &str) -> Rejection {
    rejection(fx.post(from, yield_to(to)))
}

#[test]
fn a_scoped_holder_cannot_yield() {
    let mut fx = Fx::new();
    fx.reach_review();
    assert_eq!(fx.turn_role(), Some(Who::Reviewer));
    let r = yield_code(&mut fx, Who::Reviewer, "planner");
    assert_eq!(r.code, Code::WrongState);
    assert!(r.detail.contains("scoped"), "{}", r.detail);
}

#[test]
fn a_non_consultable_target_is_refused() {
    let mut fx = Fx::new();
    let r = yield_code(&mut fx, Who::Planner, "reviewer");
    assert_eq!(r.code, Code::WrongState);
    assert!(r.detail.contains("not consultable"), "{}", r.detail);
}

#[test]
fn the_default_holder_is_not_a_yield_target() {
    let mut fx = Fx::new();
    let r = yield_code(&mut fx, Who::Planner, "planner");
    assert_eq!(r.code, Code::WrongState);
}

#[test]
fn an_unassigned_or_unknown_target_is_refused() {
    let mut roles = four_roles();
    roles.push(
        Role::new("scribe", 8)
            .post(&DOER)
            .read_artifacts(&ALL_KINDS)
            .consultable(),
    );
    let mut fx = Fx::with_roles_skipping(MemStore::new(), roles, &["scribe"]);
    let r = yield_code(&mut fx, Who::Planner, "scribe");
    assert_eq!(
        (r.code, r.detail.as_str()),
        (Code::BadRecipient, "scribe not assigned")
    );
    let r = yield_code(&mut fx, Who::Planner, "ghost");
    assert_eq!(r.code, Code::BadRecipient);
}

#[test]
fn a_consultable_assigned_non_default_target_is_granted_the_turn() {
    let mut fx = Fx::new();
    posted(fx.post(Who::Planner, yield_to("coder")));
    assert_eq!(fx.turn_role(), Some(Who::Coder));
    assert_eq!(fx.st().turn().map(|t| t.kind), Some(TurnKind::Consultation));
}

#[test]
fn a_human_ask_reaches_a_role_that_is_not_consultable() {
    // At a gate no default turn is live, so the ask grants a consultation turn.
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    assert!(fx.st().turn().is_none());
    posted(fx.human(HumanAction::Post(ask(Who::Reviewer, "your view?"))));
    assert_eq!(fx.turn_role(), Some(Who::Reviewer));
    assert_eq!(fx.st().turn().map(|t| t.kind), Some(TurnKind::Consultation));
}

// ---------------------------------------------------------------- final_decision at the merge gate

#[test]
fn a_revise_from_any_final_role_blocks_the_merge_gate() {
    // Two final roles. Under turn-taking only the default holder judges, so the
    // second role's revise is injected into the log as the event-driven policy
    // would produce it.
    let mut roles = four_roles();
    roles.push(
        Role::new("second", 9)
            .post(&JUDGE)
            .read_artifacts(&ALL_KINDS)
            .verdict_at(&[Stage::Review])
            .final_decision(),
    );
    let mut fx = Fx::with_roles(roles);
    let cand = fx.reach_merge_gate();
    let (run, _) = fx.selected_run();
    let task = fx.st().task.clone();
    let mut events = fx.d.store.events(&task, 0).expect("events");
    let at = events
        .iter()
        .rposition(|e| {
            matches!(
                e,
                Event::StateChanged {
                    to: State::MergeGate,
                    ..
                }
            )
        })
        .expect("merge gate entered");
    let revise = Draft {
        kind: Verdict,
        to: vec![Recipient::All],
        body: Body::Verdict(rusty_bbp::Verdict {
            subject: cand,
            run,
            verdict: VerdictKind::Revise,
            blocking: vec![BlockingItem {
                id: "B1".into(),
                reference: Ref::art(cand),
                issue: "x".into(),
                fix: "y".into(),
            }],
            non_blocking: vec![],
        }),
        refs: vec![Ref::art(cand)],
        evidence: vec![],
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    events.insert(
        at,
        Event::MessageAppended(Message {
            id: MsgId(999),
            from: pid("second"),
            from_role: Some("second".into()),
            draft: revise,
        }),
    );
    let st = TaskState::fold(task, &events);
    assert!(st.final_approved(), "the reviewer approved");
    assert!(st.final_blocked(), "and a final role asked for revision");
    let out = handle(
        &st,
        &Command::Human {
            op: OpId("blocked".into()),
            action: HumanAction::Approve {
                gate: Gate::Merge,
                subject: cand,
                run: Some(run),
            },
        },
        fx.now,
    );
    let r = rejection(out.response);
    assert_eq!(r.code, Code::NotReviewed);
    assert!(r.detail.contains("revision"), "{}", r.detail);
}

#[test]
fn one_approve_from_one_final_role_passes_the_gate() {
    // The four-role roster has a single final role, as v0.4.1 had one Reviewer.
    let mut fx = Fx::new();
    let c = fx.reach_merge_gate();
    let (run, _) = fx.selected_run();
    assert!(fx.st().final_approved() && !fx.st().final_blocked());
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: c,
        run: Some(run),
    }));
}

// ---------------------------------------------------------------- immutability and schema

#[test]
fn the_roster_is_recorded_and_cannot_change() {
    let mut fx = Fx::new();
    let task = fx.st().task.clone();
    let opened = fx.d.store.events(&task, 0).expect("events");
    match opened
        .iter()
        .find(|e| matches!(e, Event::TaskOpened { .. }))
    {
        Some(Event::TaskOpened { schema, roster, .. }) => {
            assert_eq!(*schema, LOG_SCHEMA);
            assert_eq!(roster, &four_roles());
        }
        _ => panic!("no task_opened"),
    }
    let before = fx.st().roster.clone();
    let r = rejection(fx.run(&Command::Assign {
        role: "ghost".into(),
        principal: Principal {
            id: "g".into(),
            kind: PrincipalKind::Agent {
                role: "ghost".into(),
                vendor: "v".into(),
            },
        },
    }));
    assert_eq!(r.code, Code::UnknownRole);
    let again = Command::Open(OpenTask {
        task: task.clone(),
        roster: two_roles(),
        repo: "r".into(),
        profile_digest: DIGEST,
        budget: fx.st().budget,
        turn_ms: 1,
        request_ms: 1,
        max_live_turns: 1,
        brief: fx.d.store.blob_put(b"b"),
        human: PrincipalId("human".into()),
    });
    assert_eq!(rejection(fx.run(&again)).code, Code::AlreadyOpen);
    assert_eq!(fx.st().roster, before);
}

#[test]
fn a_log_under_another_schema_does_not_replay() {
    let mut store = MemStore::new();
    let task = TaskId("T7".into());
    let opened = Event::TaskOpened {
        schema: 1,
        roster: four_roles(),
        task: task.clone(),
        repo: "r".into(),
        profile_digest: DIGEST,
        budget: Budget {
            messages: 1,
            bytes: 1,
            reads: 1,
            reads_per_turn: 1,
            turns: 1,
            iterations: 1,
        },
        turn_ms: 1,
        request_ms: 1,
        max_live_turns: 1,
        human: PrincipalId("human".into()),
    };
    store.append(&task, Rev(0), &[opened]).expect("append");
    let mut d = Driver::new(store, task);
    assert_eq!(
        d.reload(),
        Err(StoreError::UnsupportedSchema { found: Some(1) })
    );
}

#[test]
fn the_file_store_refuses_a_v0_4_log_and_a_future_schema() {
    let dir = std::env::temp_dir().join(format!("bbp-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tasks")).expect("dir");
    // A schema-1 task_opened has no `schema` field.
    let legacy = log_path_for(&dir, &TaskId("T1".into()));
    std::fs::write(&legacy, "[{\"TaskOpened\":{\"task\":\"T1\"}}]\n").expect("write");
    let err = FsStore::open(&dir).err().expect("refused");
    assert!(
        matches!(err, FsError::UnsupportedSchema { found: None, .. }),
        "{err}"
    );
    assert!(err.to_string().contains("archive"));
    let _ = std::fs::remove_dir_all(&dir);

    // An explicit schema from a newer build is refused the same way.
    let task = TaskId("T3".into());
    let mut store = FsStore::open(&dir).expect("fresh dir");
    let opened = Event::TaskOpened {
        schema: LOG_SCHEMA_CONCURRENT + 1,
        roster: four_roles(),
        task: task.clone(),
        repo: "r".into(),
        profile_digest: DIGEST,
        budget: Budget {
            messages: 1,
            bytes: 1,
            reads: 1,
            reads_per_turn: 1,
            turns: 1,
            iterations: 1,
        },
        turn_ms: 1,
        request_ms: 1,
        max_live_turns: 1,
        human: PrincipalId("human".into()),
    };
    store.append(&task, Rev(0), &[opened]).expect("append");
    drop(store);
    let err = FsStore::open(&dir).err().expect("refused");
    assert!(
        matches!(err, FsError::UnsupportedSchema { found: Some(n), .. } if n == LOG_SCHEMA_CONCURRENT + 1),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
