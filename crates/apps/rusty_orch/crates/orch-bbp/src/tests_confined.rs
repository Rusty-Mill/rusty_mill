//! Required Linux evidence through the real Orch CLI and production supervisor.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use rusty_bbp::{Event, FsStore, Store};
use rusty_serde::json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

fn invoke(dir: &Path, answer: &str) -> Value {
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN to built rusty_orch");
    let mut child = Command::new(orch)
        .arg("run")
        .arg(dir.join("goal.json"))
        .arg("--bbp")
        .arg(dir.join("bbp.json"))
        .arg("--state")
        .arg(dir.join("orch-state"))
        .args(["--json", "--interactive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{answer}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "Orch failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value =
        rusty_serde::json::from_str(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
    assert_eq!(report.get("calls").and_then(Value::as_u64), Some(1));
    report
}

fn handle(report: &Value) -> Handle {
    let entry = report
        .get("entries")
        .unwrap()
        .as_seq()
        .unwrap()
        .iter()
        .find(|e| e.get("body").and_then(Value::as_str) == Some("hosted task"))
        .unwrap();
    let pointer = entry.get("refs").unwrap().as_seq().unwrap()[0]
        .as_str()
        .unwrap()
        .strip_prefix("url:bbp:")
        .unwrap();
    rusty_serde::json::from_str(pointer).unwrap()
}

fn events(h: &Handle) -> Vec<Event> {
    FsStore::open(&h.dir)
        .unwrap()
        .events(&rusty_bbp::TaskId(h.task.clone()), 0)
        .unwrap()
}

fn wait_state(dir: &Path, h: &Handle, expected: State) -> Value {
    wait_state_for(dir, h, expected, Duration::from_secs(30))
}

fn wait_state_for(dir: &Path, h: &Handle, expected: State, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let report = invoke(dir, "");
        let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(h));
        let kind = report
            .get("ended")
            .unwrap()
            .get("kind")
            .and_then(Value::as_str)
            .unwrap();
        assert!(
            matches!(kind, "waiting" | "blocked" | "finished"),
            "{report:?}"
        );
        assert!(
            !matches!(view.card.state, State::Escalated | State::Cancelled),
            "{view:?}"
        );
        if view.card.state == expected && matches!(kind, "blocked" | "finished") {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {expected:?}: {view:?}; {report:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "live subscription providers: BBP_LIVE_SMOKE=1, BBP_CODEX_MODEL, BBP_CLAUDE_MODEL, BBP_TEST_BIN, ORCH_TEST_BIN; Linux sandbox required"]
fn opt_in_codex_author_claude_reviewer_smoke() {
    assert_eq!(
        std::env::var("BBP_LIVE_SMOKE").as_deref(),
        Ok("1"),
        "explicit BBP_LIVE_SMOKE=1 required; this test makes model calls"
    );
    let codex_model = std::env::var("BBP_CODEX_MODEL").expect("pin BBP_CODEX_MODEL");
    let claude_model = std::env::var("BBP_CLAUDE_MODEL").expect("pin BBP_CLAUDE_MODEL");
    let (dir, mut c) = super::tests_e2e::setup();
    let bin = c.binary.parent().unwrap();
    for (member, vendor, executable, model) in [
        ("writer", "codex", "codex-bbp-harness", codex_model),
        ("judge", "claude", "claude-bbp-harness", claude_model),
    ] {
        let agent = c.pool.get_mut(member).unwrap();
        agent.vendor = vendor.into();
        agent.program = bin.join(executable).to_str().unwrap().into();
        agent.args = vec![
            "--mcp-config".into(),
            "{mcp_config}".into(),
            "--model".into(),
            model,
        ];
    }
    c.assignments.get_mut("maker").unwrap().builtin_tools = true;
    c.max_wall_secs = 600;
    std::fs::write(
        dir.join("bbp.json"),
        rusty_serde::json::to_string(&c).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("goal.json"), r#"{"goal":"Change tracked file flag from no to ok","done_when":["flag contains exactly ok followed by a newline; confined validation passes; independent review approves"],"out_of_scope":["all other source changes","pushes and real merges"],"wall_clock_secs":600,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Replace no with ok in the tracked file flag, preserving its newline. Change no other file. Submit a nonempty candidate diff through BBP and judge the selected test report.","acceptance":["flag contains ok followed by a newline","no other source file changed"],"max_calls":1}]}"#).unwrap();
    let h = handle(&invoke(&dir, ""));
    let timeout = Duration::from_secs(480);
    wait_state_for(&dir, &h, State::PlanGate, timeout);
    let view = || TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h));
    invoke(
        &dir,
        &format!("approve-plan art:{}", view().proposal.unwrap().0),
    );
    wait_state_for(&dir, &h, State::MergeGate, timeout);
    let card = view().card;
    let candidate = card.candidate.unwrap();
    let run = card.run.unwrap().0;
    assert!(events(&h).iter().any(|e| matches!(e, Event::MessageAppended(m) if m.from.0 == "orch-auditor" && matches!(&m.draft.body, rusty_bbp::Body::Verdict(v) if v.subject == candidate && v.run == run && v.verdict == rusty_bbp::VerdictKind::Approve))));
    let store = FsStore::open(&h.dir).unwrap();
    let log = events(&h);
    let report_id = log
        .iter()
        .find_map(|e| match e {
            Event::TestReportStored {
                run: r,
                status: rusty_bbp::RunStatus::Passed,
                report,
                ..
            } if *r == run => Some(*report),
            _ => None,
        })
        .expect("selected run passed");
    let artifact = log
        .iter()
        .find_map(|e| match e {
            Event::ArtifactStored(a) if a.id == report_id => Some(a),
            _ => None,
        })
        .unwrap();
    let rusty_bbp::ArtifactPayload::TestReport(report) = rusty_bbp::decode_artifact(
        rusty_bbp::ArtifactKind::TestReport,
        &store.blob_get(&artifact.blob.sha).unwrap(),
    )
    .unwrap() else {
        panic!("test report expected")
    };
    assert_eq!(report.sandbox, "landlock+seccomp:no-endpoints");
    invoke(
        &dir,
        &format!("approve-merge art:{} run:{}", candidate.0, run.0),
    );
    wait_state_for(&dir, &h, State::Approved, timeout);
    assert!(view().receipt.is_none());
    // Test-only receipt: no Git merge/push command is run by the fixture.
    invoke(
        &dir,
        &format!("receipt art:{} smoke-receipt-no-real-merge", candidate.0),
    );
    wait_state_for(&dir, &h, State::Closed, timeout);
    assert_eq!(
        view().receipt,
        Some((candidate, "smoke-receipt-no-real-merge".into()))
    );
    println!("BBP_LIVE_SMOKE PASS codex-author/claude-reviewer");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires BBP_TEST_BIN, ORCH_TEST_BIN and enforced Linux sandbox"]
fn confined_orch_cli_lifecycle_resumes_and_records_receipt() {
    let (dir, c) = super::tests_e2e::scripted_config();
    std::fs::write(
        dir.join("bbp.json"),
        rusty_serde::json::to_string(&c).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("goal.json"), r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let h = handle(&invoke(&dir, ""));
    wait_state(&dir, &h, State::PlanGate);
    // Abruptly lose the moderator while both Orch and BBP have a durable gate.
    let key = Sha256::of(h.task.as_bytes()).hex();
    let pid_path = c.dir.join("mod").join(format!("{key}.pid"));
    let first = std::fs::read_to_string(&pid_path).unwrap();
    assert!(Command::new("kill")
        .args(["-KILL", first.trim()])
        .status()
        .unwrap()
        .success());
    let lock_path = c.dir.join("mod").join(format!("{key}.lock"));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
            .unwrap();
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(e) => panic!("moderator lock: {e}"),
        }
        assert!(Instant::now() < deadline, "dead moderator retained lock");
        std::thread::sleep(Duration::from_millis(10));
    }
    let resumed = wait_state(&dir, &h, State::PlanGate);
    assert_eq!(handle(&resumed).task, h.task);
    assert_ne!(std::fs::read_to_string(&pid_path).unwrap(), first);
    invoke(&dir, "approve-plan art:2");
    wait_state(&dir, &h, State::MergeGate);
    invoke(&dir, "approve-merge art:4 run:1");
    wait_state(&dir, &h, State::Approved);
    assert!(!events(&h)
        .iter()
        .any(|e| matches!(e, Event::MergeReceipt { .. })));
    invoke(&dir, "receipt art:4 external-merge-sha");
    let final_report = wait_state(&dir, &h, State::Closed);
    assert_eq!(
        final_report
            .get("ended")
            .unwrap()
            .get("kind")
            .and_then(Value::as_str),
        Some("finished")
    );
    let log = events(&h);
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Event::TaskOpened { .. }))
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Event::HumanApproval { .. }))
            .count(),
        2
    );
    assert!(log.iter().any(|e| matches!(e, Event::MessageAppended(m) if m.from.0 == "orch-auditor" && matches!(m.draft.body, rusty_bbp::Body::Verdict(_)))));
    // Check the actual report, not just the state machine reaching a gate.
    let store = FsStore::open(&h.dir).unwrap();
    for event in &log {
        if let Event::TestReportStored { status, report, .. } = event {
            assert_eq!(*status, rusty_bbp::RunStatus::Passed);
            let artifact = log
                .iter()
                .find_map(|e| match e {
                    Event::ArtifactStored(a) if a.id == *report => Some(a),
                    _ => None,
                })
                .unwrap();
            let bytes = store.blob_get(&artifact.blob.sha).unwrap();
            let rusty_bbp::ArtifactPayload::TestReport(report) =
                rusty_bbp::decode_artifact(rusty_bbp::ArtifactKind::TestReport, &bytes).unwrap()
            else {
                panic!("expected test report");
            };
            assert_eq!(report.sandbox, "landlock+seccomp:no-endpoints");
        }
    }
    assert!(log
        .iter()
        .any(|e| matches!(e, Event::TestReportStored { .. })));
    assert_eq!(
        TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &log).receipt,
        Some((rusty_bbp::ArtId(4), "external-merge-sha".into()))
    );
    println!("BBP_ACCEPTANCE PASS lifecycle");
    std::fs::remove_dir_all(dir).unwrap();
}
