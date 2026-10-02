//! Layer 1: pure facts about the adapter, no process.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{fixture, task, FakeCommand, REPLY_ONE, REPLY_TWO};
use orch_cli::parse;
use orch_codex::{render, CodexAgent, OUTPUT_SCHEMA, SCRUBBED_ENV};
use orch_core::task::Role;
use orch_core::Ref;

#[test]
fn argv_is_pinned() {
    let agent = CodexAgent::with_runner("/repo", FakeCommand::ok(""));
    let argv = agent.argv(Path::new("/tmp/s.json"), Path::new("/tmp/r.json"));
    assert_eq!(
        argv,
        [
            "codex",
            "exec",
            "--sandbox",
            "read-only",
            "--ephemeral",
            "--ignore-user-config",
            "-C",
            "/repo",
            "--output-schema",
            "/tmp/s.json",
            "--output-last-message",
            "/tmp/r.json",
            "-",
        ]
    );
    let pinned = agent
        .model("gpt-5-codex")
        .argv(Path::new("/s"), Path::new("/r"));
    assert_eq!(&pinned[8..10], ["-m", "gpt-5-codex"]);
    assert_eq!(SCRUBBED_ENV, ["OPENAI_API_KEY"]);
    assert_eq!(orch_codex::DEFAULT_TIMEOUT, Duration::from_secs(600));
}

#[test]
fn schema_is_valid_json_and_agrees_with_the_parser() {
    let schema = rusty_json::Value::parse(OUTPUT_SCHEMA).expect("schema parses");
    let entries = schema
        .get("properties")
        .and_then(|p| p.get("entries"))
        .expect("entries property");
    assert_eq!(entries.get("maxItems").and_then(|v| v.as_u64()), Some(8));
    let kinds = entries
        .get("items")
        .and_then(|i| i.get("properties"))
        .and_then(|p| p.get("kind"))
        .and_then(|k| k.get("enum"))
        .and_then(|e| e.as_array())
        .expect("kind enum");
    let kinds: Vec<&str> = kinds.iter().filter_map(|k| k.as_str()).collect();
    assert_eq!(kinds, ["finding", "question", "assumption", "review"]);
    // Both ADR-0004 example replies satisfy the schema's shape and the parser.
    assert_eq!(parse(REPLY_ONE, Role::Research).expect("one").len(), 1);
    assert_eq!(parse(REPLY_TWO, Role::Research).expect("two").len(), 2);
}

#[test]
fn render_keeps_path_refs_as_refs_and_inlines_only_referenced_entries() {
    let (plan, board, referenced, unreferenced) = fixture();
    let prompt = render(task(&plan), &board);

    assert!(prompt.contains("- path:crates/orch-core/src/task.rs"));
    assert!(
        !prompt.contains("pub fn start"),
        "file content must not be inlined"
    );
    assert!(prompt.contains(&format!("{referenced} [Finding")));
    assert!(!prompt.contains(&format!("{unreferenced} [")));
    assert!(prompt.contains("read files under the working directory"));
    assert!(prompt.contains("read-only and offline"));
    assert!(matches!(task(&plan).spec().refs[1], Ref::Path(_)));
}
