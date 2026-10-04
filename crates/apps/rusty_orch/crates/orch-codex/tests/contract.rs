//! Layer 1: pure facts about the adapter, no process.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{fixture, task, FakeCommand, REPLY_ONE, REPLY_TWO};
use orch_cli::parse;
use orch_codex::{output_schema, render, CodexAgent, SCRUBBED_ENV};
use orch_core::goal::StopRule;
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
            "--ignore-rules",
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
    assert_eq!(&pinned[9..11], ["-m", "gpt-5-codex"]);
    assert_eq!(SCRUBBED_ENV, ["OPENAI_API_KEY"]);
    assert_eq!(orch_codex::DEFAULT_TIMEOUT, Duration::from_secs(600));
}

/// Strict Structured Outputs: every object lists every property in
/// `required` and forbids additional properties, recursively, including
/// inside `anyOf` branches and array items. Keywords outside the strict
/// subset (size caps, formats) are absent; the parser enforces those.
fn assert_strict(node: &rusty_json::Value, path: &str) {
    const OUTSIDE_STRICT_SUBSET: [&str; 6] = [
        "minItems",
        "maxItems",
        "minLength",
        "maxLength",
        "format",
        "pattern",
    ];
    if let Some(obj) = node.as_object() {
        for key in OUTSIDE_STRICT_SUBSET {
            assert!(
                obj.get(key).is_none(),
                "{path}: {key} is outside the strict subset"
            );
        }
    }
    if node.get("type").and_then(|t| t.as_str()) == Some("object") {
        assert_eq!(
            node.get("additionalProperties").and_then(|v| v.as_bool()),
            Some(false),
            "{path}: additionalProperties must be false"
        );
        let props = node
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap_or_else(|| panic!("{path}: object without properties"));
        let mut names: Vec<&str> = props.iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        let mut required: Vec<&str> = node
            .get("required")
            .and_then(|r| r.as_array())
            .unwrap_or_else(|| panic!("{path}: object without required"))
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        required.sort_unstable();
        assert_eq!(required, names, "{path}: every property must be required");
        for (name, child) in props.iter() {
            assert_strict(child, &format!("{path}.{name}"));
        }
    }
    if let Some(items) = node.get("items") {
        assert_strict(items, &format!("{path}[]"));
    }
    if let Some(branches) = node.get("anyOf").and_then(|a| a.as_array()) {
        for (i, branch) in branches.iter().enumerate() {
            assert_strict(branch, &format!("{path}|{i}"));
        }
    }
}

#[test]
fn schema_is_strict_structured_outputs_compatible() {
    for stop in [StopRule::Checkpoint, StopRule::BestEffort] {
        let schema = rusty_json::Value::parse(&output_schema(stop)).expect("schema parses");
        assert_strict(&schema, "$");
    }
}

/// Under best effort the schema itself has no `question` variant, so a
/// schema-constrained model cannot form one; the parser agrees (ADR-0011).
#[test]
fn best_effort_schema_has_no_question_variant() {
    let schema = rusty_json::Value::parse(&output_schema(StopRule::BestEffort)).expect("parses");
    let mut kinds = schema_kinds(&schema);
    kinds.sort_unstable();
    assert_eq!(kinds, ["assumption", "finding", "review"]);
    assert_eq!(
        orch_cli::allowed_kinds(Role::Research, StopRule::BestEffort),
        ["finding", "assumption"]
    );
}

fn schema_kinds(schema: &rusty_json::Value) -> Vec<&str> {
    schema
        .get("properties")
        .and_then(|p| p.get("entries"))
        .and_then(|e| e.get("items"))
        .and_then(|i| i.get("anyOf"))
        .and_then(|a| a.as_array())
        .expect("one variant per kind")
        .iter()
        .filter_map(|v| {
            v.get("properties")?
                .get("kind")?
                .get("enum")?
                .as_array()?
                .first()?
                .as_str()
        })
        .collect()
}

#[test]
fn schema_agrees_with_the_parser() {
    let schema =
        rusty_json::Value::parse(&output_schema(StopRule::Checkpoint)).expect("schema parses");
    let variants = schema
        .get("properties")
        .and_then(|p| p.get("entries"))
        .and_then(|e| e.get("items"))
        .and_then(|i| i.get("anyOf"))
        .and_then(|a| a.as_array())
        .expect("one variant per kind");
    let mut kinds: Vec<&str> = variants
        .iter()
        .filter_map(|v| {
            v.get("properties")?
                .get("kind")?
                .get("enum")?
                .as_array()?
                .first()?
                .as_str()
        })
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["assumption", "finding", "question", "review"]);
    // Kind-dependent fields are required in exactly the variant that needs them.
    let requires = |kind: &str, field: &str| {
        variants.iter().any(|v| {
            v.get("properties")
                .and_then(|p| p.get("kind"))
                .and_then(|k| k.get("enum"))
                .and_then(|e| e.as_array())
                .and_then(|e| e.first())
                .and_then(|k| k.as_str())
                == Some(kind)
                && v.get("required")
                    .and_then(|r| r.as_array())
                    .is_some_and(|r| r.iter().any(|x| x.as_str() == Some(field)))
        })
    };
    assert!(requires("finding", "confidence") && !requires("question", "confidence"));
    assert!(requires("review", "verdict") && !requires("finding", "verdict"));
    // Both ADR-0004 example replies satisfy the schema's shape and the parser.
    assert_eq!(
        parse(REPLY_ONE, Role::Research, StopRule::Checkpoint)
            .expect("one")
            .len(),
        1
    );
    assert_eq!(
        parse(REPLY_TWO, Role::Research, StopRule::Checkpoint)
            .expect("two")
            .len(),
        2
    );
}

#[test]
fn render_keeps_path_refs_as_refs_and_inlines_only_referenced_entries() {
    let (plan, board, referenced, unreferenced) = fixture();
    let prompt = render(task(&plan), &board, StopRule::Checkpoint);

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
