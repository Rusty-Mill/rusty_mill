//! End-to-end dispatcher behaviour against the scripted fake.

use std::num::NonZeroU32;

use orch_core::board::{Author, Board, BoardError, Confidence, EntryKind, NewEntry, Verdict};
use orch_core::goal::{Goal, GoalDraft, StopRule};
use orch_core::task::{Agent, Plan, PlanError, Role, Status, TaskSpec, TaskState};
use orch_core::{GoalId, TaskId, Text};
use orch_dispatch::fake::{FakeAgent, Reply};
use orch_dispatch::{Ceiling, DispatchError, Dispatcher, Ledger, Outcome, Routing, RoutingConfig};

fn text(s: &str) -> Text {
    Text::new(s).expect("non-blank")
}

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).expect("non-zero")
}

fn goal(max_calls: u32) -> Goal {
    Goal::try_from(GoalDraft {
        outcome: Some("ship it".into()),
        done_when: vec!["tests pass".into()],
        out_of_scope: Some(vec![]),
        wall_clock_secs: Some(60),
        max_calls: Some(max_calls),
        stop: Some(StopRule::Checkpoint),
        ..GoalDraft::default()
    })
    .expect("valid goal")
}

fn spec(role: Role, depends_on: Vec<TaskId>, max_calls: u32) -> TaskSpec {
    TaskSpec {
        role,
        instruction: text("do the thing"),
        acceptance: vec![text("it is done")],
        refs: vec![],
        depends_on,
        max_calls: nz(max_calls),
    }
}

fn routing(implement: Agent, reviewers: Vec<Agent>) -> Routing {
    Routing::try_from(RoutingConfig {
        research: Agent::Claude,
        design: Agent::Claude,
        implement,
        triage: Agent::Local,
        reviewers,
    })
    .expect("valid routing")
}

/// research → implement → review, as three cards.
fn pipeline() -> (Plan, TaskId, TaskId, TaskId) {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let research = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let implement = plan
        .add(spec(Role::Implement, vec![research], 2))
        .expect("add");
    let review = plan
        .add(spec(Role::Review { target: implement }, vec![], 2))
        .expect("add");
    (plan, research, implement, review)
}

fn finding() -> EntryKind {
    EntryKind::Finding {
        confidence: Confidence::High,
    }
}

fn agent_of(plan: &Plan, id: TaskId) -> Agent {
    match plan.get(id).expect("task").state() {
        TaskState::Done { agent, .. } => *agent,
        other => panic!("{id} is {other:?}"),
    }
}

#[test]
fn pipeline_finishes_with_reviewer_distinct_from_author() {
    let (mut plan, research, implement, review) = pipeline();
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([
        Reply::Write(vec![finding()]),
        Reply::Write(vec![finding()]),
        Reply::Write(vec![EntryKind::Review {
            of: implement,
            verdict: Verdict::Approve,
        }]),
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Gemini, Agent::Codex]),
        fake,
    );
    let mut ledger = Ledger::new();

    let outcome = d
        .run(&goal(10), &mut plan, &mut board, &mut ledger)
        .expect("runs");

    assert_eq!(outcome, Outcome::Finished);
    assert!(plan.is_finished());
    assert_eq!(agent_of(&plan, implement), Agent::Codex);
    assert_eq!(agent_of(&plan, review), Agent::Gemini);
    assert_ne!(agent_of(&plan, review), agent_of(&plan, implement));
    assert_eq!(
        d.runner().calls(),
        &[
            (Agent::Claude, research),
            (Agent::Codex, implement),
            (Agent::Gemini, review)
        ]
    );
    assert_eq!(ledger.calls(), 3);
    assert_eq!(board.entries().len(), 3);
    assert!(board
        .entries()
        .iter()
        .all(|e| matches!(e.content().author, Author::Agent(_))));
}

#[test]
fn review_falls_back_when_primary_reviewer_is_the_author() {
    let (mut plan, _, implement, review) = pipeline();
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([
        Reply::Write(vec![finding()]),
        Reply::Write(vec![finding()]),
        Reply::Write(vec![EntryKind::Review {
            of: implement,
            verdict: Verdict::ChangesRequested,
        }]),
    ]);
    // Codex is both the implementer and the preferred reviewer.
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();

    d.run(&goal(10), &mut plan, &mut board, &mut ledger)
        .expect("runs");

    assert_eq!(agent_of(&plan, implement), Agent::Codex);
    assert_eq!(agent_of(&plan, review), Agent::Gemini);
}

#[test]
fn routing_rejects_fewer_than_two_distinct_reviewers() {
    let err = Routing::try_from(RoutingConfig {
        research: Agent::Claude,
        design: Agent::Claude,
        implement: Agent::Claude,
        triage: Agent::Claude,
        reviewers: vec![Agent::Codex, Agent::Codex],
    })
    .expect_err("one distinct reviewer");
    assert_eq!(
        err,
        orch_dispatch::RoutingError::TooFewReviewers { distinct: 1 }
    );
}

#[test]
fn question_blocks_and_answer_resumes() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![finding()]),
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    let first = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect("blocks");
    assert_eq!(first, Outcome::Blocked(vec![task]));
    assert_eq!(
        plan.get(task).expect("task").state().status(),
        Status::Blocked
    );
    let question = board.open_questions()[0].id();

    // Running again without an answer makes no call and stays blocked.
    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger)
            .expect("still blocked"),
        Outcome::Blocked(vec![task])
    );
    assert_eq!(ledger.calls(), 1);

    board
        .append(NewEntry {
            task: Some(task),
            author: Author::Human,
            kind: EntryKind::Answer { to: question },
            body: text("yes"),
            refs: vec![],
            supersedes: None,
        })
        .expect("answer");

    let second = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect("finishes");
    assert_eq!(second, Outcome::Finished);
    assert_eq!(ledger.calls(), 2);
    assert_eq!(
        d.runner().calls(),
        &[(Agent::Claude, task), (Agent::Claude, task)]
    );
}

#[test]
fn goal_ceiling_stops_the_loop_with_typed_error() {
    let (mut plan, _, implement, _) = pipeline();
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new(std::iter::repeat_with(|| Reply::Write(vec![finding()])).take(3));
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();

    let err = d
        .run(&goal(1), &mut plan, &mut board, &mut ledger)
        .expect_err("ceiling");

    assert_eq!(
        err,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Goal,
            task: implement,
            limit: nz(1),
        }
    );
    assert_eq!(ledger.calls(), 1);
    // The refused card was never started.
    assert_eq!(
        plan.get(implement).expect("task").state(),
        &TaskState::Pending
    );
}

#[test]
fn task_ceiling_stops_the_loop_with_typed_error() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 1)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([
        Reply::Write(vec![EntryKind::Question]),
        Reply::Write(vec![finding()]),
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    d.run(&goal, &mut plan, &mut board, &mut ledger)
        .expect("blocks");
    answer(&mut board, task);

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("ceiling");

    assert_eq!(
        err,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Task,
            task,
            limit: nz(1),
        }
    );
    assert_eq!(ledger.calls(), 1);
    assert_eq!(
        plan.get(task).expect("task").state().status(),
        Status::Failed
    );
}

#[test]
fn zero_entries_surfaces_no_outputs() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([Reply::Write(vec![])]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();

    let err = d
        .run(&goal(10), &mut plan, &mut board, &mut ledger)
        .expect_err("no outputs");

    assert_eq!(err, DispatchError::Plan(PlanError::NoOutputs(task)));
}

#[test]
fn agent_failure_is_typed_and_leaves_the_card_running() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([Reply::Fail("boom".into()), Reply::Write(vec![finding()])]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("agent error");
    assert!(matches!(err, DispatchError::Agent { task: t, agent: Agent::Claude, .. } if t == task));
    assert_eq!(
        plan.get(task).expect("task").state().status(),
        Status::Running
    );

    // A second run retries the card under the same ceilings.
    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger)
            .expect("retries"),
        Outcome::Finished
    );
    assert_eq!(ledger.calls(), 2);
}

fn answer(board: &mut Board, task: TaskId) {
    let question = board.open_questions()[0].id();
    board
        .append(NewEntry {
            task: Some(task),
            author: Author::Human,
            kind: EntryKind::Answer { to: question },
            body: text("yes"),
            refs: vec![],
            supersedes: None,
        })
        .expect("answer");
}

#[test]
fn invalid_output_leaves_board_unchanged_and_card_running() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    // Second output is an artifact with no refs, which the board rejects.
    let fake = FakeAgent::new([Reply::Write(vec![finding(), EntryKind::Artifact])]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let before = board.clone();

    let err = d
        .run(&goal(10), &mut plan, &mut board, &mut ledger)
        .expect_err("rejected");

    assert_eq!(err, DispatchError::Board(BoardError::ArtifactWithoutRefs));
    assert_eq!(board, before);
    assert_eq!(
        plan.get(task).expect("task").state().status(),
        Status::Running
    );
    assert_eq!(ledger.calls(), 1);
    assert_eq!(ledger.task_calls(task), 1);
}

#[test]
fn fresh_dispatcher_with_same_ledger_still_hits_goal_ceiling() {
    let (mut plan, research, implement, _) = pipeline();
    let mut board = Board::new(plan.goal());
    let goal = goal(1);
    let mut ledger = Ledger::new();

    let mut first = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        FakeAgent::new([Reply::Write(vec![finding()])]),
    );
    let err = first
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("ceiling");
    assert!(matches!(
        err,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Goal,
            ..
        }
    ));
    assert_eq!(
        plan.get(research).expect("task").state().status(),
        Status::Done
    );
    drop(first);

    let mut second = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        FakeAgent::new([Reply::Write(vec![finding()])]),
    );
    let err = second
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("ceiling persists");

    assert_eq!(
        err,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Goal,
            task: implement,
            limit: nz(1),
        }
    );
    assert!(second.runner().calls().is_empty());
    assert_eq!(ledger.calls(), 1);
}

#[test]
fn repeated_agent_failure_exhausts_retries_and_strands_dependents() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let dependent = plan.add(spec(Role::Implement, vec![task], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = FakeAgent::new([Reply::Fail("boom".into()), Reply::Fail("boom".into())]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    for _ in 0..2 {
        let err = d
            .run(&goal, &mut plan, &mut board, &mut ledger)
            .expect_err("agent error");
        assert!(matches!(err, DispatchError::Agent { .. }));
        assert_eq!(
            plan.get(task).expect("task").state().status(),
            Status::Running
        );
    }

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("exhausted");
    assert_eq!(
        err,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Task,
            task,
            limit: nz(2),
        }
    );
    match plan.get(task).expect("task").state() {
        TaskState::Failed { reason, .. } => assert!(reason.as_str().contains("Task ceiling")),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(ledger.calls(), 2);

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("stuck");
    assert_eq!(err, DispatchError::Stuck);
    assert_eq!(
        plan.get(dependent).expect("task").state(),
        &TaskState::Pending
    );
}
