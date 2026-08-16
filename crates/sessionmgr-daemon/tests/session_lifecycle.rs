//! The ordinary path, end to end, through the real binary: create, list,
//! attach, send input, close.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn a_new_session_is_created_running_and_listed() {
    let root = TempRoot::new("lifecycle-new");
    let id = session_new(root.path(), &long_running());

    assert_eq!(id.len(), 12, "session ids are 12 characters: {id}");
    let listing = session_list(root.path());
    assert!(listing.contains(&id));
    assert!(listing.contains("Running"));
    assert!(listing.contains("PlainTerminal"));
}

#[test]
fn a_client_starts_a_daemon_transparently() {
    // The user should not have to know this tool has a daemon at all.
    let root = TempRoot::new("lifecycle-autostart");
    assert_eq!(stdout_of(&run(root.path(), &["daemon", "status"])), "not running");

    session_new(root.path(), &long_running());
    assert!(stdout_of(&run(root.path(), &["daemon", "status"])).starts_with("running"));
}

#[test]
fn an_empty_state_root_lists_nothing_rather_than_failing() {
    let root = TempRoot::new("lifecycle-empty");
    assert_eq!(session_list(root.path()), "no sessions");
}

#[test]
fn a_session_that_exits_successfully_is_recorded_as_finished() {
    let root = TempRoot::new("lifecycle-finished");
    let command = echo("all-done");
    let id = session_new(
        root.path(),
        &command.iter().map(String::as_str).collect::<Vec<_>>(),
    );

    assert!(
        wait_until(
            || session_status(root.path(), &id) == "finished",
            Duration::from_secs(15),
        ),
        "a command that exits 0 should be Finished, but is {}",
        session_status(root.path(), &id)
    );
}

#[test]
fn a_session_that_exits_non_zero_is_recorded_as_errored() {
    let root = TempRoot::new("lifecycle-errored");
    let command: Vec<&str> = if cfg!(windows) {
        vec!["cmd", "/C", "exit 3"]
    } else {
        vec!["sh", "-c", "exit 3"]
    };
    let id = session_new(root.path(), &command);

    assert!(
        wait_until(
            || session_status(root.path(), &id) == "errored",
            Duration::from_secs(15),
        ),
        "a command that exits non-zero should be Errored, but is {}",
        session_status(root.path(), &id)
    );
}

#[test]
fn attaching_streams_the_sessions_output() {
    let root = TempRoot::new("lifecycle-attach");
    let command = if cfg!(windows) {
        vec!["cmd", "/C", "echo streamed-output && ping -n 600 127.0.0.1 > NUL"]
    } else {
        vec!["sh", "-c", "echo streamed-output; sleep 600"]
    };
    let id = session_new(root.path(), &command);

    let joined = attach_lines(root.path(), &id, 5, Duration::from_secs(15)).join("\n");
    assert!(
        joined.contains("streamed-output"),
        "attach should stream the session's output:\n{joined}"
    );
}

#[test]
fn closing_a_session_stops_both_its_worker_and_its_child() {
    // The pid pair, not just the worker: killing only the worker would
    // orphan the session's own process with nothing tracking it.
    let root = TempRoot::new("lifecycle-close");
    let id = session_new(root.path(), &long_running());
    let worker = worker_pid(root.path(), &id);
    let child = child_pid(root.path(), &id);

    assert_success("close", &run(root.path(), &["close", &id]));

    assert!(
        wait_until(|| !is_alive(worker), Duration::from_secs(10)),
        "the worker should be stopped by close"
    );
    assert!(
        wait_until(|| !is_alive(child), Duration::from_secs(10)),
        "the session's own process must not be left orphaned by close"
    );
    assert_eq!(session_status(root.path(), &id), "closed");
}

#[test]
fn closing_twice_is_rejected_rather_than_silently_succeeding() {
    let root = TempRoot::new("lifecycle-double-close");
    let id = session_new(root.path(), &long_running());
    assert_success("close", &run(root.path(), &["close", &id]));

    let second = run(root.path(), &["close", &id]);
    assert!(
        !second.status.success(),
        "a second close should fail rather than pretend to work"
    );
    assert_eq!(second.status.code(), Some(3), "conflicts exit 3");
}

#[test]
fn an_unknown_session_id_is_a_not_found_error() {
    let root = TempRoot::new("lifecycle-unknown");
    // Well-formed but nonexistent, so this exercises lookup rather than
    // validation.
    let output = run(root.path(), &["close", "zzzzzzzzzzzz"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4), "not-found exits 4");
}

#[test]
fn a_malformed_session_id_is_rejected_before_reaching_the_daemon() {
    // Ids become path components, so `../../etc` is a traversal attempt
    // and must be refused at the command line.
    let root = TempRoot::new("lifecycle-traversal");
    let output = run(root.path(), &["close", "../../etc"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2), "usage errors exit 2");
    // And it must not have started a daemon just to reject it.
    assert!(daemon_pid(root.path()).is_none());
}

#[test]
fn an_unknown_session_kind_fails_with_a_clear_message() {
    // An unrecognised `--kind` must say so rather than being silently
    // ignored and creating something the user did not ask for.
    let root = TempRoot::new("lifecycle-kind");
    let output = run(root.path(), &["new", "--kind", "nonsense"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("worktree") && stderr.contains("same-dir"),
        "the error should list the kinds that do exist: {stderr}"
    );
}

#[test]
fn a_repo_backed_kind_outside_a_repository_fails_clearly() {
    // `--kind worktree` pointed somewhere that is not a repository must
    // explain that, rather than failing somewhere deep in a git command.
    let root = TempRoot::new("lifecycle-norepo");
    let elsewhere = std::env::temp_dir().join(format!("smnr{}", std::process::id()));
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    let output = run(
        root.path(),
        &[
            "new",
            "--kind",
            "worktree",
            "--repo",
            &elsewhere.to_string_lossy(),
        ],
    );
    let _ = std::fs::remove_dir_all(&elsewhere);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("repository") || stderr.contains("git"),
        "the error should name the actual problem: {stderr}"
    );
}

#[test]
fn two_sessions_run_independently() {
    let root = TempRoot::new("lifecycle-two");
    let first = session_new(root.path(), &long_running());
    let second = session_new(root.path(), &long_running());
    assert_ne!(first, second);

    assert_ne!(
        worker_pid(root.path(), &first),
        worker_pid(root.path(), &second),
        "each session must get its own worker process"
    );

    assert_success("close", &run(root.path(), &["close", &first]));
    assert_eq!(session_status(root.path(), &second), "running");
}
