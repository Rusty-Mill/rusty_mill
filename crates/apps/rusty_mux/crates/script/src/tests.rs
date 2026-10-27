use super::*;

const SCRATCH: &str = r#"{
  "actions": [
    {
      "name": "scratch",
      "title": "Open a scratch window",
      "args": ["name"],
      "run": [
        {"call": "create-window", "args": {"name": "${name}"}},
        {"call": "create-block", "args": {"window": "${name}", "cols": 80, "rows": 24}},
        {"call": "attach"},
        {"call": "input", "args": {"data": "cd ${HOME} && echo ${name}\r"}}
      ]
    },
    {"name": "new-session", "args": ["name"], "run": [{"call": "create-session", "args": {"name": "${name}"}}]}
  ],
  "binds": [
    {"keys": "ctrl+b>c", "action": "scratch", "args": {"name": "new"}},
    {"keys": "cmd+shift+l", "action": "list"}
  ]
}"#;

fn problems(text: &str) -> Vec<String> {
    parse(text)
        .expect_err("invalid")
        .iter()
        .map(Problem::to_string)
        .collect()
}

#[test]
fn a_valid_config_keeps_its_actions_and_binds_in_order() {
    let config = parse(SCRATCH).expect("valid");
    let names: Vec<&str> = config.actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["scratch", "new-session"]);
    let scratch = config.action("scratch").expect("defined");
    assert_eq!(scratch.title.as_deref(), Some("Open a scratch window"));
    assert_eq!(scratch.args, ["name"]);
    assert_eq!(
        scratch.run[2],
        Call {
            method: "attach",
            params: Vec::new()
        }
    );
    assert_eq!(config.binds[0].keys, "ctrl+b>c");
    assert_eq!(config.binds[1].action, "list");
    assert_eq!(parse("{}"), Ok(Config::default()));
}

#[test]
fn expanding_fills_declared_placeholders_only() {
    let config = parse(SCRATCH).expect("valid");
    let given = [("name".to_owned(), "logs".to_owned())];
    let calls = config
        .action("scratch")
        .expect("defined")
        .expand(&given)
        .expect("expands");
    assert_eq!(calls[0].params, [("name", "logs".to_owned())]);
    let mut block = calls[1].params.clone();
    block.sort();
    assert_eq!(
        block,
        [
            ("cols", "80".to_owned()),
            ("rows", "24".to_owned()),
            ("window", "logs".to_owned())
        ]
    );
    assert_eq!(
        calls[3].params,
        [("data", "cd ${HOME} && echo logs\r".to_owned())]
    );
}

#[test]
fn expanding_needs_exactly_the_declared_arguments() {
    let config = parse(SCRATCH).expect("valid");
    let scratch = config.action("scratch").expect("defined");
    assert_eq!(
        scratch.expand(&[]),
        Err("scratch: missing argument `name`".into())
    );
    let extra = [("name".into(), "a".into()), ("size".into(), "b".into())];
    assert_eq!(
        scratch.expand(&extra),
        Err("scratch: unknown argument `size`".into())
    );
}

#[test]
fn a_value_is_never_expanded_again() {
    assert_eq!(
        substitute(
            "${a}-${b}",
            &[("a".into(), "${b}".into()), ("b".into(), "x".into())]
        ),
        "${b}-x"
    );
    assert_eq!(substitute("${a", &[("a".into(), "x".into())]), "${a");
    assert_eq!(substitute("$${a}}", &[("a".into(), "x".into())]), "$x}");
}

#[test]
fn every_problem_is_reported_with_where_it_is() {
    let text = r#"{
      "actions": [
        {"name": "Bad Name", "run": [{"call": "nope"}]},
        {"name": "list", "run": []},
        {"name": "ok", "args": ["x", "x", "Y"], "run": [
          {"call": "terminal.format", "args": {"block": "${y}", "lines": 3, "history": -1}}
        ]},
        {"name": "ok", "run": [{"call": "list", "extra": 1}]},
        {"name": "norun"}
      ],
      "binds": [
        {"keys": "ctrl+b>c", "action": "ok", "args": {"z": "1"}},
        {"keys": "ctrl+b>c", "action": "missing"},
        {"keys": "ctrl b", "action": "list", "args": {"block": "1"}}
      ],
      "colour": "red"
    }"#;
    let found = problems(text);
    let expected = [
        "config: unknown key `colour`",
        "actions[0]: `Bad Name` is not an action name (lowercase letters, digits, `.`, `_`, `-`)",
        "actions[0].run[0]: unknown method `nope`",
        "actions[1]: `list` is a catalog method",
        "actions[1].run: needs at least one request",
        "actions[2].args[1]: argument `x` is declared twice",
        "actions[2].args[2]: `Y` is not an argument name",
        "actions[2].run[0].args.history: expected a string, a non-negative integer or a boolean",
        "actions[2].run[0].args.block: `${y}` is not an argument of this action",
        "actions[2].run[0].args.lines: `terminal.format` has no parameter `lines`",
        "actions[2].run[0]: `terminal.format` needs parameter `history`",
        "actions[3].run[0]: unknown key `extra`",
        "actions[3]: action `ok` is defined twice",
        "actions[4]: missing `run`",
        "binds[0]: `ok` needs argument `x`",
        "binds[0]: `ok` has no argument `z`",
        "binds[1]: unknown action `missing`",
        "binds[1]: `ctrl+b>c` is bound twice",
        "binds[2]: `ctrl b` is not a key sequence such as `ctrl+b>c`",
        "binds[2]: `list` has no parameter `block`",
    ];
    for problem in expected {
        assert!(
            found.iter().any(|f| f == problem),
            "missing {problem:?} in {found:#?}"
        );
    }
    assert_eq!(found.len(), expected.len(), "{found:#?}");
}

#[test]
fn a_bound_action_needs_all_its_arguments() {
    let text = r#"{
      "actions": [{"name": "go", "args": ["to"], "run": [{"call": "list"}]}],
      "binds": [{"keys": "g", "action": "go"}]
    }"#;
    assert_eq!(problems(text), ["binds[0]: `go` needs argument `to`"]);
}

#[test]
fn malformed_json_and_shapes_are_problems() {
    assert_eq!(problems("{")[0].split(':').next(), Some("config"));
    assert_eq!(problems("[]"), ["config: expected an object"]);
    assert_eq!(
        problems(r#"{"actions": {}}"#),
        ["actions: expected an array"]
    );
    assert_eq!(
        problems(r#"{"actions": [{"run": [{"call": "list"}]}]}"#),
        ["actions[0].name: missing"]
    );
}

#[test]
fn key_sequences_and_names_have_a_grammar() {
    for ok in ["ctrl+b>c", "cmd+d", "f1", "ctrl+b>shift+%"] {
        assert!(parse::is_key_sequence_for_tests(ok), "{ok}");
    }
    for bad in ["", "ctrl+", "a>>b", "ctrl b", "+a"] {
        assert!(!parse::is_key_sequence_for_tests(bad), "{bad:?}");
    }
    assert!(is_arg_name("block-id") && is_arg_name("n2") && !is_arg_name("2n") && !is_arg_name(""));
}

#[test]
fn values_must_suit_the_catalog_as_ctl_would_check_them() {
    let text = r#"{
      "actions": [
        {"name": "bad", "run": [
          {"call": "create-session"},
          {"call": "input"},
          {"call": "resize", "args": {"cols": 0, "rows": "wide"}},
          {"call": "resize", "args": {"cols": 65536, "rows": 24}}
        ]},
        {"name": "size", "args": ["cols"], "run": [
          {"call": "resize", "args": {"block": "1/2/3", "cols": "${cols}", "rows": 24}},
          {"call": "create-window", "args": {"name": "${HOME}"}}
        ]}
      ],
      "binds": [
        {"keys": "a", "action": "size", "args": {"cols": "0"}},
        {"keys": "b", "action": "size", "args": {"cols": "120"}},
        {"keys": "c", "action": "resize", "args": {"cols": "80", "rows": "x"}},
        {"keys": "d", "action": "create-session"},
        {"keys": "e", "action": "create-window", "args": {"name": "w"}}
      ]
    }"#;
    assert_eq!(
        problems(text),
        [
            "actions[0].run[0]: `create-session` needs parameter `name`",
            "actions[0].run[1]: `input` needs parameter `data`",
            "actions[0].run[2].args.cols: `cols`: `0` is not a valid cell count",
            "actions[0].run[2].args.rows: `rows`: `wide` is not a valid cell count",
            "actions[0].run[3].args.cols: `cols`: `65536` is not a valid cell count",
            "binds[0]: `cols`: `0` is not a valid cell count",
            "binds[2]: `rows`: `x` is not a valid cell count",
            "binds[3]: `create-session` needs parameter `name`",
        ]
    );
}

const RULES: &str = r#"{
  "actions": [
    {"name": "note", "args": ["dir", "where"], "run": [
      {"call": "input", "args": {"block": "${where}", "data": "echo ${dir}\r"}}
    ]},
    {"name": "fit", "args": ["cols"], "run": [
      {"call": "resize", "args": {"cols": "${cols}", "rows": 24}}
    ]}
  ],
  "on": [
    {"event": "pwd-changed", "action": "note", "args": {"dir": "${pwd}", "where": "${block}"}},
    {"event": "block-created", "action": "fit", "args": {"cols": "${cols}"}},
    {"event": "block-closed", "action": "terminal.title", "args": {"block": "${block}"}}
  ]
}"#;

#[test]
fn rules_fill_event_fields_then_expand() {
    let config = parse(RULES).expect("valid");
    assert_eq!(config.rules.len(), 3);
    assert_eq!(config.rules[0].event, "pwd-changed");
    let fields = [
        ("block".to_owned(), "b1".to_owned()),
        ("pwd".to_owned(), "/tmp/${where}".to_owned()),
    ];
    let calls = config.rules[0].calls(&config, &fields).expect("expands");
    let mut params = calls[0].params.clone();
    params.sort();
    assert_eq!(
        params,
        [
            ("block", "b1".to_owned()),
            ("data", "echo /tmp/${where}\r".to_owned())
        ],
        "a field's value is never expanded again"
    );
    let calls = config.rules[2]
        .calls(&config, &[("block".into(), "b2".into())])
        .expect("a request");
    assert_eq!(
        calls,
        [Call {
            method: "terminal.title",
            params: vec![("block", "b2".to_owned())]
        }]
    );
}

#[test]
fn rules_are_checked_against_events_and_their_fields() {
    let text = r#"{
      "actions": [{"name": "fit", "args": ["cols"], "run": [
        {"call": "resize", "args": {"cols": "${cols}", "rows": 24}}
      ]}],
      "on": [
        {"event": "pwd_changed", "action": "fit", "args": {"cols": "80"}},
        {"event": "block-closed", "action": "fit", "args": {"cols": "${cols}"}},
        {"event": "block-created", "action": "fit", "args": {"cols": "0"}},
        {"event": "block-created", "action": "fit"},
        {"event": "pwd-changed", "action": "create-session", "args": {"name": "${pwd}", "x": "1"}},
        {"event": "pwd-changed", "action": "resize", "args": {"cols": "wide", "rows": "${block}"}},
        {"event": "attached", "action": "nope"},
        {"action": "fit", "args": {"cols": "1"}}
      ]
    }"#;
    assert_eq!(
        problems(text),
        [
            "on[0].event: unknown event `pwd_changed`",
            "on[1].args.cols: `${cols}` is not a field of `block-closed`",
            "on[2]: `cols`: `0` is not a valid cell count",
            "on[3]: `fit` needs argument `cols`",
            "on[4]: `create-session` has no parameter `x`",
            "on[5]: `cols`: `wide` is not a valid cell count",
            "on[6]: unknown action `nope`",
            "on[7].event: missing",
        ]
    );
}
