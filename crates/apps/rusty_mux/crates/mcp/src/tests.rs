use rusty_mux_proto::schema::method;

use super::*;

fn known(name: &str) -> &'static Method {
    method(name).expect(name)
}

fn args(text: &str) -> Value {
    Value::from_json_str(text).expect("valid JSON")
}

#[test]
fn every_catalog_method_has_a_shape_and_most_are_tools() {
    let tools: Vec<&str> = CATALOG
        .iter()
        .filter(|m| shape(m) != Shape::Connection)
        .map(|m| m.name)
        .collect();
    for name in ["attach", "detach", "acquire-lease", "release-lease", "sync"] {
        assert!(!tools.contains(&name), "{name}");
    }
    for name in [
        "create-session",
        "input",
        "terminal.format",
        "terminal.status",
        "list",
    ] {
        assert!(tools.contains(&name), "{name}");
    }
    assert_eq!(tools.len(), CATALOG.len() - 5);
}

#[test]
fn plain_calls_become_one_request() {
    let call = args(r#"{"block": "build", "history": 30}"#);
    assert_eq!(
        argv(known("terminal.format"), Some(&call)),
        Ok(vec![
            "terminal.format".into(),
            "--block=build".into(),
            "--history=30".into()
        ])
    );
    assert_eq!(argv(known("list"), None), Ok(vec!["list".into()]));
}

#[test]
fn a_list_argument_repeats_its_flag() {
    let call = args(r#"{"arg": ["-c", "exit 3"]}"#);
    assert_eq!(
        argv(known("create-block"), Some(&call)),
        Ok(vec![
            "create-block".into(),
            "--arg=-c".into(),
            "--arg=exit 3".into()
        ])
    );
    let refused = argv(known("create-block"), Some(&args(r#"{"arg": [1]}"#)));
    assert!(refused.expect_err("a number").contains("list of strings"));
}

#[test]
fn leased_calls_attach_and_take_the_lease_first() {
    let call = args(r#"{"block": "2", "data": "a + b\r"}"#);
    assert_eq!(
        argv(known("input"), Some(&call)),
        Ok(vec![
            "attach".into(),
            "--block=2".into(),
            "+".into(),
            "acquire-lease".into(),
            "--block=2".into(),
            "+".into(),
            "input".into(),
            "--block=2".into(),
            "--data=a + b\r".into(),
        ])
    );
    let defaulted = argv(known("input"), Some(&args(r#"{"data": "x"}"#)));
    assert_eq!(
        defaulted.map(|a| a.join(" ")),
        Ok("attach + acquire-lease + input --data=x".into())
    );
}

#[test]
fn arguments_are_checked_before_anything_runs() {
    let cases = [
        (
            "terminal.title",
            r#"{"socket": "/tmp/x"}"#,
            "unknown parameter `socket`",
        ),
        (
            "terminal.format",
            r#"{"history": -1}"#,
            "non-negative integer",
        ),
        (
            "terminal.format",
            r#"{"history": [1]}"#,
            "non-negative integer",
        ),
        ("list", r#"[]"#, "must be an object"),
    ];
    for (name, call, reason) in cases {
        let refused = argv(known(name), Some(&args(call))).expect_err(call);
        assert!(refused.contains(reason), "{call}: {refused}");
    }
}

#[test]
fn tools_carry_the_catalog_schema_and_hints() {
    let format = tool(known("terminal.format"));
    assert_eq!(
        format.input_schema,
        rusty_mux_ctl::input_schema(known("terminal.format"))
    );
    let hint = |t: &Tool, k: &str| t.annotations.as_ref().and_then(|a| a.get(k)).cloned();
    assert_eq!(hint(&format, "readOnlyHint"), Some(Value::Bool(true)));
    let close = tool(known("close-block"));
    assert_eq!(hint(&close, "destructiveHint"), Some(Value::Bool(true)));
    assert_eq!(hint(&close, "readOnlyHint"), Some(Value::Bool(false)));
    assert_eq!(hint(&close, "openWorldHint"), Some(Value::Bool(false)));
    let input = tool(known("input"));
    assert_eq!(hint(&input, "destructiveHint"), Some(Value::Bool(true)));
    assert_eq!(hint(&input, "openWorldHint"), Some(Value::Bool(true)));
    assert_eq!(hint(&input, "readOnlyHint"), Some(Value::Bool(false)));
}

#[test]
fn output_keeps_trailing_whitespace_and_drops_only_the_last_newline() {
    assert_eq!(printed(b"/tmp/dir \n"), "/tmp/dir ");
    assert_eq!(printed(b"row 1\n\n  \n"), "row 1\n\n  ");
    assert_eq!(printed(b"no newline"), "no newline");
    assert_eq!(printed(b"\n"), "");
}
