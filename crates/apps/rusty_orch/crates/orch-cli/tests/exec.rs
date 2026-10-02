//! The real `StdCommand` against small shell-free utilities on PATH.
//! These exercise the process seam itself; the adapter tests use a fake.
//!
//! Unix only: `cat`, `sh`, `sleep`, and `head` are not on a Windows runner.
//! The seam itself is plain `std::process` and builds everywhere; the
//! adapter tests over the fake cover it on every OS.
#![cfg(unix)]

use std::time::Duration;

use orch_cli::{CommandRunner, ExecError, StdCommand, JOIN_GRACE, MAX_STDOUT_BYTES};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn echoes_stdin_back_through_stdout() {
    let exit = StdCommand
        .run(&argv(&["cat"]), b"hello from stdin", Duration::from_secs(5))
        .expect("cat runs");
    assert_eq!(exit.status, 0);
    assert_eq!(exit.stdout, b"hello from stdin");
}

#[test]
fn reports_non_zero_exit_and_stderr() {
    let exit = StdCommand
        .run(
            &argv(&["sh", "-c", "echo oops >&2; exit 3"]),
            b"",
            Duration::from_secs(5),
        )
        .expect("sh runs");
    assert_eq!(exit.status, 3);
    assert_eq!(String::from_utf8_lossy(&exit.stderr).trim(), "oops");
}

#[test]
fn kills_and_reaps_on_timeout() {
    let timeout = Duration::from_millis(200);
    let started = std::time::Instant::now();
    let err = StdCommand
        .run(&argv(&["sleep", "30"]), b"", timeout)
        .expect_err("times out");
    assert_eq!(err, ExecError::Timeout(timeout));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn overflowing_stdout_is_an_error() {
    let err = StdCommand
        .run(
            &argv(&[
                "head",
                "-c",
                &(MAX_STDOUT_BYTES + 1).to_string(),
                "/dev/zero",
            ]),
            b"",
            Duration::from_secs(10),
        )
        .expect_err("overflows");
    assert_eq!(err, ExecError::StdoutOverflow);
}

#[test]
fn missing_program_is_a_spawn_error() {
    let err = StdCommand
        .run(
            &argv(&["definitely-not-a-program-zzz"]),
            b"",
            Duration::from_secs(5),
        )
        .expect_err("missing");
    assert!(matches!(err, ExecError::Spawn(_)));
}

#[test]
fn empty_argv_is_a_spawn_error() {
    assert!(matches!(
        StdCommand.run(&[], b"", Duration::from_secs(1)),
        Err(ExecError::Spawn(_))
    ));
}

/// Writes the child's pgid (== its pid under `process_group(0)`) to a file
/// the test can read after the run has been torn down.
fn pgid_file() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("orch-ollama-pgid-{}", std::process::id()));
    p
}

/// `kill -0 -- -<pgid>` succeeds while any member of the group exists.
fn group_alive(pgid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", "--", &format!("-{pgid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .expect("kill runs")
}

#[test]
fn timeout_kills_the_whole_group_and_joins_within_the_bound() {
    let file = pgid_file();
    let script = format!("echo $$ > {}; sleep 30 & sleep 30", file.display());
    let timeout = Duration::from_secs(1);
    let started = std::time::Instant::now();

    let err = StdCommand
        .run(&argv(&["sh", "-c", &script]), b"", timeout)
        .expect_err("times out");

    assert_eq!(err, ExecError::Timeout(timeout));
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");

    let pgid = std::fs::read_to_string(&file)
        .expect("pgid file")
        .trim()
        .to_owned();
    let _ = std::fs::remove_file(&file);
    // Reaping of the orphaned grandchild is init's job; give it a moment.
    let gone = (0..20).any(|_| {
        if group_alive(&pgid) {
            std::thread::sleep(Duration::from_millis(100));
            false
        } else {
            true
        }
    });
    assert!(gone, "process group {pgid} still has members");
}

#[test]
fn overflow_with_a_forking_child_returns_within_the_bound() {
    let n = MAX_STDOUT_BYTES + 1;
    // The subshell overflows stdout; the parent then holds stdout open.
    let script = format!("(head -c {n} /dev/zero) & sleep 30");
    let started = std::time::Instant::now();

    let err = StdCommand
        .run(&argv(&["sh", "-c", &script]), b"", Duration::from_secs(10))
        .expect_err("overflows");

    assert_eq!(err, ExecError::StdoutOverflow);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
}

#[test]
fn grandchild_holding_stdout_after_exit_is_bounded_by_the_grace_period() {
    // The parent exits at once; the backgrounded grandchild keeps stdout.
    let started = std::time::Instant::now();

    let err = StdCommand
        .run(
            &argv(&["sh", "-c", "sleep 30 & exit 0"]),
            b"",
            Duration::from_secs(10),
        )
        .expect_err("stdout held open");

    assert!(
        matches!(err, ExecError::Io(ref m) if m.contains("still open")),
        "{err:?}"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < JOIN_GRACE + Duration::from_secs(2),
        "took {elapsed:?}"
    );
}

#[test]
fn scrubbed_variables_are_removed_from_the_child_environment() {
    std::env::set_var("ORCH_TEST_SECRET", "leaked");
    let argv = argv(&["sh", "-c", "echo ${ORCH_TEST_SECRET:-unset}"]);

    let visible = StdCommand
        .run(&argv, b"", Duration::from_secs(5))
        .expect("sh runs");
    let scrubbed = StdCommand
        .run_scrubbed(&argv, b"", Duration::from_secs(5), &["ORCH_TEST_SECRET"])
        .expect("sh runs");

    assert_eq!(String::from_utf8_lossy(&visible.stdout).trim(), "leaked");
    assert_eq!(String::from_utf8_lossy(&scrubbed.stdout).trim(), "unset");
}
