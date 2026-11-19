//! `--db FILE` puts the cards in the board's own SQLite file (ADR-0007), and conflicts with
//! `--tick-url` unless the point is to copy one into the other.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use rusty_mill_board::domain::Column;
use rusty_mill_board::sqlite::SqliteStore;
use rusty_mill_board::store::Store;
use uuid::Uuid;

/// The board with only what the OS needs of the caller's environment.
fn board(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty_mill_board"));
    command.env_clear().env("PATH", "/usr/bin:/bin").args(args);
    command
}

/// One MCP session over stdio: initialize, then each request in turn, then the end of input.
fn session(command: &mut Command, calls: &[&str]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    for line in std::iter::once(&init).chain(calls) {
        writeln!(stdin, "{line}").expect("write");
    }
    drop(stdin);
    child.wait_with_output().expect("output")
}

fn call(id: u32, tool: &str, args: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    )
}

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mill_db_flag_{name}_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

#[test]
fn cards_added_over_mcp_are_in_the_file_and_survive_a_restart() {
    let dir = dir("persist");
    let file = dir.join("board.db");
    let arg = file.to_str().expect("utf8");
    let who = r#""actor":"nano","role":"human""#;
    let add = call(1, "card_add", &format!(r#"{{{who},"title":"kept"}}"#));
    let out = session(&mut board(&["--db", arg]), &[&add]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stored = SqliteStore::open(&file)
        .expect("open")
        .list()
        .expect("list");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].title, "kept");
    assert_eq!(stored[0].column, Column::BACKLOG);

    let list = call(1, "board_list", &format!("{{{who}}}"));
    let out = session(&mut board(&["--db", arg]), &[&list]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("kept"),
        "a second start reads the first's card: {text}"
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn without_db_the_board_stays_in_memory() {
    let who = r#""actor":"nano","role":"human""#;
    let add = call(1, "card_add", &format!(r#"{{{who},"title":"gone"}}"#));
    let list = call(2, "board_list", &format!("{{{who}}}"));
    let out = session(&mut board(&[]), &[&add, &list]);
    assert!(out.status.success());
    let started = std::env::current_dir().expect("cwd");
    assert!(!Path::new("board.db").exists() && !started.join("board.db").exists());
}

#[test]
fn db_and_tick_url_together_are_refused_before_anything_opens() {
    let dir = dir("both");
    let file = dir.join("board.db");
    let out = board(&[
        "--db",
        file.to_str().expect("utf8"),
        "--tick-url",
        "http://127.0.0.1:9",
    ])
    .env("RUSTY_TICK_TOKEN", "t")
    .output()
    .expect("run");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("two stores") && err.contains("--migrate-from-tick"),
        "{err}"
    );
    assert!(!file.exists(), "the file was not created");
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn migrating_needs_both_stores_and_a_dry_run_needs_a_migration() {
    for args in [
        &["--migrate-from-tick", "--db", "x.db"][..],
        &["--migrate-from-tick", "--tick-url", "http://127.0.0.1:9"][..],
        &["--dry-run"][..],
    ] {
        let out = board(args).output().expect("run");
        assert!(!out.status.success(), "{args:?} was accepted");
    }
}

#[test]
fn a_file_that_is_not_a_database_fails_with_its_path() {
    let dir = dir("bad");
    let file = dir.join("board.db");
    std::fs::write(&file, "not sqlite, and long enough to not be an empty file").expect("write");
    let out = board(&["--db", file.to_str().expect("utf8")])
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("board.db"));
    std::fs::remove_dir_all(dir).expect("cleanup");
}

/// Every file under `dir` with its bytes.
fn files(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .expect("dir")
        .map(|e| e.expect("entry").path())
        .map(|p| (p.display().to_string(), std::fs::read(&p).expect("read")))
        .collect()
}

#[test]
fn the_lift_dry_run_reads_the_file_without_creating_migrating_or_touching_anything() {
    // A directory name with characters a URI would mangle.
    let dir = dir("lift plan#1");
    let file = dir.join("board.db");
    let arg = file.to_str().expect("utf8");
    {
        let store = SqliteStore::open(&file).expect("open");
        let card = rusty_mill_board::domain::Card::new("t", "## Why\nBecause.\n", vec![], 1)
            .expect("card");
        store
            .insert(
                &card,
                rusty_mill_board::domain::Event::of(
                    &card,
                    rusty_mill_board::domain::EventKind::Created,
                    "t",
                ),
            )
            .expect("insert");
    }
    let before = files(&dir);
    let out = board(&["--db", arg, "--lift-sections"])
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Because."));
    assert!(stderr.contains("1 card(s) would change"), "{stderr}");
    assert!(files(&dir) == before, "the dry run changed or added a file");

    let missing = dir.join("none.db");
    let out = board(&["--db", missing.to_str().expect("utf8"), "--lift-sections"])
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(!missing.exists(), "the dry run created the file");
}
