//! `rusty_tick user ...` through the real binary: argument handling, the
//! token on stdout, errors on stderr with a failing exit code.

use rusty_tick::users::Registry;
use std::process::{Command, Output};

fn tick(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusty_tick"))
        .args(args)
        .arg("--data-dir")
        .arg(dir)
        .env_remove("RUSTY_TICK_TOKEN")
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn add_prints_only_the_token_and_list_shows_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let added = tick(dir.path(), &["user", "add", "alice", "phone"]);
    assert!(added.status.success());
    let token = stdout(&added);
    assert!(token.starts_with("alice.") && token.ends_with('\n') && token.lines().count() == 1);

    let registry = Registry::load(&dir.path().join("users.json")).unwrap();
    assert!(registry.authenticate(token.trim()).is_some());
    let listed = tick(dir.path(), &["user", "list"]);
    assert!(stdout(&listed).starts_with("alice\n"));
}

#[test]
fn a_failure_exits_nonzero_with_the_reason_on_stderr_and_nothing_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    for args in [&["user", "revoke", "alice", "x"][..], &["user"], &["frob"]] {
        let out = tick(dir.path(), args);
        assert!(!out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
    }
}
