#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use rusty_bbp::{PrincipalId, Stage, TaskId, TurnId};
use rusty_serde::json::Value;

fn direct() -> Channel {
    Channel::Direct(binding())
}

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
        &direct(),
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
        &direct(),
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
    assert!(validate_role(Provider::Claude, &Capabilities::of(&reviewer)).is_ok());
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
        assert!(validate_role(Provider::Claude, &Capabilities::of(&role)).is_err());
    }
    assert!(validate_role(Provider::Codex, &Capabilities::of(&reviewer)).is_err());
    assert!(validate_role(
        Provider::Codex,
        &Capabilities::of(&Role::new("maker", 0).write(&[ArtifactKind::Candidate]))
    )
    .is_ok());
}

#[test]
fn mcp_json_carries_exact_binding_and_no_other_servers() {
    let value: Value =
        rusty_serde::json::from_str(&mcp_json(&direct(), Path::new("/bbp path")).unwrap()).unwrap();
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
    let text = mcp_json(&direct(), Path::new("/bbp")).unwrap();
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

#[test]
fn author_preapproves_only_named_bbp_agent_tools_and_scrubs_git_context() {
    let args = arguments(
        Provider::Codex,
        &direct(),
        Path::new("/bbp"),
        Path::new("/mcp"),
        Path::new("/clone"),
        None,
    )
    .unwrap();
    let config = args.iter().find(|a| a.starts_with("mcp_servers=")).unwrap();
    for tool in ["task_card", "read", "get_artifact", "put_artifact", "post"] {
        assert!(config.contains(&format!("{tool}={{approval_mode=\"approve\"}}")));
    }
    assert!(!config.contains("default_tools_approval_mode"));
    assert!(args.iter().any(|a| a == "approval_policy=\"never\""));
    assert!(args
        .iter()
        .any(|a| a.starts_with("shell_environment_policy.exclude=") && a.contains("GIT_*")));
    assert!(args
        .iter()
        .any(|a| a.contains("GIT_CONFIG_GLOBAL=\"/dev/null\"")));
}

fn facts(provider: Provider) -> Facts {
    let role = match provider {
        Provider::Codex => Role::new("maker", 0).write(&[ArtifactKind::Candidate]),
        Provider::Claude => Role::new("auditor", 1)
            .scoped()
            .verdict_at(&[Stage::Review])
            .final_decision(),
    };
    Facts {
        repo: "/repo".into(),
        review: provider == Provider::Claude,
        live: true,
        role: Some(Capabilities::of(&role)),
        vendor: Some(provider.name().into()),
        candidate_vendors: vec!["codex".into()],
    }
}

#[test]
fn the_same_checks_refuse_a_launch_whatever_its_facts_came_from() {
    for provider in [Provider::Codex, Provider::Claude] {
        assert_eq!(check(provider, &facts(provider)), Ok(()));
        for (broken, error) in [
            (
                Facts {
                    role: None,
                    ..facts(provider)
                },
                "not an assigned agent",
            ),
            (
                Facts {
                    live: false,
                    ..facts(provider)
                },
                "live turn",
            ),
            (
                Facts {
                    vendor: Some("other".into()),
                    ..facts(provider)
                },
                "vendor differs",
            ),
        ] {
            assert!(
                check(provider, &broken).unwrap_err().contains(error),
                "{error}"
            );
        }
    }
    assert!(check(Provider::Codex, &facts(Provider::Claude)).is_err());
    let planning = Facts {
        review: false,
        ..facts(Provider::Claude)
    };
    assert!(check(Provider::Claude, &planning)
        .unwrap_err()
        .contains("Review turn"));
    let same = Facts {
        candidate_vendors: vec!["claude".into()],
        ..facts(Provider::Claude)
    };
    assert!(check(Provider::Claude, &same)
        .unwrap_err()
        .contains("also authors"));
}

#[test]
fn facts_survive_their_file() {
    let f = facts(Provider::Claude);
    let back: Facts = rusty_serde::json::from_str(&f.to_json().unwrap()).unwrap();
    assert_eq!(back, f);
}

#[test]
fn a_relayed_provider_gets_only_the_relay_and_keeps_its_restrictions() {
    let relay = Channel::Relay {
        task: TaskId("t".into()),
        turn: TurnId(7),
        pipes: "/work/turn-7.mcp".into(),
    };
    let config: McpConfig =
        rusty_serde::json::from_str(&mcp_json(&relay, Path::new("/bbp")).unwrap()).unwrap();
    assert_eq!(
        config.servers["bbp"].env,
        BTreeMap::from([("BBP_MCP_RELAY".to_owned(), "/work/turn-7.mcp".to_owned())])
    );
    let args = arguments(
        Provider::Codex,
        &relay,
        Path::new("/bbp"),
        Path::new("/mcp"),
        Path::new("/clone"),
        None,
    )
    .unwrap();
    let config = args.iter().find(|a| a.starts_with("mcp_servers=")).unwrap();
    assert!(
        config.contains("\"BBP_MCP_RELAY\"=\"/work/turn-7.mcp\"") && !config.contains("BBP_DIR")
    );
    assert!(args
        .windows(2)
        .any(|w| w == ["--sandbox", "workspace-write"]));
    assert!(args.last().unwrap().contains("t7-"));
}
