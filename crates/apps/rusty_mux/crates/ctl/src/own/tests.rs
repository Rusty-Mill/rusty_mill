use super::*;

const WINDOW: &str = "00000000-0000-0000-0000-00000000000a";

fn parsed(line: &str) -> Request {
    parse(line).expect("parses").1
}

fn refusal(line: &str) -> (Value, String) {
    parse(line).expect_err("refused")
}

#[test]
fn a_bare_launch_owns_the_default_program_at_the_default_size() {
    let line = format!(r#"{{"id":1,"op":"launch","window":"{WINDOW}"}}"#);
    let Request::Launch {
        window,
        size,
        launch,
    } = parsed(&line)
    else {
        panic!("not a launch");
    };
    assert_eq!(window.uuid().to_string(), WINDOW);
    assert_eq!((size.cols(), size.rows()), DEFAULT_SIZE);
    assert_eq!(
        launch,
        Launch {
            own: true,
            ..Launch::default()
        }
    );
}

#[test]
fn a_launch_carries_its_program_size_and_directory() {
    let line = format!(
        r#"{{"id":"a","op":"launch","window":"{WINDOW}","cols":100,"rows":30,
            "program":"lel-jail","args":["--slot","3"],"cwd":"/srv"}}"#
    )
    .replace('\n', "");
    let (id, request) = parse(&line).expect("parses");
    assert_eq!(id, Value::from("a"));
    let Request::Launch { size, launch, .. } = request else {
        panic!("not a launch");
    };
    assert_eq!((size.cols(), size.rows()), (100, 30));
    assert_eq!(
        launch.program,
        Some(Program {
            command: "lel-jail".into(),
            args: vec!["--slot".into(), "3".into()],
        })
    );
    assert_eq!(launch.cwd.as_deref(), Some("/srv"));
    assert!(launch.own);
}

#[test]
fn a_close_names_its_block() {
    let line = format!(r#"{{"id":2,"op":"close","block":"{WINDOW}"}}"#);
    assert!(matches!(parsed(&line), Request::Close { .. }));
}

#[test]
fn bad_requests_are_refused_with_their_id() {
    let cases = [
        (r#"{"id":3,"op":"fly"}"#, "unknown op"),
        (r#"{"id":3}"#, "no `op`"),
        (r#"{"id":3,"op":"close","block":"nope"}"#, "not an id"),
        (r#"{"id":3,"op":"close"}"#, "must be an id"),
    ];
    for (line, why) in cases {
        let (id, reason) = refusal(line);
        assert_eq!(id, Value::from(3u64), "{line}");
        assert!(reason.contains(why), "{line}: {reason}");
    }
    let launch = |extra: &str| format!(r#"{{"id":3,"op":"launch","window":"{WINDOW}"{extra}}}"#);
    for (extra, why) in [
        (r#","args":["x"]"#, "need a `program`"),
        (r#","cols":0"#, "zero"),
        (r#","rows":70000"#, "number of cells"),
        (r#","program":7"#, "must be text"),
        (r#","program":"sh","args":[1]"#, "list of text"),
        (r#","input":"shut""#, "`open` or `owner-only`"),
    ] {
        let (_, reason) = refusal(&launch(extra));
        assert!(reason.contains(why), "{extra}: {reason}");
    }
}

#[test]
fn an_agent_launch_carries_its_environment_and_descriptor_count() {
    let line = format!(
        r#"{{"id":1,"op":"launch","window":"{WINDOW}","env":{{"PATH":"/bin","EMPTY":""}},"fds":2}}"#
    );
    let request = parsed(&line);
    let Request::Launch { launch, .. } = &request else {
        panic!("not a launch");
    };
    let mut env = launch.env.clone().expect("an exact environment");
    env.sort();
    assert_eq!(
        env,
        [
            ("EMPTY".into(), String::new()),
            ("PATH".into(), "/bin".into())
        ]
    );
    assert_eq!(launch.fds, 2);
    assert!(request.agent());
    let plain = format!(r#"{{"id":1,"op":"launch","window":"{WINDOW}"}}"#);
    assert!(!parsed(&plain).agent());
}

#[test]
fn bad_agent_launches_are_refused() {
    let launch = |extra: &str| format!(r#"{{"id":3,"op":"launch","window":"{WINDOW}"{extra}}}"#);
    for (extra, why) in [
        (r#","env":["A"]"#, "map names to text"),
        (r#","env":{"A":1}"#, "must be text"),
        (r#","fds":9"#, "from 0 to 8"),
        (r#","fds":-1"#, "from 0 to 8"),
    ] {
        let (_, reason) = refusal(&launch(extra));
        assert!(reason.contains(why), "{extra}: {reason}");
    }
}

#[test]
fn an_owner_only_launch_needs_the_input_gate() {
    let line = format!(r#"{{"id":1,"op":"launch","window":"{WINDOW}","input":"owner-only"}}"#);
    let request = parsed(&line);
    let Request::Launch { launch, .. } = &request else {
        panic!("not a launch");
    };
    assert_eq!(launch.input, InputMode::OwnerOnly);
    assert!(request.gated());
    let open = format!(r#"{{"id":1,"op":"launch","window":"{WINDOW}","input":"open"}}"#);
    assert!(!parsed(&open).gated());
}

#[test]
fn grants_and_revokes_name_a_block_a_client_and_a_time() {
    let line = format!(
        r#"{{"id":4,"op":"grant","block":"{WINDOW}","client":"{WINDOW}","until":1700000000000}}"#
    );
    let request = parsed(&line);
    assert!(matches!(
        request,
        Request::Grant {
            until: 1_700_000_000_000,
            ..
        }
    ));
    assert!(request.gated());
    let line = format!(r#"{{"id":5,"op":"revoke","block":"{WINDOW}","client":"{WINDOW}"}}"#);
    assert!(matches!(parsed(&line), Request::Revoke { .. }));
    let line = format!(r#"{{"id":6,"op":"grant","block":"{WINDOW}","client":"{WINDOW}"}}"#);
    assert!(refusal(&line).1.contains("unix ms"));
    let line = format!(r#"{{"id":7,"op":"revoke","block":"{WINDOW}"}}"#);
    assert!(refusal(&line).1.contains("`client`"));
}

#[test]
fn input_is_printed_as_hex() {
    assert_eq!(hex(b"ls\r\x00\xff"), "6c730d00ff");
    assert_eq!(hex(&[]), "");
}

#[test]
fn a_line_that_is_not_json_is_refused_without_an_id() {
    let (id, reason) = refusal("launch please");
    assert_eq!(id, Value::Null);
    assert!(reason.starts_with("not JSON"), "{reason}");
}

#[test]
fn answers_and_events_are_single_json_lines() {
    assert_eq!(
        answered(Value::from(1u64), Some(("block", Value::from("b")))),
        r#"{"block":"b","id":1,"ok":true}"#
    );
    assert_eq!(
        failed(Value::Null, "no"),
        r#"{"error":"no","id":null,"ok":false}"#
    );
    assert_eq!(event("lost", []), r#"{"event":"lost"}"#);
}
