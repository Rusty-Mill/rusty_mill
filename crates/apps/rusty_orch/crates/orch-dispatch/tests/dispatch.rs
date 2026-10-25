//! End-to-end dispatcher behaviour against the scripted fake.

use std::collections::VecDeque;
use std::num::NonZeroU32;

use orch_core::board::{Author, Board, BoardError, Confidence, EntryKind, NewEntry, Verdict};
use orch_core::goal::{Goal, GoalDraft, StopRule};
use orch_core::task::{Agent, Plan, PlanError, Role, Status, Task, TaskSpec, TaskState};
use orch_core::{GoalId, TaskId, Text};
use orch_dispatch::fake::{FakeAgent, Reply};
use orch_dispatch::{
    AgentError, AgentRunner, Ceiling, ClassifiedError, DispatchError, Dispatcher, Ledger, Outcome,
    Output, Routing, RoutingConfig,
};

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
    assert_eq!(ledger.task_calls(task), 2);
    assert_eq!(board.entries().len(), 1);
}

#[derive(Debug)]
struct NoImplement(FakeAgent);

impl AgentRunner for NoImplement {
    fn supports(&self, _agent: Agent, role: Role) -> bool {
        role != Role::Implement
    }

    fn run(
        &mut self,
        agent: Agent,
        task: &orch_core::task::Task,
        board: &Board,
    ) -> Result<Vec<Output>, AgentError> {
        self.0.run(agent, task, board)
    }
}

#[derive(Debug)]
enum TypedReply {
    Write,
    Fail(ClassifiedError),
}

#[derive(Debug)]
struct TypedRunner {
    replies: VecDeque<TypedReply>,
    calls: usize,
}

impl TypedRunner {
    fn new(replies: impl IntoIterator<Item = TypedReply>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            calls: 0,
        }
    }

    fn next(&mut self) -> Result<Vec<Output>, ClassifiedError> {
        self.calls += 1;
        match self.replies.pop_front() {
            Some(TypedReply::Write) => Ok(vec![Output {
                kind: finding(),
                body: text("typed result"),
                refs: vec![],
                supersedes: None,
            }]),
            Some(TypedReply::Fail(error)) => Err(error),
            None => Err(ClassifiedError::Transient(AgentError(
                "typed script exhausted".into(),
            ))),
        }
    }
}

impl AgentRunner for TypedRunner {
    fn run(&mut self, _: Agent, _: &Task, _: &Board) -> Result<Vec<Output>, AgentError> {
        self.next().map_err(ClassifiedError::into_error)
    }

    fn run_classified(
        &mut self,
        _: Agent,
        _: &Task,
        _: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        self.next()
    }
}

#[derive(Debug)]
struct Composite(TypedRunner);

impl AgentRunner for Composite {
    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError> {
        self.0.run(agent, task, board)
    }

    fn run_classified(
        &mut self,
        agent: Agent,
        task: &Task,
        board: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        self.0.run_classified(agent, task, board)
    }
}

#[test]
fn permanent_agent_failure_fails_the_card_once_and_strands_dependents() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 3)).expect("add");
    let dependent = plan.add(spec(Role::Design, vec![task], 3)).expect("add");
    let mut board = Board::new(plan.goal());
    // A second reply is scripted so a wrongful retry would be visible.
    let fake = TypedRunner::new([
        TypedReply::Fail(ClassifiedError::Permanent(AgentError("refused".into()))),
        TypedReply::Write,
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("permanent agent error");
    assert!(matches!(
        &err,
        DispatchError::Agent { task: t, agent: Agent::Claude, source: AgentError(m) }
            if *t == task && m == "refused"
    ));
    assert_eq!(
        plan.get(task).expect("task").state().status(),
        Status::Failed
    );
    assert_eq!(ledger.calls(), 1, "the failing call is still metered");

    // Nothing is retried: the card is failed, its dependent can never run.
    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger),
        Err(DispatchError::Stuck)
    );
    assert_eq!(ledger.calls(), 1);
    assert_eq!(d.runner().calls, 1);
    assert_eq!(
        plan.get(dependent).expect("task").state().status(),
        Status::Pending
    );
    assert!(board.entries().is_empty());
}

#[test]
fn unavailable_prerequisite_can_repeat_then_resume_at_max_calls() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 1)).expect("add");
    let mut board = Board::new(plan.goal());
    let fake = Composite(TypedRunner::new([
        TypedReply::Fail(ClassifiedError::Unavailable(AgentError(
            "not logged in".into(),
        ))),
        TypedReply::Fail(ClassifiedError::Unavailable(AgentError(
            "still not logged in".into(),
        ))),
        TypedReply::Write,
    ]));
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        fake,
    );
    let mut ledger = Ledger::new();
    let goal = goal(1);

    for reason in ["not logged in", "still not logged in"] {
        let err = d
            .run(&goal, &mut plan, &mut board, &mut ledger)
            .expect_err("external prerequisite is absent");
        assert!(matches!(
            err,
            DispatchError::Agent { source: AgentError(message), .. } if message == reason
        ));
        assert_eq!(
            plan.get(task).expect("task").state().status(),
            Status::Running
        );
        assert_eq!(
            ledger.calls(),
            0,
            "unavailable attempts do not consume budget"
        );
        assert_eq!(ledger, Ledger::new(), "rollback removes zero-count keys");
        assert!(board.entries().is_empty());
    }

    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger)
            .expect("same card resumes after the prerequisite is restored"),
        Outcome::Finished
    );
    assert_eq!(ledger.calls(), 1);
    assert_eq!(ledger.task_calls(task), 1);
    assert_eq!(d.runner().0.calls, 3);
}

#[test]
fn unavailable_restores_only_its_charge_with_prior_usage() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Research, vec![], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let runner = TypedRunner::new([
        TypedReply::Fail(ClassifiedError::Transient(AgentError("retry".into()))),
        TypedReply::Fail(ClassifiedError::Unavailable(AgentError("login".into()))),
        TypedReply::Write,
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        runner,
    );
    let mut ledger = Ledger::new();
    let goal = goal(2);

    assert!(matches!(
        d.run(&goal, &mut plan, &mut board, &mut ledger),
        Err(DispatchError::Agent { .. })
    ));
    assert_eq!((ledger.calls(), ledger.task_calls(task)), (1, 1));
    assert!(matches!(
        d.run(&goal, &mut plan, &mut board, &mut ledger),
        Err(DispatchError::Agent { .. })
    ));
    assert_eq!((ledger.calls(), ledger.task_calls(task)), (1, 1));
    assert!(board.entries().is_empty());

    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Finished)
    );
    assert_eq!((ledger.calls(), ledger.task_calls(task)), (2, 2));
    assert_eq!(d.runner().calls, 3);
}

#[test]
fn exhausted_budget_refuses_without_invocation_or_recovery_credit() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let first = plan.add(spec(Role::Research, vec![], 1)).expect("add");
    let second = plan.add(spec(Role::Design, vec![], 1)).expect("add");
    let mut board = Board::new(plan.goal());
    let runner = TypedRunner::new([
        TypedReply::Write,
        TypedReply::Fail(ClassifiedError::Unavailable(AgentError("login".into()))),
    ]);
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        runner,
    );
    let mut ledger = Ledger::new();

    let error = d
        .run(&goal(1), &mut plan, &mut board, &mut ledger)
        .expect_err("goal budget is genuinely exhausted");
    assert!(matches!(
        error,
        DispatchError::CeilingReached {
            ceiling: Ceiling::Goal,
            task,
            ..
        } if task == second
    ));
    assert_eq!((ledger.calls(), ledger.task_calls(first)), (1, 1));
    assert_eq!(ledger.task_calls(second), 0);
    assert_eq!(d.runner().calls, 1, "no unavailable call was attempted");
    assert_eq!(board.entries().len(), 1);
}

#[test]
fn downstream_legacy_api_remains_exhaustively_matchable() {
    fn legacy_call(runner: &mut impl AgentRunner, task: &Task, board: &Board) -> String {
        match runner.run(Agent::Claude, task, board) {
            Ok(_) => String::new(),
            Err(error) => error.0,
        }
    }

    fn reply(reply: Reply) {
        match reply {
            Reply::Write(_) | Reply::Fail(_) => {}
        }
    }

    fn dispatch(error: DispatchError) {
        match error {
            DispatchError::Plan(_)
            | DispatchError::Board(_)
            | DispatchError::Agent { .. }
            | DispatchError::UnsupportedRole { .. }
            | DispatchError::CeilingReached { .. }
            | DispatchError::Unroutable { .. }
            | DispatchError::Stuck => {}
        }
    }

    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Research, vec![], 1)).expect("add");
    let board = Board::new(plan.goal());
    let mut runner = FakeAgent::new([Reply::Fail("legacy".into())]);
    assert_eq!(
        legacy_call(&mut runner, plan.get(id).expect("task"), &board),
        "legacy"
    );
    reply(Reply::Fail("still exhaustive".into()));
    dispatch(DispatchError::Stuck);
}

#[test]
fn unsupported_role_fails_once_without_call_or_fabricated_output() {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let task = plan.add(spec(Role::Implement, vec![], 5)).expect("add");
    let dependent = plan.add(spec(Role::Research, vec![task], 2)).expect("add");
    let mut board = Board::new(plan.goal());
    let runner = NoImplement(FakeAgent::new([Reply::Write(vec![finding()])]));
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Codex, Agent::Gemini]),
        runner,
    );
    let mut ledger = Ledger::new();
    let goal = goal(10);

    let err = d
        .run(&goal, &mut plan, &mut board, &mut ledger)
        .expect_err("unsupported");
    assert_eq!(
        err,
        DispatchError::UnsupportedRole {
            task,
            agent: Agent::Codex,
            role: Role::Implement,
        }
    );
    assert!(matches!(
        plan.get(task).expect("task").state(),
        TaskState::Failed { reason, .. }
            if reason.as_str().contains("cannot serve Implement")
    ));
    assert_eq!(
        plan.get(dependent).expect("dependent").state(),
        &TaskState::Pending
    );
    assert_eq!(ledger.calls(), 0);
    assert_eq!(ledger.task_calls(task), 0);
    assert!(board.entries().is_empty());
    assert!(d.runner().0.calls().is_empty());

    assert_eq!(
        d.run(&goal, &mut plan, &mut board, &mut ledger)
            .expect_err("terminal card is not retried"),
        DispatchError::Stuck
    );
    assert_eq!(ledger.calls(), 0);
    assert!(d.runner().0.calls().is_empty());
    assert!(board.entries().is_empty());
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

fn fence(generation: u64, holder: &str) -> orch_dispatch::Fence {
    orch_dispatch::Fence {
        assignment: text("goal-orchestrator"),
        generation,
        holder: text(holder),
        lease: None,
    }
}

fn gate(rev: u64) -> orch_dispatch::Progress {
    orch_dispatch::Progress::Gate(orch_dispatch::GateContext {
        prompt: text("approve-plan art:2"),
        rev,
        ids: vec![text("art:2")],
    })
}

#[test]
fn hosted_cards_resume_gates_and_complete_once_without_runner_calls() {
    use orch_dispatch::{fake::FakeHost, Progress, Relayed};
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Implement, vec![], 1)).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    host.0
        .borrow_mut()
        .progress
        .extend([Ok(Progress::Working), Ok(gate(10))]);
    let make_dispatcher = || {
        Dispatcher::new(
            routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]),
            FakeAgent::new([]),
        )
        .with_host(host.clone(), fence(1, "first"))
    };
    let mut dispatcher = make_dispatcher();
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Waiting(vec![id]))
    );
    assert_eq!(ledger.calls(), 1);
    // Drop all dispatcher state; only the caller-owned snapshot survives.
    drop(dispatcher);
    let mut dispatcher = make_dispatcher();
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Blocked(vec![id]))
    );
    let old = board.open_questions()[0].id();
    board
        .append(NewEntry {
            task: Some(id),
            author: Author::Human,
            kind: EntryKind::Answer { to: old },
            body: text("approve-plan art:2"),
            refs: vec![],
            supersedes: None,
        })
        .unwrap();
    host.0
        .borrow_mut()
        .relays
        .push_back(Ok(Relayed::Stale(gate(20))));
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Blocked(vec![id]))
    );
    let fresh = board.open_questions()[0].id();
    assert_ne!(old, fresh);
    assert!(board.is_superseded(old));
    let recorded = host.0.borrow().answers[0].0.clone();
    assert!(recorded
        .content()
        .refs
        .iter()
        .any(|r| matches!(r, orch_core::Ref::Url(t) if t.as_str() == "host:rev:10")));
    board
        .append(NewEntry {
            task: Some(id),
            author: Author::Human,
            kind: EntryKind::Answer { to: fresh },
            body: text("approve-plan art:2"),
            refs: vec![],
            supersedes: None,
        })
        .unwrap();
    host.0.borrow_mut().progress.push_back(Ok(Progress::Closed {
        outputs: vec![Output {
            kind: EntryKind::Artifact,
            body: text("receipt"),
            refs: vec![orch_core::Ref::Commit(text("abc"))],
            supersedes: None,
        }],
    }));
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Finished)
    );
    assert_eq!(host.0.borrow().opens, 1);
    assert_eq!(ledger.calls(), 1);
}

#[test]
fn hosted_failure_or_empty_completion_does_not_finish_a_card() {
    use orch_dispatch::{fake::FakeHost, Progress};
    for progress in [
        Progress::Closed { outputs: vec![] },
        Progress::Cancelled {
            reason: text("cancelled by owner"),
        },
    ] {
        let mut plan = Plan::new(GoalId::from_raw(1));
        let id = plan.add(spec(Role::Implement, vec![], 1)).unwrap();
        let mut board = Board::new(plan.goal());
        let mut ledger = Ledger::new();
        let host = FakeHost::default();
        host.0.borrow_mut().progress.push_back(Ok(progress));
        let mut d = Dispatcher::new(
            routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]),
            FakeAgent::new([]),
        )
        .with_host(host, fence(1, "first"));
        assert!(d.run(&goal(1), &mut plan, &mut board, &mut ledger).is_err());
        assert_ne!(plan.get(id).unwrap().state().status(), Status::Done);
        assert_eq!(ledger.calls(), 1);
    }
}

#[test]
fn hosting_does_not_reroute_research_or_design() {
    use orch_dispatch::fake::FakeHost;
    let mut plan = Plan::new(GoalId::from_raw(1));
    plan.add(spec(Role::Research, vec![], 1)).unwrap();
    plan.add(spec(Role::Design, vec![], 1)).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    let mut d = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]),
        FakeAgent::new([Reply::Write(vec![finding()]), Reply::Write(vec![finding()])]),
    )
    .with_host(host.clone(), fence(1, "first"));
    assert_eq!(
        d.run(&goal(2), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Finished)
    );
    assert_eq!(host.0.borrow().opens, 0);
    assert_eq!(ledger.calls(), 2);
}

#[test]
fn an_agent_answer_cannot_approve_a_hosted_task() {
    use orch_dispatch::fake::FakeHost;
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Implement, vec![], 1)).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    host.0.borrow_mut().progress.push_back(Ok(gate(1)));
    let mut dispatcher = Dispatcher::new(
        routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]),
        FakeAgent::new([]),
    )
    .with_host(host.clone(), fence(1, "first"));
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Blocked(vec![id]))
    );
    let question = board.open_questions()[0].id();
    board
        .append(NewEntry {
            task: Some(id),
            author: Author::Agent(Agent::Claude),
            kind: EntryKind::Answer { to: question },
            body: text("approve-plan art:2"),
            refs: vec![],
            supersedes: None,
        })
        .unwrap();
    assert_eq!(
        dispatcher.run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Blocked(vec![id]))
    );
    assert!(host.0.borrow().answers.is_empty());
}

#[test]
fn a_saved_host_handle_cannot_fall_back_to_a_one_call_runner() {
    use orch_dispatch::{fake::FakeHost, Progress};
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Implement, vec![], 1)).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    host.0
        .borrow_mut()
        .progress
        .push_back(Ok(Progress::Working));
    let route = routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]);
    Dispatcher::new(route.clone(), FakeAgent::new([]))
        .with_host(host, fence(1, "first"))
        .run(&goal(1), &mut plan, &mut board, &mut ledger)
        .unwrap();
    let result = Dispatcher::new(route, FakeAgent::new([])).run(
        &goal(1),
        &mut plan,
        &mut board,
        &mut ledger,
    );
    assert!(
        matches!(result,Err(DispatchError::Agent{source,..}) if source.0.contains("requires its TaskHost"))
    );
    assert_eq!(ledger.calls(), 1);
    assert_eq!(plan.get(id).unwrap().state().status(), Status::Running);
}

#[test]
fn a_superseded_holder_stops_without_changing_the_card_or_its_charge() {
    use orch_dispatch::{fake::FakeHost, Progress};
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(spec(Role::Implement, vec![], 1)).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let host = FakeHost::default();
    let dispatcher = |fence| {
        Dispatcher::new(
            routing(Agent::Codex, vec![Agent::Claude, Agent::Codex]),
            FakeAgent::new([]),
        )
        .with_host(host.clone(), fence)
    };
    assert_eq!(
        dispatcher(fence(1, "first")).run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Waiting(vec![id]))
    );
    // A later holder polls once; the earlier one is now refused.
    assert_eq!(
        dispatcher(fence(2, "second")).run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Waiting(vec![id]))
    );
    let before = (plan.clone(), board.clone(), ledger.calls());
    for stale in [fence(1, "first"), fence(2, "diverged")] {
        assert!(matches!(
            dispatcher(stale).run(&goal(1), &mut plan, &mut board, &mut ledger),
            Err(DispatchError::Agent { task, .. }) if task == id
        ));
        assert_eq!((plan.clone(), board.clone(), ledger.calls()), before);
    }
    host.0.borrow_mut().progress.push_back(Ok(Progress::Closed {
        outputs: vec![orch_dispatch::Output {
            kind: EntryKind::Artifact,
            body: text("merged"),
            refs: vec![orch_core::Ref::Commit(text("abc123"))],
            supersedes: None,
        }],
    }));
    assert_eq!(
        dispatcher(fence(2, "second")).run(&goal(1), &mut plan, &mut board, &mut ledger),
        Ok(Outcome::Finished)
    );
    assert_eq!(host.0.borrow().opens, 1);
}
