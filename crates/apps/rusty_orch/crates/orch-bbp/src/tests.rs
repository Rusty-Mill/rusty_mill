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
                },
            ),
            (
                "judge".into(),
                Agent {
                    vendor: "future-provider".into(),
                    program: "future-cli".into(),
                    args: vec!["{mcp_config}".into()],
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
    let handle = host.open(&task, &board).unwrap();
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
    assert_eq!(resumed.open(&task, &board).unwrap(), handle);
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
        host.relay(&handle, &gate, &txt("approve-plan art:2")),
        Ok(Relayed::Stale(Progress::Gate(_)))
    ));
    let call = cmd.calls.borrow()[0].clone();
    assert!(call.windows(2).any(|a| a == ["--fence", "12"]));
    cmd.replies.borrow_mut().push_back(json(&Response::Ok));
    assert_eq!(
        host.relay(&handle, &gate, &txt("approve-plan art:2")),
        Ok(Relayed::Applied)
    );
    assert_eq!(
        cmd.calls.borrow()[2],
        call,
        "retry reuses the exact operation"
    );
    let n = cmd.calls.borrow().len();
    assert!(host
        .relay(&handle, &gate, &txt("approve-plan art:99"))
        .is_err());
    assert!(host
        .relay(&handle, &gate, &txt("cancel --dir /other"))
        .is_err());
    assert_eq!(cmd.calls.borrow().len(), n);
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
