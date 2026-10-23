#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use orch_core::board::{Author, NewEntry};
use orch_core::task::{Plan, Role as OrchRole, TaskSpec};
use orch_core::{GoalId, Ref, TaskId};
use rusty_bbp::*;
use rusty_serde::json;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(super) fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "orch_bbp_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
pub(super) fn txt(s: &str) -> Text {
    Text::new(s).unwrap()
}
/// The fence of a test holder at `generation`.
pub(super) fn fence_at(generation: u64, holder: &str) -> Fence {
    Fence {
        assignment: txt("goal-orchestrator"),
        generation,
        holder: txt(holder),
    }
}
pub(super) fn fence() -> Fence {
    fence_at(1, "test")
}
pub(super) fn config(dir: &std::path::Path) -> Config {
    use ArtifactKind::*;
    use MessageKind::*;
    let profiles = dir.join("input-profiles.json");
    std::fs::write(&profiles, "{}").unwrap();
    Config {
        binary: "bbp".into(),
        dir: dir.join("store"),
        repo: dir.join("repo"),
        work: dir.join("work"),
        profiles,
        pool: BTreeMap::from([
            (
                "writer".into(),
                Agent {
                    vendor: "ollama".into(),
                    program: "ollama".into(),
                    args: vec!["{task}".into()],
                    sandbox: None,
                },
            ),
            (
                "judge".into(),
                Agent {
                    vendor: "future-provider".into(),
                    program: "future-cli".into(),
                    args: vec!["{mcp_config}".into()],
                    sandbox: None,
                },
            ),
        ]),
        roster: vec![
            Role::new("maker", 0)
                .post(&[RequestDecision, Verdict, Pass])
                .write(&[Spec, Diff, Candidate])
                .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
                .verdict_at(&[Stage::Test]),
            Role::new("auditor", 1)
                .post(&[Verdict, Pass])
                .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
                .scoped()
                .verdict_at(&[Stage::Review])
                .final_decision(),
        ],
        assignments: BTreeMap::from([
            (
                "maker".into(),
                Assignment {
                    agent: "writer".into(),
                    builtin_tools: false,
                },
            ),
            (
                "auditor".into(),
                Assignment {
                    agent: "judge".into(),
                    builtin_tools: false,
                },
            ),
        ]),
        live_turns: 1,
        max_wall_secs: 60,
        end_grace_secs: None,
        slots: None,
    }
}
pub(super) fn task() -> (Task, Board) {
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan
        .add(TaskSpec {
            role: OrchRole::Implement,
            instruction: txt("implement"),
            acceptance: vec![txt("tested")],
            refs: vec![],
            depends_on: vec![],
            max_calls: NonZeroU32::new(1).unwrap(),
        })
        .unwrap();
    (plan.get(id).unwrap().clone(), Board::new(plan.goal()))
}
#[derive(Debug, Default, Clone)]
pub(super) struct FakeSupervisor(Rc<Cell<usize>>);
impl Supervisor for FakeSupervisor {
    fn ensure(&mut self, _: &Config, _: &str, _: &std::path::Path) -> Result<Supervised, String> {
        self.0.set(self.0.get() + 1);
        Ok(Supervised::Running)
    }
}

/// A finite-process double with scripted protocol replies.
/// Failure injection occurs after a side effect, as when its reply is lost.
#[derive(Debug, Clone, Default)]
struct Contract {
    calls: Rc<RefCell<Vec<Vec<String>>>>,
    replies: Rc<RefCell<std::collections::VecDeque<String>>>,
    fail_at: Rc<Cell<Option<usize>>>,
}
impl CommandRunner for Contract {
    fn run_in(
        &self,
        cwd: Option<&std::path::Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        scrub: &[&str],
    ) -> Result<orch_cli::Exit, ExecError> {
        assert!(cwd.is_none() && stdin.is_empty());
        assert_eq!(timeout, Duration::from_secs(30));
        assert!(scrub.contains(&"GH_TOKEN") && scrub.contains(&"ANTHROPIC_API_KEY"));
        self.calls.borrow_mut().push(argv.to_vec());
        let reply = self
            .replies
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| "null".into());
        if self.fail_at.get() == Some(self.calls.borrow().len()) {
            return Err(ExecError::Timeout(timeout));
        }
        Ok(orch_cli::Exit {
            status: 0,
            stdout: reply.into_bytes(),
            stderr: vec![],
        })
    }
}
fn view(config: &Config, state: State) -> TaskView {
    let mut st = TaskState::new(rusty_bbp::TaskId("test".into()));
    st.state = state;
    st.rev = Rev(12);
    st.candidate = Some(ArtId(4));
    st.run = Some(rusty_bbp::state::Run {
        id: RunId(1),
        candidate: ArtId(4),
        status: Some(RunStatus::Passed),
        report: Some(ArtId(6)),
        log: Some(ArtId(5)),
    });
    let mut v = TaskView::from_events(st.task.clone(), &[]);
    v.card = st.card();
    v.repo = config.repo.to_string_lossy().into();
    v.roster = config.roster.clone();
    v.proposal = Some(ArtId(2));
    v
}
fn json<T: Serialize>(t: &T) -> String {
    json::to_string(t).unwrap()
}

#[test]
fn pools_are_dynamic_and_rosters_are_capabilities_not_named_roles() {
    let dir = temp();
    let mut c = config(&dir);
    c.validate().unwrap();
    c.pool.get_mut("judge").unwrap().vendor = "ollama".into();
    assert!(c.validate().unwrap_err().contains("independent"));
    c.pool.get_mut("judge").unwrap().vendor = "claude".into();
    c.assignments.get_mut("auditor").unwrap().builtin_tools = true;
    assert!(c.validate().is_err());
    c.assignments.get_mut("auditor").unwrap().builtin_tools = false;
    c.roster.push(Role::new("observer", 2));
    c.assignments.insert(
        "observer".into(),
        Assignment {
            agent: "judge".into(),
            builtin_tools: false,
        },
    );
    c.validate().unwrap();
    c.live_turns = 4;
    assert!(c.validate().is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn slots_are_optional_must_admit_each_class_and_are_pinned_absolute() {
    let dir = temp();
    let mut c = config(&dir);
    let text = rusty_serde::json::to_string(&c).unwrap();
    let without: Config =
        rusty_serde::json::from_str(&text.replace(",\"slots\":null", "")).unwrap();
    assert_eq!(
        without.slots, None,
        "a manifest frozen before slots still loads"
    );
    c.slots = Some(crate::Slots {
        dir: "slots".into(),
        agents: 2,
        validators: 0,
    });
    assert!(c.validate().unwrap_err().contains("slots"));
    c.slots.as_mut().unwrap().validators = 1;
    c.validate().unwrap();
    let pinned = c.absolute().unwrap();
    assert!(pinned.slots.unwrap().dir.is_absolute());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn open_decodes_exit_zero_already_open_and_resumes_partial_assignment() {
    let dir = temp();
    let c = config(&dir);
    let (task, board) = task();
    let command = Contract::default();
    command.replies.borrow_mut().extend([
        "null".into(),
        json(&Response::Rejected(Rejection::new(
            Code::AlreadyOpen,
            "race",
        ))),
        json(&Some(view(&c, State::Planning))),
        json(&Response::Ok),
        json(&Response::Ok),
    ]);
    let sup = FakeSupervisor::default();
    let mut host = BbpHost::with_seams(
        c.clone(),
        "goal".into(),
        "contract".into(),
        command.clone(),
        sup.clone(),
    )
    .unwrap();
    let handle = host.open(&task, &board, &fence()).unwrap();
    let calls = command.calls.borrow();
    assert_eq!(
        calls.iter().map(|a| a[1].as_str()).collect::<Vec<_>>(),
        ["show", "open", "show", "assign", "assign"]
    );
    assert!(calls[1].contains(&"--roster".into()));
    assert!(calls[3].windows(2).any(|a| a == ["--vendor", "ollama"]));
    drop(calls);
    assert_eq!(sup.0.get(), 1);
    // A later pool configuration cannot overwrite this task's launch policy.
    let mut next = c.clone();
    next.pool.get_mut("writer").unwrap().vendor = "codex".into();
    let mut adopted_view = view(&c, State::Planning);
    adopted_view.principals = c
        .roster
        .iter()
        .map(|r| {
            let a = &c.assignments[&r.id.0];
            Principal {
                id: PrincipalId(format!("orch-{}", r.id.0)),
                kind: PrincipalKind::Agent {
                    role: r.id.clone(),
                    vendor: c.pool[&a.agent].vendor.clone(),
                },
            }
        })
        .collect();
    command
        .replies
        .borrow_mut()
        .extend([json(&Some(adopted_view.clone())), json(&Some(adopted_view))]);
    let mut resumed =
        BbpHost::with_seams(next, "goal".into(), "changed".into(), command.clone(), sup).unwrap();
    assert_eq!(resumed.open(&task, &board, &fence()).unwrap(), handle);
    let h = resumed.handle(&handle).unwrap();
    assert_eq!(
        BbpHost::<Contract, FakeSupervisor>::load(&h)
            .unwrap()
            .config,
        c
    );
    assert_eq!(
        command
            .calls
            .borrow()
            .iter()
            .filter(|a| a[1] == "open")
            .count(),
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn question(board: &mut Board, handle: &HostHandle) -> Entry {
    let id = board
        .append(NewEntry {
            task: Some(TaskId::from_raw(1)),
            author: Author::Human,
            kind: EntryKind::Question,
            body: txt("approve-plan art:2"),
            refs: vec![
                Ref::Url(handle.0.clone()),
                Ref::Url(txt("host:rev:12")),
                Ref::Url(txt("art:2")),
            ],
            supersedes: None,
        })
        .unwrap();
    board.get(id).unwrap().clone()
}

#[test]
fn relay_uses_saved_fence_and_stable_op_and_rejects_subject_or_flag_injection() {
    let dir = temp();
    let c = config(&dir);
    let (task, mut board) = task();
    let cmd = Contract::default();
    let sup = FakeSupervisor::default();
    let mut host =
        BbpHost::with_seams(c.clone(), "goal".into(), "context".into(), cmd.clone(), sup).unwrap();
    let h = Handle {
        dir: c.dir.clone(),
        task: "task".into(),
    };
    host.freeze(&h, &task, &board).unwrap();
    let handle = HostHandle(txt(&format!("bbp:{}", json(&h))));
    let gate = question(&mut board, &handle);
    cmd.replies.borrow_mut().extend([
        json(&Response::Rejected(Rejection::new(
            Code::StaleRev,
            "changed",
        ))),
        json(&Some(view(&c, State::PlanGate))),
    ]);
    assert!(matches!(
        host.relay(&handle, &gate, &txt("approve-plan art:2"), &fence()),
        Ok(Relayed::Stale(Progress::Gate(_)))
    ));
    let call = cmd.calls.borrow()[0].clone();
    assert!(call.windows(2).any(|a| a == ["--fence", "12"]));
    cmd.replies.borrow_mut().push_back(json(&Response::Ok));
    assert_eq!(
        host.relay(&handle, &gate, &txt("approve-plan art:2"), &fence()),
        Ok(Relayed::Applied)
    );
    assert_eq!(
        cmd.calls.borrow()[2],
        call,
        "retry reuses the exact operation"
    );
    let n = cmd.calls.borrow().len();
    for invalid in ["approve-plan art:99", "cancel --dir /other"] {
        cmd.replies
            .borrow_mut()
            .push_back(json(&Some(view(&c, State::PlanGate))));
        assert!(
            matches!(host.relay(&handle, &gate, &txt(invalid), &fence()), Ok(Relayed::Rejected(Progress::Gate(g))) if g.prompt.as_str().contains("Answer rejected"))
        );
    }
    assert!(
        cmd.calls.borrow()[n..].iter().all(|argv| argv[1] == "show"),
        "bad input never reaches the human command"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn approved_is_a_human_receipt_gate_and_closed_requires_recorded_receipt() {
    let dir = temp();
    let c = config(&dir);
    assert!(
        matches!(BbpHost::<Contract, FakeSupervisor>::progress(view(&c, State::Approved)), Ok(Progress::Gate(g)) if g.prompt.as_str().contains("receipt"))
    );
    let mut v = view(&c, State::Closed);
    assert!(BbpHost::<Contract, FakeSupervisor>::progress(v.clone()).is_err());
    v.receipt = Some((ArtId(4), "merge-revision".into()));
    assert!(
        matches!(BbpHost::<Contract, FakeSupervisor>::progress(v), Ok(Progress::Closed { outputs }) if outputs.len() == 2)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn task_secrets_cannot_be_stored_in_the_source_checkout() {
    let dir = temp();
    let mut c = config(&dir);
    c.dir = c.repo.join(".bbp");
    assert!(c.absolute().unwrap_err().contains("outside"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hosted_brief_reuses_the_existing_renderer_to_resolve_board_context() {
    let dir = temp();
    let c = config(&dir);
    let (task, mut board) = task();
    let entry = board
        .append(NewEntry {
            task: None,
            author: Author::Human,
            kind: EntryKind::Assumption,
            body: txt("required research context"),
            refs: vec![],
            supersedes: None,
        })
        .unwrap();
    let mut plan = Plan::new(board.goal());
    let mut spec = task.spec().clone();
    spec.refs.push(Ref::Entry(entry));
    let id = plan.add(spec).unwrap();
    let host = BbpHost::with_seams(
        c.clone(),
        "context".into(),
        "Goal context".into(),
        Contract::default(),
        FakeSupervisor::default(),
    )
    .unwrap();
    let frozen = host
        .freeze(
            &Handle {
                dir: c.dir,
                task: "context".into(),
            },
            plan.get(id).unwrap(),
            &board,
        )
        .unwrap();
    assert!(frozen.brief.contains("required research context"));
    assert!(frozen.brief.contains("Goal context"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_fence_admits_later_generations_and_refuses_superseded_holders() {
    let dir = temp();
    let h = Handle {
        dir: dir.clone(),
        task: "orch-fenced".into(),
    };
    let admit = |fence: Fence| <BbpHost>::admit(&h, &fence);
    let unavailable =
        |r: Result<(), ClassifiedError>| matches!(r, Err(ClassifiedError::Unavailable(_)));
    admit(fence_at(1, "first")).unwrap();
    admit(fence_at(1, "first")).unwrap();
    assert!(unavailable(admit(fence_at(1, "diverged"))));
    admit(fence_at(3, "second")).unwrap();
    assert!(unavailable(admit(fence_at(1, "first"))));
    assert!(unavailable(admit(fence_at(2, "late"))));
    let mut foreign = fence_at(4, "other");
    foreign.assignment = txt("maintenance");
    assert!(matches!(admit(foreign), Err(ClassifiedError::Permanent(_))));
    admit(fence_at(3, "second")).unwrap();
    // A later generation of the owner still takes over.
    admit(fence_at(4, "third")).unwrap();
    assert!(unavailable(admit(fence_at(3, "second"))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cancel_retries_a_moved_revision_waits_until_quiesced_and_skips_an_ended_task() {
    let dir = temp();
    let c = config(&dir);
    let (task, board) = task();
    let command = Contract::default();
    command.replies.borrow_mut().extend([
        "null".into(),
        json(&Response::Ok),
        json(&Some(view(&c, State::Planning))),
        json(&Response::Ok),
        json(&Response::Ok),
    ]);
    let sup = FakeSupervisor::default();
    let mut host = BbpHost::with_seams(
        c.clone(),
        "goal".into(),
        "contract".into(),
        command.clone(),
        sup,
    )
    .unwrap();
    host.open(&task, &board, &fence()).unwrap();
    command.calls.borrow_mut().clear();
    let mut ended = view(&c, State::Cancelled);
    ended.cancelled = Some("owner withdrew".into());
    // The board never got the handle: the host finds the task anyway.
    command.replies.borrow_mut().extend([
        json(&Some(view(&c, State::Planning))),
        json(&Some(view(&c, State::Planning))),
        json(&Response::Rejected(Rejection::new(Code::StaleRev, "moved"))),
        json(&Some(view(&c, State::Planning))),
        json(&Response::Ok),
        "false".into(),
        "true".into(),
        json(&Some(ended.clone())),
    ]);
    let reason = txt("owner withdrew");
    let progress = host.cancel(&task, &board, &reason, &fence()).unwrap();
    assert_eq!(
        progress,
        Progress::Cancelled {
            reason: reason.clone()
        }
    );
    let calls = command.calls.borrow().clone();
    assert_eq!(
        calls.iter().map(|a| a[1].as_str()).collect::<Vec<_>>(),
        ["show", "show", "human", "show", "human", "quiesce", "quiesce", "show"]
    );
    assert!(calls[2].windows(2).any(|a| a == ["--rev", "12"]));
    assert!(calls[4].ends_with(&["cancel".into(), "owner".into(), "withdrew".into()]));

    // An ended task is reported as it stands; nothing is sent.
    command.calls.borrow_mut().clear();
    command.replies.borrow_mut().extend([
        json(&Some(ended.clone())),
        json(&Some(ended.clone())),
        "true".into(),
        json(&Some(ended)),
    ]);
    assert_eq!(
        host.cancel(&task, &board, &reason, &fence()).unwrap(),
        Progress::Cancelled { reason }
    );
    assert!(command.calls.borrow().iter().all(|a| a[1] != "human"));
    // A reason cannot smuggle a flag into BBP's parser.
    command.calls.borrow_mut().clear();
    assert!(host
        .cancel(&task, &board, &txt("x --rev 1"), &fence())
        .is_err());
    assert!(command.calls.borrow().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cancel_opens_a_never_opened_task_straight_into_cancelled_and_starts_nothing() {
    let dir = temp();
    let c = config(&dir);
    let (task, board) = task();
    let command = Contract::default();
    let mut ended = view(&c, State::Cancelled);
    ended.cancelled = Some("withdrawn".into());
    command.replies.borrow_mut().extend([
        "null".into(),
        json(&Response::Ok),
        json(&Some(view(&c, State::Planning))),
        json(&Response::Ok),
        "true".into(),
        json(&Some(ended)),
    ]);
    let sup = FakeSupervisor::default();
    let mut host = BbpHost::with_seams(
        c,
        "goal".into(),
        "contract".into(),
        command.clone(),
        sup.clone(),
    )
    .unwrap();
    let progress = host
        .cancel(&task, &board, &txt("withdrawn"), &fence())
        .unwrap();
    assert!(matches!(progress, Progress::Cancelled { .. }));
    assert_eq!(
        command
            .calls
            .borrow()
            .iter()
            .map(|a| a[1].as_str())
            .collect::<Vec<_>>(),
        ["show", "open", "show", "human", "quiesce", "show"],
        "no principal is assigned"
    );
    assert_eq!(sup.0.get(), 0, "no moderator is started");
    std::fs::remove_dir_all(dir).unwrap();
}
