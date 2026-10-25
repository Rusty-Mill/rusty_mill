use rusty_uuid::Uuid;

use super::*;

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
            ("block", Value::Id(ID.parse().unwrap())),
            ("cols", Value::Cells(80)),
            ("rows", Value::Cells(24)),
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
            "resize --block x --cols 1 --rows 1",
            "`x` is not a valid block id",
        ),
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
