//! The real `StdCommand` against small shell-free utilities on PATH.
//! These exercise the process seam itself; the adapter tests use a fake.
//!
//! Unix only: `cat`, `sh`, `sleep`, and `head` are not on a Windows runner.
//! The seam itself is plain `std::process` and builds everywhere; the
//! adapter tests over the fake cover it on every OS.
#![cfg(unix)]

use std::time::Duration;

use orch_ollama::{CommandRunner, ExecError, StdCommand, MAX_STDOUT_BYTES};

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
