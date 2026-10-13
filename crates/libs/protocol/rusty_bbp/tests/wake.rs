#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `wake_set`: which roles the board state makes eligible. The engine's real
//! grants are turn-based and always leave a live turn, so most cases examine a
//! state with the live turn cleared, or a log with a turn end spliced in.

mod common;
use common::*;
use rusty_bbp::*;

use ArtifactKind::{Candidate, Diff, Spec, TestReport};
use MessageKind::{Answer, Ask, Finding, Pass, RequestDecision};

fn ids(w: &[(RoleId, WakeReason)]) -> Vec<(String, WakeReason)> {
    w.iter().map(|(r, why)| (r.0.clone(), *why)).collect()
}

fn pair(role: &str, why: WakeReason) -> (String, WakeReason) {
    (role.to_string(), why)
}

/// The live state with no live turn: the moment before a grant.
fn idle(fx: &Fx) -> TaskState {
    let mut st = fx.st().clone();
    st.turn = None;
    st
}

fn events(fx: &Fx) -> Vec<Event> {
    fx.d.store.events(&fx.st().task, 0).expect("events")
}

fn fold(fx: &Fx, evs: &[Event]) -> TaskState {
    TaskState::fold(fx.st().task.clone(), evs)
}

/// A message as if `from` had appended it, `to` addressed.
fn message(id: u64, from: Option<&str>, to: Recipient, kind: MessageKind) -> Event {
    let mut draft = Draft::new(kind, "x");
    draft.to = vec![to];
    Event::MessageAppended(Message {
        id: MsgId(id),
        from: from.map_or(PrincipalId("human".into()), pid),
        from_role: from.map(RoleId::from),
        draft,
    })
}

/// A turn of `role` that starts and ends, as the engine would log it.
fn turn_of(role: &str, n: u64) -> [Event; 2] {
    [
        Event::TurnGranted {
            role: role.into(),
            turn: TurnId(n),
            kind: TurnKind::Default,
            deadline: Time(u64::MAX),
            return_to: None,
        },
        Event::TurnEnded {
            turn: TurnId(n),
            cause: rusty_bbp::event::TurnEnd::Pass,
        },
    ]
}

/// The log up to the first turn end, with the grant after it dropped.
fn before_first_regrant(fx: &Fx) -> Vec<Event> {
    let evs = events(fx);
    let end = evs
        .iter()
        .position(|e| matches!(e, Event::TurnEnded { .. }))
        .expect("a turn ended");
    evs[..=end].to_vec()
}

// ---------------------------------------------------------------- guards

#[test]
fn a_fresh_task_wakes_the_state_holder_only() {
    let fx = Fx::new();
    assert_eq!(
        ids(&wake_set(&idle(&fx))),
        [pair("planner", WakeReason::State)]
    );
}

#[test]
fn nobody_is_woken_while_a_turn_is_live() {
    let fx = Fx::new();
    assert!(fx.st().turn.is_some());
    assert!(wake_set(fx.st()).is_empty());
}

#[test]
fn nobody_is_woken_while_a_request_is_open() {
    let mut fx = Fx::new();
    let spec = fx.spec();
    let mut d = Draft::new(RequestDecision, "may I?")
        .to(Recipient::Human)
        .refs(vec![Ref::art(spec)]);
    d.gate = false;
    posted(fx.post(Who::Planner, d));
    assert!(fx.st().pending_request.is_some());
    assert!(wake_set(&idle(&fx)).is_empty());
}

#[test]
fn gates_and_terminal_states_wake_nobody() {
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    assert!(wake_set(&idle(&fx)).is_empty(), "plan_gate");
    let mut fx = Fx::new();
    fx.reach_merge_gate();
    assert!(wake_set(&idle(&fx)).is_empty(), "merge_gate");
    let mut fx = Fx::new();
    let c = fx.reach_approved();
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "r".into(),
    }));
    assert!(wake_set(&idle(&fx)).is_empty(), "closed");
}

#[test]
fn test_waits_for_the_report_then_wakes_its_judge() {
    let mut fx = Fx::new();
    fx.reach_test();
    assert!(wake_set(&idle(&fx)).is_empty(), "no final report yet");
    stored(fx.report(RunStatus::Passed));
    let w = ids(&wake_set(&idle(&fx)));
    // The tester has had no turn yet, so the stored artifacts count as new for
    // it and its best reason is `Artifact`, which outranks `State`.
    assert!(w.contains(&pair("tester", WakeReason::Artifact)), "{w:?}");
}

#[test]
fn an_unassigned_role_is_never_woken() {
    let fx = Fx::with_roles_skipping(MemStore::new(), four_roles(), &["planner"]);
    assert!(wake_set(&idle(&fx)).is_empty());
}

#[test]
fn it_is_pure() {
    let fx = Fx::new();
    let st = idle(&fx);
    let before = st.clone();
    assert_eq!(wake_set(&st), wake_set(&st));
    assert_eq!(st, before);
}

// ---------------------------------------------------------------- state trigger

#[test]
fn the_default_holder_is_eligible_in_every_working_state() {
    let mut fx = Fx::new();
    let at_planning = idle(&fx);
    fx.reach_build();
    let at_build = idle(&fx);
    let mut fx2 = Fx::new();
    fx2.reach_test();
    stored(fx2.report(RunStatus::Passed));
    let at_test = idle(&fx2);
    let mut fx3 = Fx::new();
    fx3.reach_review();
    let at_review = idle(&fx3);
    for st in [at_planning, at_build, at_test, at_review] {
        let holder = st.roster.default_holder(st.state).cloned().expect("holder");
        let w = wake_set(&st);
        // The holder is always eligible; its best reason may outrank `State`.
        assert!(w.iter().any(|(r, _)| *r == holder), "{:?}: {w:?}", st.state);
    }
}

#[test]
fn a_role_that_just_ended_its_turn_is_not_woken_by_the_same_state() {
    // The planner passed without progress. Turn-taking regrants it at once;
    // the event-driven trigger does not fire again for the same state entry.
    let mut fx = Fx::new();
    posted(fx.post(Who::Planner, pass()));
    let st = fold(&fx, &before_first_regrant(&fx));
    assert!(st.turn.is_none() && st.last_turn_end.contains_key(&"planner".into()));
    assert!(wake_set(&st).is_empty());
}

// ---------------------------------------------------------------- addressed

#[test]
fn an_addressed_role_is_woken_and_the_author_is_not() {
    let mut fx = Fx::new();
    posted(fx.post(Who::Planner, ask(Who::Coder, "which crate?")));
    posted(fx.post(Who::Planner, pass()));
    let st = fold(&fx, &before_first_regrant(&fx));
    assert_eq!(ids(&wake_set(&st)), [pair("coder", WakeReason::Addressed)]);
}

#[test]
fn a_message_to_everyone_wakes_every_role_that_can_act_except_its_author() {
    let mut fx = Fx::new();
    let mut d = Draft::new(Finding, "heads up");
    d.to = vec![Recipient::All];
    let spec = fx.spec();
    d = d.refs(vec![Ref::art(spec)]).evidence(vec![Ref::art(spec)]);
    posted(fx.post(Who::Planner, d));
    posted(fx.post(Who::Planner, pass()));
    let w = wake_set(&fold(&fx, &before_first_regrant(&fx)));
    let got: Vec<&str> = w.iter().map(|(r, _)| r.0.as_str()).collect();
    assert!(!got.contains(&"planner"), "{got:?}");
    assert!(got.contains(&"coder") && got.contains(&"tester"), "{got:?}");
}

#[test]
fn a_role_is_not_woken_by_its_own_events_even_after_its_turn_ended() {
    // Cooldown alone already covers an author whose turn ends after it writes.
    // This splices in a message and an artifact stamped after the author's last
    // turn end, so only the self-wake exclusion keeps the author out.
    let fx = Fx::new();
    let mut evs = events(&fx);
    evs.extend(turn_of("planner", 90));
    let n = evs.len() as u64;
    evs.push(message(n, Some("planner"), Recipient::All, Ask));
    let mut st = fold(&fx, &evs);
    st.turn = None;
    let woken = wake_set(&st);
    let got: Vec<&str> = woken.iter().map(|(r, _)| r.0.as_str()).collect();
    assert!(!got.contains(&"planner"), "{got:?}");
    assert!(got.contains(&"coder"), "others are woken: {got:?}");
}

#[test]
fn a_pass_wakes_nobody() {
    let fx = Fx::new();
    let mut evs = events(&fx);
    let n = evs.len() as u64;
    evs.push(message(n, Some("planner"), Recipient::All, Pass));
    let mut st = fold(&fx, &evs);
    st.turn = None;
    let woken = wake_set(&st);
    let got: Vec<&str> = woken.iter().map(|(r, _)| r.0.as_str()).collect();
    assert_eq!(got, ["planner"], "only the state holder: {got:?}");
}

#[test]
fn a_cooled_down_role_is_woken_again_only_by_something_new() {
    let fx = Fx::new();
    let mut evs = events(&fx);
    evs.push(message(
        1,
        Some("planner"),
        Recipient::Role("coder".into()),
        Ask,
    ));
    // The coder's turn ends after the ask: nothing newer addresses it.
    evs.extend(turn_of("coder", 90));
    let st = fold(&fx, &evs);
    assert!(
        !ids(&wake_set(&st)).contains(&pair("coder", WakeReason::Addressed)),
        "cooled down"
    );
    // A new message after the turn end wakes it again.
    evs.push(message(2, None, Recipient::Role("coder".into()), Ask));
    let st = fold(&fx, &evs);
    assert!(ids(&wake_set(&st)).contains(&pair("coder", WakeReason::Addressed)));
}

#[test]
fn a_scoped_reader_is_woken_only_by_what_it_may_read() {
    let mut fx = Fx::new();
    fx.reach_review();
    let mut evs = events(&fx);
    let end = evs.len();
    // Another agent addresses the reviewer: invisible to a scoped reader.
    evs.insert(
        end,
        message(900, Some("tester"), Recipient::Role("reviewer".into()), Ask),
    );
    let mut st = fold(&fx, &evs);
    st.turn = None;
    let w = ids(&wake_set(&st));
    assert!(
        !w.contains(&pair("reviewer", WakeReason::Addressed)),
        "{w:?}"
    );
    assert!(
        w.iter().any(|(r, _)| r == "reviewer"),
        "still eligible: {w:?}"
    );
    // The human addressing it is visible.
    evs.push(message(901, None, Recipient::Role("reviewer".into()), Ask));
    let mut st = fold(&fx, &evs);
    st.turn = None;
    assert!(ids(&wake_set(&st)).contains(&pair("reviewer", WakeReason::Addressed)));
}

#[test]
fn a_role_that_could_not_act_is_not_woken() {
    // An observer holds no post kind; a pass-only role has nothing but `pass`.
    let mut roles = four_roles();
    roles.push(Role::new("watcher", 8).read_artifacts(&[Spec, Diff, Candidate, TestReport]));
    roles.push(
        Role::new("passer", 9)
            .post(&[Pass])
            .read_artifacts(&[Spec, Diff, Candidate, TestReport]),
    );
    let mut fx = Fx::with_roles(roles);
    posted(fx.post(Who::Planner, ask(Who::Coder, "q")));
    let mut d = Draft::new(Ask, "to all");
    d.to = vec![Recipient::All];
    posted(fx.post(Who::Planner, d));
    posted(fx.post(Who::Planner, pass()));
    let got: Vec<String> = wake_set(&fold(&fx, &before_first_regrant(&fx)))
        .into_iter()
        .map(|(r, _)| r.0)
        .collect();
    assert!(!got.contains(&"watcher".to_string()), "{got:?}");
    assert!(!got.contains(&"passer".to_string()), "{got:?}");
}

#[test]
fn a_verdict_poster_needs_authority_for_the_current_stage() {
    let verdicter = |id: &str, order, stages: &[Stage]| {
        Role::new(id, order)
            .post(&[MessageKind::Verdict])
            .verdict_at(stages)
    };
    let mut roles = four_roles();
    roles.push(verdicter("judge", 8, &[Stage::Test]));
    roles.push(verdicter("wrong", 9, &[Stage::Review]));
    roles.push(verdicter("none", 10, &[]));
    let mut fx = Fx::with_roles(roles);
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let mut st = idle(&fx);
    let mut d = Draft::new(Ask, "to all");
    d.to = vec![Recipient::All];
    st.messages.push(Message {
        id: MsgId(900),
        from: PrincipalId("human".into()),
        from_role: None,
        draft: d,
    });
    st.msg_rev.push(Rev(u64::MAX));
    let got: Vec<String> = wake_set(&st).into_iter().map(|(r, _)| r.0).collect();
    assert!(got.contains(&"judge".to_string()), "{got:?}");
    assert!(!got.contains(&"wrong".to_string()), "{got:?}");
    assert!(!got.contains(&"none".to_string()), "{got:?}");
}

/// Table: for build, test and review, every post kind and verdict-stage set, a role's
/// `can_act` agrees with what the real `post` path admits. Rejections that
/// are about the role, kind or state (`RoleForbidden`, `WrongState`,
/// `DecisionForbidden`) mean "not admitted"; any other outcome (accepted, or
/// rejected on content such as a stale candidate) means admitted.
#[test]
fn can_act_agrees_with_the_real_post_path() {
    use MessageKind::*;
    let kinds = [
        Ask,
        Answer,
        Propose,
        Finding,
        Verdict,
        RequestDecision,
        MessageKind::Pass,
        Decision,
    ];
    let stage_sets: [&[Stage]; 4] = [
        &[],
        &[Stage::Test],
        &[Stage::Review],
        &[Stage::Test, Stage::Review],
    ];
    let mut checked = 0;
    for stage in ["build", "test", "review"] {
        let mut roles = four_roles();
        for (i, k) in kinds.iter().enumerate() {
            for (j, set) in stage_sets.iter().enumerate() {
                roles.push(
                    Role::new(&format!("x{i}-{j}"), 100 + (i * 10 + j) as u16)
                        .post(&[*k])
                        .verdict_at(set),
                );
            }
        }
        let mut fx = Fx::with_roles(roles.clone());
        match stage {
            "build" => {
                fx.reach_build();
            }
            "test" => {
                fx.reach_test();
            }
            _ => {
                fx.reach_review();
            }
        }
        let live = fx.st().clone();
        let mut st = live.clone();
        st.turn = None;
        for role in roles.iter().filter(|r| r.id.0.starts_with('x')) {
            let kind = role.post[0];
            let mut with_turn = live.clone();
            let turn_id = TurnId(9_999);
            with_turn.turn = Some(rusty_bbp::state::Turn {
                id: turn_id,
                role: role.id.clone(),
                kind: TurnKind::Default,
                deadline: Time(u64::MAX),
                return_to: None,
                read_arts: Default::default(),
                state_at: with_turn.state,
                candidate_at: None,
                run_at: None,
            });
            let token = with_turn.token_for(&pid(role.id.clone()), turn_id);
            let cmd = Command::Agent {
                principal: pid(role.id.clone()),
                token: Some(token),
                op: OpId("probe".into()),
                action: AgentAction::Post(Draft::new(kind, "x")),
            };
            let admitted = !matches!(
                handle(&with_turn, &cmd, fx.now).response,
                Response::Rejected(r) if matches!(
                    r.code,
                    Code::RoleForbidden | Code::WrongState | Code::DecisionForbidden
                )
            );
            let expected = admitted && !matches!(kind, MessageKind::Pass | Decision);
            assert_eq!(can_act(&st, role), expected, "{stage} {kind:?} {role:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 3 * kinds.len() * stage_sets.len());
}

// ---------------------------------------------------------------- artifact

#[test]
fn a_stored_artifact_wakes_readers_that_can_act_but_not_its_author() {
    let mut fx = Fx::new();
    fx.reach_build();
    let diff = stored(fx.put(Who::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    assert!(fx.st().artifacts.contains_key(&diff));
    posted(fx.post(Who::Coder, pass()));
    let w = ids(&wake_set(&fold(&fx, &before_last_turn_end(&fx))));
    assert!(
        !w.iter().any(|(r, _)| *r == "coder"),
        "author excluded: {w:?}"
    );
    assert!(w.contains(&pair("tester", WakeReason::Artifact)), "{w:?}");
    assert!(w.contains(&pair("planner", WakeReason::Artifact)), "{w:?}");
}

fn before_last_turn_end(fx: &Fx) -> Vec<Event> {
    let evs = events(fx);
    let end = evs
        .iter()
        .rposition(|e| matches!(e, Event::TurnEnded { .. }))
        .expect("a turn ended");
    evs[..=end].to_vec()
}

// ---------------------------------------------------------------- order

#[test]
fn eligible_roles_are_ordered_by_reason_then_turn_order() {
    let mut roles = four_roles();
    // Two more spec writers and a second addressee, with turn orders chosen to
    // make the tiebreak visible.
    roles.push(
        Role::new("lead2", 7)
            .post(&[Ask, Answer, Finding, RequestDecision, Pass])
            .write(&[Spec])
            .read_artifacts(&[Spec]),
    );
    roles.push(
        Role::new("helper", 6)
            .post(&[Ask, Answer, Finding, Pass])
            .read_artifacts(&[Spec]),
    );
    let fx = Fx::with_roles(roles);
    let mut evs = events(&fx);
    evs.push(message(1, None, Recipient::Role("helper".into()), Ask));
    let mut st = fold(&fx, &evs);
    st.turn = None;
    let w = ids(&wake_set(&st));
    assert_eq!(
        w,
        [
            pair("helper", WakeReason::Addressed),
            pair("planner", WakeReason::State),
            pair("lead2", WakeReason::State),
        ]
    );
}

// ---------------------------------------------------------------- baseline

#[test]
fn a_role_with_no_turn_yet_counts_every_earlier_event_as_new() {
    // `seen_rev` starts at 0, so on reaching `build` the coder, which has not
    // had a turn, is woken by the spec the planner stored. Shadow mode measures
    // how much this inflates the set; changing the baseline is a separate call.
    let mut fx = Fx::new();
    fx.reach_build();
    let w = ids(&wake_set(&idle(&fx)));
    assert!(w.contains(&pair("coder", WakeReason::Artifact)), "{w:?}");
    assert!(w.contains(&pair("reviewer", WakeReason::Artifact)), "{w:?}");
}

// ---------------------------------------------------------------- fold-derived fields

#[test]
fn fold_fields_are_derived_from_the_log_alone() {
    let mut fx = Fx::new();
    fx.reach_review();
    let evs = events(&fx);
    let folded = fold(&fx, &evs);
    assert_eq!(
        &folded,
        fx.st(),
        "incremental state equals the fold of the log"
    );
    assert_eq!(folded.msg_rev.len(), folded.messages.len());
    assert_eq!(folded.art_rev.len(), folded.artifacts.len());
    // Every recorded rev is the position of its event in the log.
    for (m, rev) in folded.messages.iter().zip(&folded.msg_rev) {
        let at = evs
            .iter()
            .position(|e| matches!(e, Event::MessageAppended(x) if x.id == m.id))
            .expect("logged");
        assert_eq!(rev.0 as usize, at + 1);
    }
    for (role, rev) in &folded.last_turn_end {
        let ended = evs[..rev.0 as usize].last();
        assert!(matches!(ended, Some(Event::TurnEnded { .. })), "{role}");
    }
}
