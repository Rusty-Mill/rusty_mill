use std::cell::RefCell;

use super::*;

/// A workspace that answers from memory and records what it was asked.
#[derive(Default)]
struct Fake {
    exec_seen: RefCell<Option<ExecSpec>>,
    written: RefCell<Option<(SessionPath, Vec<u8>)>>,
}

impl Workspace for Fake {
    fn read(&self, path: &SessionPath, cap: usize) -> Result<(Vec<u8>, bool), String> {
        match path.as_path().to_str() {
            Some("a.txt") => Ok((b"hello".to_vec(), false)),
            Some("bin") => Ok((vec![0xff, b'x'], false)),
            Some("big") => Ok((vec![b'x'; cap], true)),
            _ => Err("no such file".to_owned()),
        }
    }

    fn list(&self, _path: &SessionPath) -> Result<Vec<Entry>, String> {
        Ok(vec![Entry {
            name: "src".to_owned(),
            kind: Kind::Dir,
            size: 0,
        }])
    }

    fn stat(&self, path: &SessionPath) -> Result<Entry, String> {
        Ok(Entry {
            name: path.as_path().display().to_string(),
            kind: Kind::File,
            size: 5,
        })
    }

    fn write(&self, path: &SessionPath, bytes: &[u8]) -> Result<(), String> {
        *self.written.borrow_mut() = Some((path.clone(), bytes.to_vec()));
        Ok(())
    }

    fn exec(&self, spec: &ExecSpec) -> Result<ExecOutcome, String> {
        *self.exec_seen.borrow_mut() = Some(spec.clone());
        Ok(ExecOutcome {
            status: Some(0),
            timed_out: false,
            stdout: b"ok\n".to_vec(),
            stderr: Vec::new(),
            truncated: false,
        })
    }
}

fn args(json: &str) -> Value {
    Value::parse(json).expect("test JSON")
}

fn call(ws: &Fake, role: Role, name: &str, json: &str) -> Answer {
    handle(ws, role, name, Some(&args(json)))
}

fn error_text(answer: &Answer) -> &str {
    assert!(answer.is_error, "expected an error: {:?}", answer.body);
    answer
        .body
        .get("error")
        .and_then(Value::as_str)
        .expect("error text")
}

#[test]
fn read_returns_text_and_flags() {
    let ws = Fake::default();
    let a = call(&ws, Role::Critic, "read", r#"{"path":"a.txt"}"#);
    assert!(!a.is_error);
    assert_eq!(a.body.get("content").and_then(Value::as_str), Some("hello"));
    assert_eq!(
        a.body.get("truncated").and_then(Value::as_bool),
        Some(false)
    );
    assert!(a.body.get("content_lossy").is_none());

    let big = call(&ws, Role::Builder, "read", r#"{"path":"big"}"#);
    assert_eq!(
        big.body.get("truncated").and_then(Value::as_bool),
        Some(true)
    );

    let bin = call(&ws, Role::Builder, "read", r#"{"path":"bin"}"#);
    assert_eq!(
        bin.body.get("content_lossy").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn escapes_are_tool_errors_before_the_workspace_is_asked() {
    let ws = Fake::default();
    for path in ["/etc/passwd", "../x", "a/../../b", "/proc/self/environ"] {
        let json = format!(r#"{{"path":"{path}","content":"x"}}"#);
        for tool in ["read", "stat", "write", "list"] {
            let a = call(&ws, Role::Builder, tool, &json);
            assert!(error_text(&a).contains("refused"), "{tool} {path}");
        }
    }
    assert!(ws.written.borrow().is_none());
    let a = call(
        &ws,
        Role::Builder,
        "exec",
        r#"{"argv":["ls"],"cwd":"../.."}"#,
    );
    assert!(error_text(&a).contains("refused"));
    assert!(ws.exec_seen.borrow().is_none());
}

#[test]
fn workspace_failures_are_tool_errors() {
    let ws = Fake::default();
    let a = call(&ws, Role::Builder, "read", r#"{"path":"missing"}"#);
    assert_eq!(error_text(&a), "no such file");
}

#[test]
fn critic_cannot_write_and_unknown_tools_are_refused() {
    let ws = Fake::default();
    let a = call(&ws, Role::Critic, "write", r#"{"path":"a","content":"x"}"#);
    assert!(error_text(&a).contains("not offered"));
    assert!(ws.written.borrow().is_none());
    let a = call(&ws, Role::Builder, "rm", "{}");
    assert!(error_text(&a).contains("not offered"));
    assert!(!tools(Role::Critic).contains(&"write"));
}

#[test]
fn write_checks_its_arguments() {
    let ws = Fake::default();
    let a = call(&ws, Role::Builder, "write", r#"{"path":"","content":"x"}"#);
    assert!(error_text(&a).contains("name a file"));
    let a = call(&ws, Role::Builder, "write", r#"{"path":"a"}"#);
    assert!(error_text(&a).contains("`content`"));

    let over = "x".repeat(OUTPUT_CAP + 1);
    let json = format!(r#"{{"path":"a","content":"{over}"}}"#);
    let a = call(&ws, Role::Builder, "write", &json);
    assert!(error_text(&a).contains("over"));
    assert!(ws.written.borrow().is_none());

    let a = call(
        &ws,
        Role::Builder,
        "write",
        r#"{"path":"src/x.rs","content":"hi"}"#,
    );
    assert_eq!(a.body.get("written").and_then(Value::as_u64), Some(2));
    let (path, bytes) = ws.written.borrow().clone().expect("written");
    assert_eq!(path, SessionPath::parse("src/x.rs").expect("path"));
    assert_eq!(bytes, b"hi");
}

#[test]
fn list_and_stat_shape_entries() {
    let ws = Fake::default();
    let a = handle(&ws, Role::Critic, "list", None);
    let entries = a
        .body
        .get("entries")
        .and_then(Value::as_array)
        .expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].get("kind").and_then(Value::as_str), Some("dir"));

    let a = call(&ws, Role::Critic, "stat", r#"{"path":"a.txt"}"#);
    assert_eq!(a.body.get("size").and_then(Value::as_u64), Some(5));
    assert_eq!(a.body.get("kind").and_then(Value::as_str), Some("file"));
}

#[test]
fn exec_defaults_and_caps() {
    let ws = Fake::default();
    let a = call(&ws, Role::Critic, "exec", r#"{"argv":["cargo","test"]}"#);
    assert_eq!(a.body.get("status").and_then(Value::as_i64), Some(0));
    assert_eq!(a.body.get("stdout").and_then(Value::as_str), Some("ok\n"));
    let spec = ws.exec_seen.borrow().clone().expect("spec");
    assert_eq!(spec.timeout, DEFAULT_TIMEOUT);
    assert!(spec.cwd.is_root());
    assert!(spec.stdin.is_empty());

    call(
        &ws,
        Role::Builder,
        "exec",
        r#"{"argv":["cat"],"timeout_s":99999,"cwd":"src","stdin":"in"}"#,
    );
    let spec = ws.exec_seen.borrow().clone().expect("spec");
    assert_eq!(spec.timeout, MAX_TIMEOUT);
    assert_eq!(spec.cwd, SessionPath::parse("src").expect("path"));
    assert_eq!(spec.stdin, b"in");
}

#[test]
fn exec_refuses_bad_arguments() {
    let ws = Fake::default();
    for (json, want) in [
        ("{}", "`argv`"),
        (r#"{"argv":[]}"#, "name a program"),
        (r#"{"argv":[""]}"#, "name a program"),
        (r#"{"argv":["ls",1]}"#, "list of strings"),
        (r#"{"argv":["ls"],"timeout_s":0}"#, "positive"),
        (r#"{"argv":["ls"],"timeout_s":"5"}"#, "positive"),
        (r#"{"argv":["ls"],"stdin":3}"#, "`stdin`"),
    ] {
        let a = call(&ws, Role::Builder, "exec", json);
        assert!(error_text(&a).contains(want), "{json}");
    }
    assert!(ws.exec_seen.borrow().is_none());
}
