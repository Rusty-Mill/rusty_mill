#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The host keeps shadow records: every dispatch through `open_driver` appends
//! the grants' records to the task's shadow log, once per `rev`, and `bbp shadow`
//! tallies them.

mod common;
use common::Who;
use rusty_bbp::*;
use rusty_bbp_host::{admin, open_driver, shadow};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_shadow_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A task with the four roles assigned and the first turn granted.
fn setup(dir: &Path) -> TaskId {
    let task = TaskId("T1".into());
    admin::open_task(
        dir,
        &task,
        "github.com/example/repo",
        b"Add retry with backoff.",
        &PrincipalId("human".into()),
        &rusty_bbp_host::profiles::ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("open");
    for w in Who::ALL {
        admin::assign(
            dir,
            &task,
            &w.id(),
            &PrincipalId(w.name().into()),
            "anthropic",
        )
        .expect("assign");
    }
    admin::tick(dir, &task).expect("tick");
    task
}

#[test]
fn the_first_grant_is_logged_once_and_tallied() {
    let dir = tempdir("first");
    let task = setup(&dir);
    let logged = shadow::read(&dir, &task).expect("read");
    assert_eq!(logged.unreadable, 0);
    assert_eq!(logged.records.len(), 1, "{:?}", logged.records);
    assert_eq!(logged.records[0].state, State::Planning);
    assert_eq!(logged.records[0].comparison(), Some(Comparison::Agree));

    // Dispatches that grant nothing add nothing, however often they run.
    admin::tick(&dir, &task).expect("tick");
    admin::tick(&dir, &task).expect("tick");
    assert_eq!(shadow::read(&dir, &task).expect("read").records.len(), 1);

    let report = shadow::of(&dir, &task).expect("report");
    assert_eq!((report.records, report.total.agree), (1, 1));
    assert_eq!(report.by_state["Planning"].agree, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn appending_a_logged_rev_again_writes_nothing() {
    let dir = tempdir("dedupe");
    let task = setup(&dir);
    let have = shadow::read(&dir, &task).expect("read").records;
    assert_eq!(shadow::append(&dir, &task, &have).expect("append"), 0);
    let mut later = have[0].clone();
    later.rev = Rev(later.rev.0 + 100);
    assert_eq!(
        shadow::append(&dir, &task, &[later.clone(), later]).expect("append"),
        1
    );
    assert_eq!(shadow::read(&dir, &task).expect("read").records.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_torn_line_is_counted_and_does_not_block_logging() {
    let dir = tempdir("torn");
    let task = setup(&dir);
    let path = shadow::log_path(&dir, &task);
    let mut text = std::fs::read_to_string(&path).expect("log");
    text.push_str("{\"rev\":9,\"sta");
    std::fs::write(&path, text).expect("tear");
    let logged = shadow::read(&dir, &task).expect("read");
    assert_eq!((logged.records.len(), logged.unreadable), (1, 1));
    assert_eq!(shadow::of(&dir, &task).expect("report").unreadable_lines, 1);
    let mut next = logged.records[0].clone();
    next.rev = Rev(50);
    assert_eq!(shadow::append(&dir, &task, &[next]).expect("append"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_with_no_log_reports_nothing() {
    let dir = tempdir("none");
    let report = shadow::of(&dir, &TaskId("T9".into())).expect("report");
    assert_eq!(report.records, 0);
    assert!(report.by_state.is_empty());
}

#[test]
fn bbp_shadow_reports_the_log_the_drivers_wrote() {
    let dir = tempdir("process");
    let task = setup(&dir);
    // End the planner's turn by a pass through a second driver on the same
    // directory, then let the `bbp` binary, a separate process, report the log.
    let mut d = open_driver(&dir, &task).expect("driver");
    let turn = d.state.turns[0].clone();
    let token = d.state.token_for(&PrincipalId("planner".into()), turn.id);
    d.dispatch(
        &Command::Agent {
            principal: PrincipalId("planner".into()),
            token: Some(token),
            op: OpId("p1".into()),
            action: AgentAction::Post(Draft::new(MessageKind::Pass, "nothing to add")),
        },
        Time(rusty_bbp_host::now().0),
    )
    .expect("pass");
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(["shadow", "--dir"])
        .arg(&dir)
        .args(["--task", "T1"])
        .output()
        .expect("run bbp shadow");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf8");
    assert!(text.contains("\"task\":\"T1\""), "{text}");
    let logged = shadow::read(&dir, &task).expect("read");
    assert!(
        logged.records.len() >= 2,
        "the regrant after the pass is logged: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
