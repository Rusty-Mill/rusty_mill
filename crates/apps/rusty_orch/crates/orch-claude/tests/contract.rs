//! Layer 1: pure facts about the adapter, no process.

mod common;

use std::time::Duration;

use common::{fixture, task, FakeCommand};
use orch_claude::{render, ClaudeAgent, DEFAULT_MAX_TURNS, SCRUBBED_ENV, TOOLS};
use orch_cli::output_schema;
use orch_core::goal::StopRule;
use orch_core::Ref;

#[test]
fn argv_is_pinned() {
    let agent = ClaudeAgent::with_runner("/repo", FakeCommand::ok(""));
    let argv = agent.argv();
    let fixed = [
        "claude",
        "-p",
        "--output-format",
        "json",
        "--tools",
        "Read,Grep,Glob",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--restricted",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--max-turns",
        "20",
        "--json-schema",
    ];
    assert_eq!(&argv[..fixed.len()], fixed);
    assert_eq!(argv[fixed.len()], output_schema(StopRule::Checkpoint));
    assert_eq!(argv.len(), fixed.len() + 1);
    assert_eq!(TOOLS, "Read,Grep,Glob");
    assert_eq!(DEFAULT_MAX_TURNS, 20);

    let pinned = ClaudeAgent::with_runner("/repo", FakeCommand::ok(""))
        .model("opus")
        .max_turns(7)
        .argv();
    assert_eq!(&pinned[16..20], ["--max-turns", "7", "--model", "opus"]);

    let best_effort = agent.with_stop_rule(StopRule::BestEffort).argv();
    assert_eq!(
        best_effort[best_effort.len() - 1],
        output_schema(StopRule::BestEffort)
    );
    assert!(!best_effort[best_effort.len() - 1].contains("\"question\""));
}

#[test]
fn argv_never_grants_write_shell_or_bypass() {
    let argv = ClaudeAgent::with_runner("/repo", FakeCommand::ok("")).argv();
    for forbidden in [
        "--bare",
        "--dangerously-skip-permissions",
        "--allow-dangerously-skip-permissions",
        "bypassPermissions",
        "acceptEdits",
        "--allowedTools",
        "--add-dir",
    ] {
        assert!(!argv.iter().any(|a| a == forbidden), "{forbidden}");
    }
    assert!(!argv
        .iter()
        .any(|a| a.contains("Bash") || a.contains("Edit") || a.contains("Write")));
}

#[test]
fn constants_are_pinned() {
    assert_eq!(
        SCRUBBED_ENV,
        [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
        ]
    );
    assert_eq!(orch_claude::DEFAULT_TIMEOUT, Duration::from_secs(600));
    assert_eq!(orch_claude::AUTH_TIMEOUT, Duration::from_secs(60));
    assert_eq!(
        ClaudeAgent::<FakeCommand>::auth_argv(),
        ["claude", "auth", "status"]
    );
    let agent = ClaudeAgent::with_runner("/repo", FakeCommand::ok(""));
    assert_eq!(agent.repo_root(), std::path::Path::new("/repo"));
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
    assert!(prompt.contains("read-only and there is no shell or network"));
    assert!(matches!(task(&plan).spec().refs[1], Ref::Path(_)));
}
