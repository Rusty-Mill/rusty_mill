use rusty_uuid::Uuid;

use super::*;

/// Parses with no defaults.
fn parse(args: &[String]) -> Result<Invocation, String> {
    super::parse(args, &Defaults::default())
}

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(String::from).collect()
}

fn calls(line: &str) -> Vec<Call> {
    match parse(&args(line)).expect("parses").command {
        Command::Calls(calls) => calls,
        other => panic!("expected calls, got {other:?}"),
    }
}

const ID: &str = "00000000-0000-0000-0000-000000000009";

#[test]
fn global_flags_come_first() {
    let invocation = parse(&args("--json --socket /tmp/s create-session --name main")).unwrap();
    assert_eq!(invocation.socket, Some(PathBuf::from("/tmp/s")));
    assert!(invocation.json);
    let line = "create-session --json";
    assert!(parse(&args(line)).is_err(), "--json is not a method flag");
}

#[test]
fn flags_take_a_separate_or_attached_value() {
    let line = format!("resize --block {ID} --cols=80 --rows 24");
    let [call] = calls(&line).try_into().expect("one call");
    assert_eq!(call.method.name, "resize");
    assert_eq!(
        call.args,
        [
            ("block", Arg::Value(Value::Id(ID.parse().unwrap()))),
            ("cols", Arg::Value(Value::Cells(80))),
            ("rows", Arg::Value(Value::Cells(24))),
        ]
    );
}

#[test]
fn plus_chains_requests() {
    let line =
        format!("attach --block {ID} + acquire-lease --block {ID} + input --block {ID} --data x");
    let names: Vec<&str> = calls(&line).iter().map(|c| c.method.name).collect();
    assert_eq!(names, ["attach", "acquire-lease", "input"]);
}

#[test]
fn bad_calls_fail_before_sending() {
    for (line, want) in [
        ("frobnicate", "unknown command `frobnicate`"),
        ("create-session", "create-session: missing parameter `name`"),
        ("create-session --nme x", "unknown parameter `nme`"),
        ("create-session --name", "--name needs a value"),
        ("create-session name", "expected a --flag"),
        (
            "resize --block x --cols 0 --rows 1",
            "`0` is not a valid cell count",
        ),
        ("close-block --block=", "`` is not a valid block id"),
        ("create-session --name a +", "empty request"),
        ("create-session --name a --name b", "given twice"),
    ] {
        let error = parse(&args(line)).expect_err(line);
        assert!(error.contains(want), "{line}: {error}");
    }
}

#[test]
fn events_flags() {
    let cursor = format!("{ID}:42");
    let parsed = parse(&args(&format!("events --follow --after {cursor}"))).unwrap();
    assert_eq!(
        parsed.command,
        Command::Events {
            after: Some(Cursor {
                epoch: ID.parse::<Uuid>().unwrap(),
                seq: 42
            }),
            follow: true,
        }
    );
    for bad in ["events --after 42", "events --after x:1", "events --tail"] {
        assert!(parse(&args(bad)).is_err(), "{bad}");
    }
}

#[test]
fn help_and_nothing_show_usage() {
    assert_eq!(parse(&[]).unwrap().command, Command::Help(None));
    let parsed = parse(&args("help input")).unwrap();
    assert_eq!(parsed.command, Command::Help(schema::method("input")));
    assert!(usage(None).contains("create-block"));
    assert!(usage(schema::method("input")).contains("--data <bytes>"));
}

#[test]
fn a_flag_alone_is_on() {
    let line = format!("terminal.format --block {ID} --trim --history 0 --vt=false --unwrap");
    let [call] = calls(&line).try_into().expect("one call");
    let flags: Vec<_> = call.args[1..].iter().collect();
    assert_eq!(
        flags,
        [
            &("trim", Arg::Value(Value::Flag(true))),
            &("history", Arg::Value(Value::Lines(0))),
            &("vt", Arg::Value(Value::Flag(false))),
            &("unwrap", Arg::Value(Value::Flag(true))),
        ]
    );
    let refused = parse(&args(&format!("terminal.format --block {ID} --history")));
    assert!(refused.is_err(), "only a flag stands alone");
    let help = usage(schema::method("terminal.format"));
    assert!(
        help.contains("--history <line count> [--trim] [--unwrap] [--vt]"),
        "{help}"
    );
}

#[test]
fn an_id_flag_takes_a_target() {
    let [call] = calls("terminal.format --block build/2 --history 0")
        .try_into()
        .expect("one call");
    assert!(call.has_target());
    assert_eq!(
        call.args[0],
        (
            "block",
            Arg::Target {
                entity: Entity::Block,
                spec: "build/2".into()
            }
        )
    );
    let [call] = calls(&format!("close-block --block {ID}"))
        .try_into()
        .expect("one call");
    assert!(!call.has_target());
}

#[test]
fn a_missing_id_flag_defaults_to_where_ctl_runs() {
    let here = Defaults {
        block: Some(ID.parse().unwrap()),
        ..Defaults::default()
    };
    let parsed = super::parse(&args("close-block"), &here).unwrap();
    let Command::Calls(calls) = parsed.command else {
        panic!("calls")
    };
    assert_eq!(
        calls[0].args,
        [("block", Arg::Value(Value::Id(ID.parse().unwrap())))]
    );
    // A given flag wins, and without a default the flag is still required.
    let Command::Calls(calls) = super::parse(&args("close-block --block 3"), &here)
        .unwrap()
        .command
    else {
        panic!("calls")
    };
    assert_eq!(
        calls[0].args,
        [(
            "block",
            Arg::Target {
                entity: Entity::Block,
                spec: "3".into()
            }
        )]
    );
    assert!(
        parse(&args("close-block"))
            .unwrap_err()
            .contains("missing parameter `block`")
    );
}

#[test]
fn uuid_shaped_text_is_an_id_never_a_name() {
    // A session named with a UUID is still reached only by that id.
    let line = format!("create-window --session {ID} --name w");
    let [call] = calls(&line).try_into().expect("one call");
    assert_eq!(
        call.args[0],
        ("session", Arg::Value(Value::Id(ID.parse().unwrap())))
    );
    assert!(!call.has_target());
}

fn wait_of(line: &str) -> Wait {
    match parse(&args(line)).expect("parses").command {
        Command::Wait(wait) => wait,
        other => panic!("expected wait, got {other:?}"),
    }
}

#[test]
fn wait_takes_a_block_or_an_event_filter_and_a_timeout() {
    assert_eq!(
        wait_of("wait --block build --timeout 1.5"),
        Wait {
            until: Until::Exit {
                block: "build".into()
            },
            timeout: Some(Duration::from_millis(1500)),
        }
    );
    // With --event, --block is one of the fields to match, in any order.
    assert_eq!(
        wait_of("wait --block build --event program-exited --code 0"),
        Wait {
            until: Until::Event {
                kind: "program-exited",
                fields: vec![("block", "build".into()), ("code", "0".into())],
            },
            timeout: None,
        }
    );
    // A window or a session alone waits for every block in it to end.
    for (line, scope, target) in [
        ("wait --window logs", Scope::Window, "logs"),
        ("wait --session main --timeout 2", Scope::Session, "main"),
    ] {
        assert_eq!(
            wait_of(line).until,
            Until::Ended {
                scope,
                target: target.into()
            },
            "{line}"
        );
    }
    for (line, want) in [
        ("wait", "usage: wait"),
        ("wait --block a --block b", "usage: wait"),
        ("wait --block a --window w", "usage: wait"),
        ("wait --window w --session s", "usage: wait"),
        ("wait --client c", "usage: wait"),
        ("wait --event nope", "unknown event `nope`"),
        ("wait --event bell --code 1", "has no field `code`"),
        ("wait --block a --timeout -1", "not a timeout"),
        ("wait --block a --timeout soon", "not a timeout"),
    ] {
        let err = parse(&args(line)).expect_err(line);
        assert!(err.contains(want), "{line}: {err}");
    }
}

fn run_of(line: &str, here: &Defaults) -> Run {
    match super::parse(&args(line), here).expect("parses").command {
        Command::Run(run) => run,
        other => panic!("expected run, got {other:?}"),
    }
}

fn arg_of<'a>(call: &'a Call, name: &str) -> Option<&'a Arg> {
    call.args.iter().find(|(n, _)| *n == name).map(|(_, a)| a)
}

#[test]
fn run_builds_a_create_block_request_in_the_current_directory() {
    fn text(s: &str) -> Option<Arg> {
        Some(Arg::Value(Value::Text(s.into())))
    }
    // Absolute on every platform.
    let work = std::env::temp_dir().join("work");
    let here = Defaults {
        window: Some(ID.parse().unwrap()),
        dir: Some(work.clone()),
        ..Defaults::default()
    };
    let native = |path: PathBuf| text(path.to_str().expect("UTF-8"));
    let run = run_of("run --wait -- make -j 4 --keep-going", &here);
    assert_eq!(run.wait, Some(None));
    assert_eq!(arg_of(&run.create, "command").cloned(), text("make"));
    assert_eq!(
        arg_of(&run.create, "arg").cloned(),
        Some(Arg::Value(Value::Words(
            ["-j", "4", "--keep-going"].map(String::from).to_vec()
        )))
    );
    assert_eq!(arg_of(&run.create, "cwd").cloned(), native(work.clone()));
    assert_eq!(
        arg_of(&run.create, "cols").cloned(),
        Some(Arg::Value(Value::Cells(80)))
    );
    assert_eq!(
        arg_of(&run.create, "window").cloned(),
        Some(Arg::Value(Value::Id(ID.parse().unwrap())))
    );

    // Flags end at the first argument that is not one; a relative --cwd is
    // taken from the current directory.
    let run = run_of(
        "run --window w --cols 100 --cwd sub --wait --timeout 2 true",
        &here,
    );
    assert_eq!(run.wait, Some(Some(Duration::from_secs(2))));
    assert_eq!(
        arg_of(&run.create, "cwd").cloned(),
        native(work.join("sub"))
    );
    assert_eq!(arg_of(&run.create, "arg"), None);
    assert!(run.create.has_target());
    // An absolute --cwd is kept as given.
    let elsewhere = std::env::temp_dir().join("elsewhere");
    let line = ["run", "--cwd", elsewhere.to_str().expect("UTF-8"), "true"].map(String::from);
    let Command::Run(run) = super::parse(&line, &here).expect("parses").command else {
        panic!("run");
    };
    assert_eq!(arg_of(&run.create, "cwd").cloned(), native(elsewhere));

    let nowhere = Defaults::default();
    assert_eq!(
        arg_of(&run_of("run --window w true", &nowhere).create, "cwd"),
        None
    );
    for (line, want) in [
        ("run", "usage: run"),
        ("run --wait --", "usage: run"),
        ("run --timeout 1 true", "--timeout needs --wait"),
        ("run --colour red true", "unknown flag `--colour`"),
        ("run --cols", "--cols needs a value"),
        ("run --cols 0 true", "not a valid cell count"),
        ("run --window w --cwd rel true", "is relative"),
    ] {
        let err = super::parse(&args(line), &nowhere).expect_err(line);
        assert!(err.contains(want), "{line}: {err}");
    }
}

#[test]
fn a_repeated_list_flag_gathers_its_values() {
    let line =
        format!("create-block --window {ID} --cols 80 --rows 24 --command sh --arg -c --arg x");
    let [call] = calls(&line).try_into().expect("one call");
    assert_eq!(
        arg_of(&call, "arg").cloned(),
        Some(Arg::Value(Value::Words(vec!["-c".into(), "x".into()])))
    );
    let line = format!("create-block --window {ID} --cols 80 --rows 24 --arg x");
    let err = parse(&args(&line)).expect_err("arguments without a command");
    assert!(err.contains("need a `command`"), "{err}");
}

#[test]
fn agent_verbs_take_an_optional_dir() {
    let agent = |line: &str| parse(&args(line)).map(|i| i.command);
    assert_eq!(
        agent("agent setup"),
        Ok(Command::Agent(AgentCommand::Setup(None)))
    );
    assert_eq!(
        agent("agent remove --dir .claude"),
        Ok(Command::Agent(AgentCommand::Remove(Some(".claude".into()))))
    );
    assert_eq!(
        agent("agent status --dir=x"),
        Ok(Command::Agent(AgentCommand::Status(Some("x".into()))))
    );
    assert_eq!(agent("agent hook"), Ok(Command::Agent(AgentCommand::Hook)));
    for bad in [
        "agent",
        "agent install",
        "agent hook --dir x",
        "agent setup --to x",
        "agent setup x",
    ] {
        assert!(agent(bad).is_err(), "{bad}");
    }
}

#[test]
fn bridge_takes_only_the_socket() {
    let parsed = parse(&args("--socket /tmp/s bridge")).unwrap();
    assert_eq!(parsed.command, Command::Bridge);
    assert_eq!(parsed.socket, Some("/tmp/s".into()));
    assert!(parse(&args("bridge now")).is_err());
}
