#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use rusty_bbp::{PrincipalId, Stage, TaskId, TurnId};
use rusty_serde::json::Value;

fn direct() -> Channel {
    Channel::Direct(binding(), TurnId(42))
}

fn binding() -> Binding {
    Binding {
        dir: "/tmp/store with spaces".into(),
        task: TaskId("task\"x".into()),
        principal: PrincipalId("agent".into()),
        turn: Some(TurnId(42)),
        hidden: vec![],
        view: None,
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
        None,
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

#[test]
fn a_session_is_both_variables_a_uuid_and_yes_or_no() {
    let id = "0123abcd-0123-4567-89ab-0123456789ab";
    let some = |v: &str| Some(v.to_owned());
    assert_eq!(Session::parse(None, None, None), Ok(None));
    assert_eq!(
        Session::parse(some(id), some("yes"), None),
        Ok(Some(Session {
            id: id.into(),
            resume: true,
            provider: None,
        }))
    );
    for (i, r) in [
        (some(id), None),
        (None, some("no")),
        (some("0123ABCD-0123-4567-89ab-0123456789ab"), some("no")),
        (some("../../etc/passwd"), some("no")),
        (some(id), some("maybe")),
    ] {
        assert!(Session::parse(i, r, None).is_err());
    }
}

#[test]
fn a_reviewer_session_starts_then_resumes_and_persists_only_then() {
    let args = |session: Option<&Session>| {
        arguments(
            Provider::Claude,
            &direct(),
            Path::new("/bin/bbp"),
            Path::new("/turn/mcp.json"),
            Path::new("/empty"),
            None,
            session,
        )
        .unwrap()
    };
    let id = "0123abcd-0123-4567-89ab-0123456789ab".to_owned();
    let has = |a: &[String], pair: [&str; 2]| a.windows(2).any(|w| w == pair);
    let none = args(None);
    assert!(none.iter().any(|a| a == "--no-session-persistence"));
    assert!(!none.iter().any(|a| a == "--resume" || a == "--session-id"));
    let first = args(Some(&Session {
        id: id.clone(),
        resume: false,
        provider: None,
    }));
    assert!(has(&first, ["--session-id", &id]));
    let later = args(Some(&Session {
        id: id.clone(),
        resume: true,
        provider: None,
    }));
    assert!(has(&later, ["--resume", &id]));
    for a in [&first, &later] {
        assert!(!a.iter().any(|x| x == "--no-session-persistence"));
        // The prompt still comes last, after the separator.
        assert_eq!(a[a.len() - 2], "--");
    }
}

const THREAD: &str = "01a21ab4-d922-7602-801b-328d139e9aa7";

#[test]
fn a_provider_session_is_a_uuid_on_a_resumed_turn_only() {
    let id = "0123abcd-0123-4567-89ab-0123456789ab";
    let some = |v: &str| Some(v.to_owned());
    let resumed = Session::parse(some(id), some("yes"), some(THREAD)).unwrap();
    assert_eq!(resumed.unwrap().provider.as_deref(), Some(THREAD));
    for (r, p) in [
        (some("no"), some(THREAD)),
        (some("yes"), some("01A21AB4-d922-7602-801b-328d139e9aa7")),
        (some("yes"), some("../thread")),
    ] {
        assert!(Session::parse(some(id), r, p).is_err());
    }
    assert!(Session::parse(None, None, some(THREAD)).is_err());
}

fn codex_args(session: Option<&Session>) -> Result<Vec<String>, String> {
    arguments(
        Provider::Codex,
        &direct(),
        Path::new("/bin/bbp"),
        Path::new("/turn/mcp.json"),
        Path::new("/clone"),
        Some("gpt-x"),
        session,
    )
}

#[test]
fn a_codex_session_persists_its_first_turn_and_resumes_the_captured_thread() {
    let id = "0123abcd-0123-4567-89ab-0123456789ab".to_owned();
    let session = |resume: bool, provider: Option<&str>| Session {
        id: id.clone(),
        resume,
        provider: provider.map(str::to_owned),
    };
    let ephemeral = |a: &[String]| a.iter().any(|x| x == "--ephemeral");
    let resumes = |a: &[String]| a.iter().any(|x| x == "resume");
    let none = codex_args(None).unwrap();
    assert!(ephemeral(&none) && !resumes(&none));
    let first = codex_args(Some(&session(false, None))).unwrap();
    assert!(!ephemeral(&first) && !resumes(&first));
    let later = codex_args(Some(&session(true, Some(THREAD)))).unwrap();
    assert!(!ephemeral(&later));
    let at = later.iter().position(|x| x == "resume").unwrap();
    // `resume <id>` comes after every global flag and override, right before
    // the prompt; the sandbox is set before it.
    assert_eq!(later[at + 1], THREAD);
    assert_eq!(later[at + 2], "--");
    assert_eq!(at + 4, later.len());
    let sandbox = later.iter().position(|x| x == "--sandbox").unwrap();
    assert!(sandbox < at && later[sandbox + 1] == "workspace-write");
    assert!(later[..at].iter().any(|x| x.starts_with("mcp_servers=")));
    // The derived BBP id never reaches Codex.
    assert!(!later.contains(&id));
    let err = codex_args(Some(&session(true, None))).unwrap_err();
    assert!(err.contains("captured from its first turn"), "{err}");
}

#[test]
fn the_codex_thread_id_is_the_first_thread_started_before_any_turn_event() {
    let started = format!("{{\"type\":\"thread.started\",\"thread_id\":\"{THREAD}\"}}");
    let ok = |log: &str| codex_thread_id(log).unwrap();
    assert_eq!(ok(&started), Some(THREAD.into()));
    // Harness and broker lines may come first.
    let noisy =
        format!("bbp: relay up\nReading additional input from stdin...\n{{oops\n  {started}\n");
    assert_eq!(ok(&noisy), Some(THREAD.into()));
    // An id after the agent's output began is not Codex's.
    for early in [
        "{\"type\":\"turn.started\"}",
        "{\"type\":\"item.completed\",\"item\":{}}",
    ] {
        assert_eq!(ok(&format!("{early}\n{started}")), None);
    }
    assert_eq!(ok(""), None);
    // A Claude stream never starts with one.
    assert_eq!(ok("{\"type\":\"system\",\"session_id\":\"x\"}\n"), None);
    // Only the first counts.
    let other = started.replace(THREAD, "0123abcd-0123-4567-89ab-0123456789ab");
    assert_eq!(ok(&format!("{started}\n{other}")), Some(THREAD.into()));
    for bad in [
        "{\"type\":\"thread.started\"}".to_owned(),
        started.replace(THREAD, "../../etc"),
    ] {
        assert!(codex_thread_id(&bad).is_err(), "{bad}");
    }
}
