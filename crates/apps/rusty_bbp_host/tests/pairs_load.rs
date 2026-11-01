#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `bbp pairs` over real run directories: the loader reads each run's own
//! `bbp mod` output and refuses a run whose `bbp` was not built from the
//! pinned checkout.

mod common;

use rusty_bbp::*;
use rusty_bbp_host::moderator;
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::{admin, pairs};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_pairs_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

/// A clean checkout with one commit; returns the repo and its HEAD.
fn checkout(root: &Path) -> (PathBuf, String) {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a"), "1").unwrap();
    git(&repo, &["add", "a"]);
    git(&repo, &["commit", "-q", "-m", "one"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    (repo, head)
}

const ASSISTANT: &str = r#"{"type":"assistant","message":{"id":"a","model":"m","usage":{"input_tokens":1,"output_tokens":1}}}"#;
const RESULT_ONLY: &str = r#"{"type":"result","usage":{"input_tokens":5,"output_tokens":5}}"#;

/// An opened task whose planner was granted turn 1, which printed `turn_log`
/// (none: no harness output), and the saved `bbp mod` output of its run.
fn run(root: &Path, name: &str, mod_output: &str, turn_log: Option<&str>) -> serde_json_like::Run {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let task = TaskId("T1".into());
    admin::open_task(
        &dir,
        &task,
        "local",
        b"brief",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("open");
    admin::assign(
        &dir,
        &task,
        &RoleId::from("planner"),
        &PrincipalId("planner".into()),
        "v",
    )
    .expect("assign");
    admin::tick(&dir, &task).expect("tick");
    if let Some(text) = turn_log {
        let log = moderator::log_path(&dir, &task, TurnId(1));
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(log, text).unwrap();
    }
    // As `bbp mod --max-usd 5 --criteria ..` leaves it.
    moderator::write_evidence_marker(&dir, &task, CAP_USD, &criteria_sha(root)).unwrap();
    // The one segment that `bbp mod` records, by the build its output reports.
    let build = pairs::build_sha_of(mod_output).unwrap_or_default();
    moderator::record_evidence_segment(&dir, &task, &build).unwrap();
    let out = root.join(format!("{name}.out.json"));
    std::fs::write(&out, mod_output).unwrap();
    serde_json_like::Run {
        dir: dir.display().to_string(),
        out: out.display().to_string(),
    }
}

mod serde_json_like {
    pub struct Run {
        pub dir: String,
        pub out: String,
    }
    impl Run {
        pub fn json(&self) -> String {
            format!(
                r#"{{"dir":{:?},"task":"T1","mod_output":{:?}}}"#,
                self.dir, self.out
            )
        }
    }
}

const CAP_USD: f64 = 5.0;

/// The sha of the criteria file `load_with_logs` writes under `root`.
fn criteria_sha(root: &Path) -> String {
    rusty_bbp_host::tokens::read_criteria(&root.join("criteria.json"))
        .unwrap()
        .1
}

fn mod_out(sha: Option<&str>) -> String {
    match sha {
        Some(s) => format!(
            r#"{{"state":"Closed","turns_launched":0,"runs":0,"elapsed_ms":1,"state_ms":[["Planning",10]],"bbp_build_sha":"{s}"}}"#
        ),
        None => r#"{"state":"Closed","turns_launched":0,"runs":0,"elapsed_ms":1,"state_ms":[["Planning",10]]}"#.to_owned(),
    }
}

/// Load a one-pair manifest whose control and treatment runs reported these
/// build identities, as an evaluator built from the checkout's HEAD.
fn load(tag: &str, control: Option<&str>, treatment: Option<&str>) -> Result<(), String> {
    load_with_logs(tag, control, treatment, Some(ASSISTANT), Some(ASSISTANT))
}

fn load_with_logs(
    tag: &str,
    control: Option<&str>,
    treatment: Option<&str>,
    control_log: Option<&str>,
    treatment_log: Option<&str>,
) -> Result<(), String> {
    let root = tempdir(tag);
    let (repo, head) = checkout(&root);
    let criteria = root.join("criteria.json");
    std::fs::write(
        &criteria,
        format!(
            r#"{{"prices_as_of":"test","models":{{"m":{{"input":1,"output":1,"cache_creation_5m":1,"cache_creation_1h":1,"cache_read":1}}}},"min_commit":"{head}"}}"#
        ),
    )
    .unwrap();
    let sha = |s: Option<&str>| s.map(|s| s.replace("HEAD", &head));
    let c = run(
        &root,
        "control",
        &mod_out(sha(control).as_deref()),
        control_log,
    );
    let t = run(
        &root,
        "treatment",
        &mod_out(sha(treatment).as_deref()),
        treatment_log,
    );
    let manifest = root.join("manifest.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{"poll_ms":500,"reads_per_turn":30,"cap_usd":5.0,"criteria":{:?},"repo":{:?},"pairs":[{{"id":"p1","kind":"separable","control":{},"treatment":{}}}]}}"#,
            criteria.display().to_string(),
            repo.display().to_string(),
            c.json(),
            t.json()
        ),
    )
    .unwrap();
    let r = pairs::load_as(&manifest, &head).map(|_| ());
    let _ = std::fs::remove_dir_all(&root);
    r
}

const OTHER: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn runs_built_from_the_pinned_checkout_pass_the_pin() {
    // Everything after the pin is out of scope here; the pin must not be what fails.
    let err = load("ok", Some("HEAD"), Some("HEAD"))
        .err()
        .unwrap_or_default();
    assert!(
        !err.contains("bbp_build_sha") && !err.contains("built from"),
        "{err}"
    );
}

#[test]
fn a_run_that_reported_no_build_identity_is_refused() {
    let err = load("missing", None, Some("HEAD")).unwrap_err();
    assert!(
        err.contains("p1/control") && err.contains("bbp_build_sha"),
        "{err}"
    );
}

#[test]
fn a_dirty_or_unknown_build_is_refused() {
    let err = load("dirty", Some("HEAD-dirty"), Some("HEAD")).unwrap_err();
    assert!(err.contains("p1/control") && err.contains("dirty"), "{err}");
    let err = load("unknown", Some("HEAD"), Some("unknown")).unwrap_err();
    assert!(
        err.contains("p1/treatment") && err.contains("unknown"),
        "{err}"
    );
}

#[test]
fn a_build_from_another_commit_is_refused_alone_or_mixed_with_a_good_one() {
    let err = load("other", Some(OTHER), Some(OTHER)).unwrap_err();
    assert!(err.contains(OTHER), "{err}");
    let err = load("mixed", Some("HEAD"), Some(OTHER)).unwrap_err();
    assert!(err.contains("p1/treatment") && err.contains(OTHER), "{err}");
}

#[test]
fn a_result_only_turn_log_names_no_model_and_the_evaluation_refuses() {
    let err = load_with_logs(
        "result_only",
        Some("HEAD"),
        Some("HEAD"),
        Some(RESULT_ONLY),
        Some(ASSISTANT),
    )
    .unwrap_err();
    assert!(
        err.contains("p1/control") && err.contains("no model"),
        "{err}"
    );
}

#[test]
fn a_granted_turn_with_no_harness_output_names_no_model_and_the_evaluation_refuses() {
    let err = load_with_logs(
        "no_output",
        Some("HEAD"),
        Some("HEAD"),
        Some(ASSISTANT),
        None,
    )
    .unwrap_err();
    assert!(
        err.contains("p1/treatment") && err.contains("no model"),
        "{err}"
    );
}

// ---- second review, finding 1: a run the moderator stopped is a failed run

fn stopped_facts(tag: &str, stop: &str) -> pairs::RunFacts {
    facts_with_log(tag, stop, Some(ASSISTANT))
}

fn facts_with_log(tag: &str, stop: &str, log: Option<&str>) -> pairs::RunFacts {
    facts_after(tag, stop, log, |_, _, _| {}).expect("a stopped run loads")
}

/// [`facts_with_log`], with `edit` applied to the run directory (`dir`, task,
/// the checkout's HEAD) after the run and its one segment were recorded.
fn facts_after(
    tag: &str,
    stop: &str,
    log: Option<&str>,
    edit: impl FnOnce(&Path, &TaskId, &str),
) -> Result<pairs::RunFacts, String> {
    let root = tempdir(tag);
    let (_repo, head) = checkout(&root);
    let criteria = root.join("criteria.json");
    std::fs::write(
        &criteria,
        r#"{"prices_as_of":"test","models":{"m":{"input":1,"output":1,"cache_creation_5m":1,"cache_creation_1h":1,"cache_read":1}}}"#,
    )
    .unwrap();
    // An empty `stop` is an output that shows none.
    let stop = if stop.is_empty() {
        String::new()
    } else {
        format!(r#","stop":"{stop}","stop_reason":"stopped""#)
    };
    let out = format!(
        r#"{{"state":"Planning","turns_launched":1,"runs":0,"elapsed_ms":1,"state_ms":[["Planning",10]],"replay_equals_live":true{stop},"bbp_build_sha":"{head}"}}"#
    );
    let r = run(&root, "stopped", &out, log);
    edit(Path::new(&r.dir), &TaskId("T1".into()), &head);
    let (c, sha) = rusty_bbp_host::tokens::read_criteria(&criteria).unwrap();
    let f = pairs::facts(
        &rusty_bbp_host::pairs::RunRef {
            dir: r.dir.clone(),
            task: "T1".into(),
            mod_output: r.out.clone(),
            rubric_ok: true,
        },
        &c,
        &sha,
        CAP_USD,
    );
    let _ = std::fs::remove_dir_all(&root);
    f
}

#[test]
fn a_run_stopped_by_the_cap_loads_as_a_failed_run_over_the_cap() {
    let f = stopped_facts("stopped-cap", "cap");
    assert!(f.over_cap);
    assert!(!f.succeeded());
}

#[test]
fn a_run_stopped_by_the_wall_limit_loads_as_a_failed_run() {
    let f = stopped_facts("stopped-wall", "wall");
    assert!(!f.over_cap);
    assert!(!f.succeeded());
    assert_eq!(f.replay_equals_live, Some(true));
}

// ---- round two: a turn that never printed a result

#[test]
fn a_run_with_a_turn_that_never_printed_a_result_fails_and_keeps_its_known_cost() {
    // The assistant line is $0.000002 at the test prices; the harness was
    // killed before its result.
    let f = facts_with_log("killed", "wall", Some(ASSISTANT));
    assert_eq!(f.killed_turns, 1);
    assert!(
        f.usd_floor.is_some_and(|u| (u - 0.000002).abs() < 1e-12),
        "{:?}",
        f.usd_floor
    );
    assert!(!f.succeeded());
}

// ---- third review, finding 2: a log the cap could not read still loads, as a failed run

#[test]
fn a_run_stopped_over_a_log_the_tool_cannot_read_loads_as_a_failed_run() {
    // A usage line that is valid JSON but not a shape the tool reads: the cap
    // stopped the run, and `bbp pairs` must not drop the whole manifest.
    let bad = r#"{"type":"assistant","message":{"id":"a","model":"m","usage":{"input_tokens":1,"output_tokens":1,"cache_creation":null}}}"#;
    let f = facts_with_log("unreadable-stop", "cap", Some(bad));
    assert!(f.over_cap);
    assert!(f.stopped);
    assert!(!f.succeeded());
}

// ---- #1326: a run is judged by every `bbp mod` segment that produced it

#[test]
fn a_run_with_one_segment_by_the_build_its_output_reports_loads() {
    let f = facts_after("one-segment", "wall", Some(ASSISTANT), |_, _, _| {});
    assert!(f.is_ok(), "{f:?}");
}

#[test]
fn a_resumed_run_loads_as_a_failed_stopped_run_marked_resumed() {
    // The saved output of the last segment shows no stop; the record shows two.
    let f = facts_after("two-segments", "", Some(ASSISTANT), |dir, task, head| {
        moderator::record_evidence_segment(dir, task, head).unwrap();
    })
    .expect("a resumed run loads, as a failed run");
    assert!(f.stopped && !f.succeeded(), "{f:?}");
    assert_eq!(f.stop.as_deref(), Some("resumed: 2 bbp mod segments"));
    assert_eq!(
        f.spec_wall_ms, None,
        "the last segment's timing is not the run's"
    );
    assert_eq!(
        f.replay_equals_live, None,
        "the last segment's replay check is not the run's"
    );
}

#[test]
fn a_resumed_run_whose_last_segment_stopped_keeps_that_stop_visible() {
    let f = facts_after(
        "two-segments-cap",
        "cap",
        Some(ASSISTANT),
        |dir, task, head| {
            moderator::record_evidence_segment(dir, task, head).unwrap();
        },
    )
    .expect("loads");
    assert_eq!(
        f.stop.as_deref(),
        Some("resumed: 2 bbp mod segments; last: cap")
    );
    assert!(f.over_cap && f.stopped && !f.succeeded());
}

#[test]
fn a_resumed_run_with_a_segment_from_another_build_is_refused() {
    let err = facts_after("two-builds", "wall", Some(ASSISTANT), |dir, task, _| {
        moderator::record_evidence_segment(dir, task, OTHER).unwrap();
    })
    .unwrap_err();
    assert!(err.contains("segment 2") && err.contains(OTHER), "{err}");
}

#[test]
fn a_segment_built_from_another_commit_than_the_output_reports_fails_the_run() {
    // The saved output reports the pinned HEAD, but the segment that produced
    // the run was another build: the output is not that run's.
    let err = facts_after("other-segment", "wall", Some(ASSISTANT), |dir, task, _| {
        std::fs::write(moderator::evidence_segments_path(dir, task), "").unwrap();
        moderator::record_evidence_segment(dir, task, OTHER).unwrap();
    })
    .unwrap_err();
    assert!(err.contains(OTHER), "{err}");
}

#[test]
fn a_run_with_no_segment_or_a_cut_off_one_is_refused() {
    let err = facts_after("no-segment", "wall", Some(ASSISTANT), |dir, task, _| {
        std::fs::remove_file(moderator::evidence_segments_path(dir, task)).unwrap();
    })
    .unwrap_err();
    assert!(err.contains("no bbp mod segment"), "{err}");
    let err = facts_after("empty-segments", "wall", Some(ASSISTANT), |dir, task, _| {
        std::fs::write(moderator::evidence_segments_path(dir, task), "").unwrap();
    })
    .unwrap_err();
    assert!(err.contains("no bbp mod segment"), "{err}");
    let err = facts_after("cut-segment", "wall", Some(ASSISTANT), |dir, task, head| {
        let path = moderator::evidence_segments_path(dir, task);
        std::fs::write(&path, format!("{{\"bbp_build_sha\":\"{head}\"")).unwrap();
    })
    .unwrap_err();
    assert!(err.contains("cut off"), "{err}");
    let err = facts_after("bad-segment", "wall", Some(ASSISTANT), |dir, task, _| {
        let path = moderator::evidence_segments_path(dir, task);
        std::fs::write(&path, "{\"build\":1}\n").unwrap();
    })
    .unwrap_err();
    assert!(err.contains("unreadable"), "{err}");
}

/// A marker in the format before segments were recorded (no `schema`) is
/// refused: such a run's segments are unknown. The new marker differs from
/// that format as text, which is what an older `bbp` compares, so an older
/// `bbp mod` will not resume a run started here, nor an older `bbp pairs`
/// count one.
#[test]
fn a_marker_from_before_segments_is_refused_and_an_older_bbp_refuses_the_new_one() {
    let mut new_marker = String::new();
    let err = facts_after("schema-1", "wall", Some(ASSISTANT), |dir, task, _| {
        let path = moderator::evidence_marker_path(dir, task);
        new_marker = std::fs::read_to_string(&path).unwrap();
        let old = new_marker.replace("\"schema\":2,", "");
        assert_ne!(old, new_marker);
        std::fs::write(&path, old).unwrap();
    })
    .unwrap_err();
    assert!(err.contains("records evidence differently"), "{err}");
    assert!(new_marker.starts_with("{\"schema\":2,"), "{new_marker}");
}
