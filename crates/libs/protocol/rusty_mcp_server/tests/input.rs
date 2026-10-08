#![allow(clippy::unwrap_used)]
//! Multi-round-trip input (2026-07-28) through the stdio loop: a tool asks,
//! the client retries with its answer, and sealed `requestState` keeps the
//! rounds honest.

mod support;

use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ContentBlock, ElicitAction, ElicitParams, ErrorData, Tool,
};
use rusty_mcp_server::{answer, Ask, BuildError, CallContext, Server, ToolOutcome};
use std::sync::Arc;
use support::{code, reply, result, run_with};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28""#;
const ELICIT: &str = r#""io.modelcontextprotocol/clientCapabilities":{"elicitation":{}}"#;
const NO_CAPS: &str = r#""io.modelcontextprotocol/clientCapabilities":{}"#;

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn text(s: &str) -> ToolOutcome {
    ToolOutcome::Done(CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    })
}

#[cfg(feature = "request-state")]
fn kept(ctx: &CallContext) -> String {
    ctx.request_state()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default()
}

#[cfg(not(feature = "request-state"))]
fn kept(_: &CallContext) -> String {
    "-".to_owned()
}

fn book(ctx: &CallContext, call: CallToolParams) -> Result<ToolOutcome, ErrorData> {
    let Some(reply) = answer(&call, "confirm")? else {
        let form = ElicitParams::Form {
            message: "Book it?".to_owned(),
            requested_schema: schema(),
            meta: None,
        };
        let ask = Ask::new().elicit("confirm", &form);
        #[cfg(feature = "request-state")]
        let ask = ask.with_state("draft:42");
        return Ok(ToolOutcome::Ask(ask));
    };
    Ok(match reply.action {
        ElicitAction::Accept => {
            let who = reply
                .content
                .as_ref()
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            text(&format!("booked {} for {who}", kept(ctx)))
        }
        _ => text("not booked"),
    })
}

fn builder() -> rusty_mcp_server::ServerBuilder {
    Server::builder("ask", "1")
        .interactive_tool(Tool::new("book", schema()), book)
        .interactive_tool(Tool::new("other", schema()), book)
        .interactive_tool(Tool::new("empty", schema()), |_c, _p| {
            Ok(ToolOutcome::Ask(Ask::new()))
        })
        .tool(Tool::new("plain", schema()), |_c, _p| {
            Ok(CallToolResult::default())
        })
}

fn server() -> Arc<Server> {
    Arc::new(builder().build().unwrap())
}

fn call(id: i64, tool: &str, extra: &str, caps: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}"{extra},"_meta":{{{V},{caps}}}}}}}"#
    )
}

fn first(server: &Arc<Server>, tool: &str) -> Value {
    let out = run_with(server.clone(), &call(1, tool, "", ELICIT));
    result(reply(&out, 1)).clone()
}

fn text_of(r: &Value) -> &str {
    r.get("content")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no text in {r:?}"))
}

const ACCEPT: &str =
    r#","inputResponses":{"confirm":{"action":"accept","content":{"name":"Ann"}}}"#;

#[test]
fn a_tool_can_ask_and_the_retry_finishes() {
    let s = server();
    let asked = first(&s, "book");
    assert_eq!(
        asked.get("resultType").and_then(Value::as_str),
        Some("input_required")
    );
    let q = asked.get("inputRequests").unwrap().get("confirm").unwrap();
    assert_eq!(
        q.get("method").and_then(Value::as_str),
        Some("elicitation/create")
    );
    assert_eq!(
        q.get("params")
            .and_then(|p| p.get("message"))
            .and_then(Value::as_str),
        Some("Book it?")
    );

    let state = asked.get("requestState").and_then(Value::as_str);
    let echo = state.map_or_else(String::new, |s| format!(r#","requestState":"{s}""#));
    let out = run_with(
        s.clone(),
        &call(2, "book", &format!("{ACCEPT}{echo}"), ELICIT),
    );
    let done = result(reply(&out, 2));
    assert_eq!(
        done.get("resultType").and_then(Value::as_str),
        Some("complete")
    );
    let expect = if cfg!(feature = "request-state") {
        "booked draft:42 for Ann"
    } else {
        "booked - for Ann"
    };
    assert_eq!(text_of(done), expect);
}

#[test]
fn a_declined_question_is_the_tools_to_handle() {
    let out = run_with(
        server(),
        &call(
            1,
            "book",
            r#","inputResponses":{"confirm":{"action":"decline"}}"#,
            ELICIT,
        ),
    );
    assert_eq!(text_of(result(reply(&out, 1))), "not booked");
}

#[test]
fn a_malformed_answer_is_invalid_params() {
    let out = run_with(
        server(),
        &call(
            1,
            "book",
            r#","inputResponses":{"confirm":{"action":"maybe"}}"#,
            ELICIT,
        ),
    );
    assert_eq!(code(reply(&out, 1)), -32602);
}

#[test]
fn asking_needs_the_elicitation_capability() {
    let out = run_with(server(), &call(1, "book", "", NO_CAPS));
    assert_eq!(code(reply(&out, 1)), -32021);
}

#[test]
fn asking_needs_the_stateless_revision() {
    let input = format!(
        "{}\n{}\n",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{"elicitation":{}},"clientInfo":{"name":"c","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"book"}}"#,
    );
    let out = run_with(server(), &input);
    assert_eq!(code(reply(&out, 2)), -32600);
}

#[test]
fn an_empty_ask_is_a_server_error() {
    let out = run_with(server(), &call(1, "empty", "", ELICIT));
    assert_eq!(code(reply(&out, 1)), -32603);
}

#[test]
fn a_tool_that_never_asks_ignores_input_responses() {
    let out = run_with(server(), &call(1, "plain", ACCEPT, NO_CAPS));
    assert!(out[0].get("result").is_some());
}

#[cfg(feature = "request-state")]
mod sealed {
    use super::*;
    use std::time::Duration;

    fn state_of(server: &Arc<Server>, tool: &str) -> String {
        first(server, tool)
            .get("requestState")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned()
    }

    fn retry(server: &Arc<Server>, tool: &str, state: &str) -> Value {
        let extra = format!(r#"{ACCEPT},"requestState":"{state}""#);
        let out = run_with(server.clone(), &call(2, tool, &extra, ELICIT));
        reply(&out, 2).clone()
    }

    #[test]
    fn state_is_sealed_not_plain() {
        let state = state_of(&server(), "book");
        assert!(!state.contains("draft"), "{state}");
        assert_eq!(state.matches('.').count(), 1);
    }

    #[test]
    fn a_forged_state_is_refused() {
        let s = server();
        let mut state = state_of(&s, "book");
        let last = state.pop().unwrap();
        state.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(code(&retry(&s, "book", &state)), -32602);
        assert_eq!(code(&retry(&s, "book", "draft:42")), -32602);
    }

    #[test]
    fn a_state_is_good_for_its_own_tool_only() {
        let s = server();
        let state = state_of(&s, "book");
        assert_eq!(code(&retry(&s, "other", &state)), -32602);
        assert!(retry(&s, "book", &state).get("result").is_some());
    }

    #[test]
    fn a_shared_key_lets_another_instance_continue_and_a_random_one_does_not() {
        let key = vec![9u8; 32];
        let a = Arc::new(builder().state_key(key.clone()).build().unwrap());
        let b = Arc::new(builder().state_key(key).build().unwrap());
        let state = state_of(&a, "book");
        assert!(retry(&b, "book", &state).get("result").is_some());
        let stranger = server();
        assert_eq!(code(&retry(&stranger, "book", &state)), -32602);
    }

    #[test]
    fn an_old_state_expires() {
        let s = Arc::new(builder().state_ttl(Duration::from_secs(1)).build().unwrap());
        let state = state_of(&s, "book");
        std::thread::sleep(Duration::from_millis(2200));
        let r = retry(&s, "book", &state);
        assert_eq!(code(&r), -32602);
        let why = r["error"]["message"].as_str().unwrap();
        assert!(why.contains("expired"), "{why}");
    }

    #[test]
    fn a_weak_key_or_zero_lifetime_is_refused_at_build() {
        assert!(matches!(
            builder().state_key(vec![1u8; 31]).build(),
            Err(BuildError::WeakStateKey)
        ));
        assert!(matches!(
            builder().state_ttl(Duration::from_millis(500)).build(),
            Err(BuildError::ZeroLimit(_))
        ));
    }
}
