#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use rusty_bbp::{PrincipalId, Stage, TaskId, TurnId};
use rusty_serde::json::Value;

fn binding() -> Binding {
    Binding {
        dir: "/tmp/store with spaces".into(),
        task: TaskId("task\"x".into()),
        principal: PrincipalId("agent".into()),
        turn: TurnId(42),
    }
}

#[test]
fn reviewer_has_no_builtin_tools_or_extra_mcp_sources() {
    let args = arguments(
        Provider::Claude,
        &binding(),
        Path::new("/bin/bbp"),
        Path::new("/turn/mcp.json"),
        Path::new("/empty"),
        Some("review-model"),
    )
    .unwrap();
    for pair in [
        ["--tools", ""],
        ["--setting-sources", ""],
        ["--mcp-config", "/turn/mcp.json"],
        ["--permission-mode", "dontAsk"],
        ["--permission-prompts", "none"],
        ["--max-turns", "20"],
        ["--settings", "{\"disableAllHooks\":true}"],
    ] {
        assert!(args.windows(2).any(|w| w == pair));
    }
    for flag in [
        "--strict-mcp-config",
        "--restricted",
        "--disable-slash-commands",
        "--no-session-persistence",
    ] {
        assert!(args.iter().any(|a| a == flag));
    }
    assert!(!args
        .iter()
        .any(|a| a == "--bare" || a == "--resume" || a.contains("bypass")));
    assert!(!REVIEW_TOOLS.contains("put_artifact"));
    assert!(args.last().unwrap().contains("t42-"));
    assert_eq!(
        args[args.len() - 2],
        "--",
        "variadic config must not consume the prompt"
    );
}

#[test]
fn author_pins_one_required_mcp_server_and_uses_ephemeral_sandbox() {
    let args = arguments(
        Provider::Codex,
        &binding(),
        Path::new("/bin/bbp"),
        Path::new("/turn/mcp.json"),
        Path::new("/private/checkout"),
        None,
    )
    .unwrap();
    for flag in [
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--strict-config",
    ] {
        assert!(args.iter().any(|a| a == flag));
    }
    assert!(args
        .windows(2)
        .any(|w| w == ["--sandbox", "workspace-write"]));
    assert!(args.iter().any(|a| a.contains("mcp_servers={bbp={")
        && a.contains("required=true")
        && a.contains("BBP_TURN\"=\"42\"")
        && a.contains("task\\\"x")));
    assert!(args.iter().any(|a| a == "features.hooks=false"));
    assert!(args.iter().any(|a| a.contains("trust_level=\"untrusted\"")));
    assert!(!args
        .iter()
        .any(|a| a.contains("danger-full-access") || a.contains("bypass")));
}

#[test]
fn flags_cannot_weaken_the_fixed_launch_contract() {
    for args in [
        vec!["--tools", "Bash"],
        vec!["--model"],
        vec!["--model", "--tools"],
        vec!["--model", "x", "--model", "y"],
        vec!["--", "--resume"],
    ] {
        assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
    }
    let opts = Options::parse(
        [
            "--model",
            "m",
            "--provider-bin",
            "/cli",
            "--mcp-config",
            "/turn.json",
        ]
        .map(str::to_owned),
    )
    .unwrap();
    assert_eq!(opts.model.as_deref(), Some("m"));
}

#[test]
fn reviewer_capabilities_are_checked_instead_of_role_names() {
    let reviewer = Role::new("arbitrary", 1)
        .scoped()
        .verdict_at(&[Stage::Review])
        .final_decision();
    assert!(validate_role(Provider::Claude, &reviewer).is_ok());
    for role in [
        Role {
            read_messages: ReadScope::All,
            ..reviewer.clone()
        },
        Role {
            write: vec![ArtifactKind::Diff],
            ..reviewer.clone()
        },
        Role {
            read_artifacts: vec![ArtifactKind::Log],
            ..reviewer.clone()
        },
    ] {
        assert!(validate_role(Provider::Claude, &role).is_err());
    }
    assert!(validate_role(Provider::Codex, &reviewer).is_err());
    assert!(validate_role(
        Provider::Codex,
        &Role::new("maker", 0).write(&[ArtifactKind::Candidate])
    )
    .is_ok());
}

#[test]
fn mcp_json_carries_exact_binding_and_no_other_servers() {
    let value: Value =
        rusty_serde::json::from_str(&mcp_json(&binding(), Path::new("/bbp path")).unwrap())
            .unwrap();
    let server = value.get("mcpServers").unwrap().get("bbp").unwrap();
    assert_eq!(
        server
            .get("env")
            .unwrap()
            .get("BBP_TURN")
            .and_then(Value::as_str),
        Some("42")
    );
    assert_eq!(
        server
            .get("env")
            .unwrap()
            .get("BBP_TASK")
            .and_then(Value::as_str),
        Some("task\"x")
    );
    assert_eq!(
        server.get("command").and_then(Value::as_str),
        Some("/bbp path")
    );
}

#[test]
fn config_decoding_is_order_independent_but_rejects_extra_server_fields() {
    let text = mcp_json(&binding(), Path::new("/bbp")).unwrap();
    let expected: McpConfig = rusty_serde::json::from_str(&text).unwrap();
    let reordered = text.replace(
        "\"command\":\"/bbp\",\"args\":[\"mcp\"]",
        "\"args\":[\"mcp\"],\"command\":\"/bbp\"",
    );
    assert_eq!(
        rusty_serde::json::from_str::<McpConfig>(&reordered).unwrap(),
        expected
    );
    let extra = text.replace(
        "\"command\":",
        "\"url\":\"https://unexpected.invalid\",\"command\":",
    );
    assert!(rusty_serde::json::from_str::<McpConfig>(&extra).is_err());
}
