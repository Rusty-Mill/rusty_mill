#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `bbp verify` (#722): reports the chain head, accepts an earlier head after
//! appends, refuses an absent one, and never repairs a partial line.

use rusty_bbp::*;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/why")
        .join(name)
}

/// A data directory holding a legacy fixture log for `task`.
fn store_with(tag: &str, task: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("bbp_verify_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = log_path_for(&dir, &TaskId(task.into()));
    std::fs::create_dir_all(path.parent().expect("tasks dir")).expect("mkdir");
    std::fs::copy(fixture("run_a_event_driven.log"), &path).expect("copy log");
    (dir, path)
}

fn bbp(args: &[&str]) -> (bool, String, String) {
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(args)
        .output()
        .expect("run bbp");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn head_of(json: &str) -> String {
    let at = json.find("\"head\":\"").expect("head") + 8;
    json[at..at + 64].to_owned()
}

/// Append one chained batch through the store, as a writer would.
fn append_one(dir: &Path, task: &str) {
    let task = TaskId(task.into());
    let mut store = FsStore::open(dir).expect("open");
    let rev = store.rev(&task);
    let ev = Event::Assigned {
        role: RoleId("planner".into()),
        principal: Principal {
            id: PrincipalId("p-extra".into()),
            kind: PrincipalKind::Human,
        },
    };
    store.append(&task, rev, &[ev]).expect("append");
}

#[test]
fn an_earlier_head_still_verifies_after_appends() {
    let (dir, _) = store_with("grow", "slug-ed-1");
    let d = dir.to_str().unwrap();
    let (ok, legacy, err) = bbp(&["verify", "--dir", d, "--task", "slug-ed-1"]);
    assert!(ok, "{err}");
    assert!(legacy.contains("\"partial\":false"), "{legacy}");
    let lines = legacy
        .split("\"lines\":")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .to_owned();
    assert!(
        legacy.contains(&format!("\"unchained\":{lines}")),
        "fixture predates the chain: {legacy}"
    );
    let old = head_of(&legacy);

    append_one(&dir, "slug-ed-1");
    let (ok, now, err) = bbp(&[
        "verify",
        "--dir",
        d,
        "--task",
        "slug-ed-1",
        "--expect",
        &old,
    ]);
    assert!(ok, "{err}");
    assert_ne!(head_of(&now), old, "the head moved");
    assert!(now.contains("\"expect_found\":true"), "{now}");

    let absent = "ab".repeat(32);
    let (ok, out, err) = bbp(&[
        "verify",
        "--dir",
        d,
        "--task",
        "slug-ed-1",
        "--expect",
        &absent,
    ]);
    assert!(!ok, "{out}");
    assert!(out.contains("\"expect_found\":false"), "{out}");
    assert!(err.contains("--expect"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_line_is_reported_and_left_in_place() {
    let (dir, log) = store_with("partial", "slug-ed-1");
    let mut bytes = std::fs::read(&log).expect("read");
    bytes.extend_from_slice(b"[{\"Assig");
    std::fs::write(&log, &bytes).expect("write");
    let (ok, out, err) = bbp(&[
        "verify",
        "--dir",
        dir.to_str().unwrap(),
        "--task",
        "slug-ed-1",
    ]);
    assert!(!ok, "{out}");
    assert!(out.contains("\"partial\":true"), "{out}");
    assert!(err.contains("partial"), "{err}");
    assert_eq!(
        std::fs::read(&log).expect("read"),
        bytes,
        "verify never repairs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_broken_chain_fails_with_its_line() {
    let (dir, log) = store_with("broken", "slug-ed-1");
    append_one(&dir, "slug-ed-1");
    append_one(&dir, "slug-ed-1");
    let text = std::fs::read_to_string(&log).expect("read");
    std::fs::write(&log, text.replacen("p-extra", "p-forged", 1)).expect("write");
    let (ok, out, err) = bbp(&[
        "verify",
        "--dir",
        dir.to_str().unwrap(),
        "--task",
        "slug-ed-1",
    ]);
    assert!(!ok, "{out}");
    assert!(err.contains("chain broken"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
