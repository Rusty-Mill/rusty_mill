//! The builder's ACP client (#1318 S3): one turn of the Agent Client
//! Protocol, newline-delimited JSON-RPC 2.0 over orch's end of the fd-4
//! socketpair, and the `acp.log` renderer.
//!
//! One turn is `initialize` (no client fs or terminal capabilities, D101),
//! `session/new` with the worktree as `cwd` and no MCP servers, one
//! `session/prompt` carrying the builder prompt, then `session/update`
//! notifications until the prompt answers with its `stopReason`.
//!
//! Permission requests are answered with the offered `allow_once` option,
//! or `allow_always` only when there is no `allow_once`; with neither the
//! request is cancelled. Every request and answer is written to the log. In
//! v0 the jail confines and the agent's built-in tools stay on (D101, D187):
//! a permission prompt is not a boundary.
//!
//! The turn is synchronous. Whoever enforces the wall clock (S5) shuts the
//! socket down from another thread; the read then ends and the turn fails
//! as [`TurnError::Closed`]. A malformed line fails it too. Nothing here
//! waits without reading, so a turn never hangs on a dead agent.
//!
//! The log is a display aid, never evidence (design §6).

use std::io::{BufRead, Write};
use std::path::Path;

use rusty_json::{json, Value};

/// The ACP protocol version this client speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// The longest line an agent may send: well past any real ACP message, far
/// short of what would hurt orch.
pub const MAX_LINE: usize = 8 << 20;

/// JSON-RPC "method not found": the answer to any agent request other than
/// a permission request, since this client offers no fs or terminal.
const METHOD_NOT_FOUND: i64 = -32601;

/// Why the agent ended its turn: ACP's `stopReason`, kept as sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopReason(pub String);

impl StopReason {
    /// True for `end_turn`, the only reason a candidate may follow (D188).
    pub fn is_end_turn(&self) -> bool {
        self.0 == "end_turn"
    }
}

/// Tokens the agent reported for the turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// A finished turn. `usage` is `None` when the agent reported none, which
/// is not the same as zero (D187).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub stop: StopReason,
    pub usage: Option<Usage>,
}

impl Turn {
    /// Tokens in and out for the session row: 0 when none were reported.
    pub fn tokens(&self) -> (u64, u64) {
        self.usage.map_or((0, 0), |u| (u.tokens_in, u.tokens_out))
    }
}

/// Why a turn failed. Every variant ends the session as failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnError {
    /// The socket closed (or was shut down) before the prompt answered.
    Closed,
    /// A line that is not a JSON-RPC message.
    Malformed(String),
    /// The agent answered a request with an error, or with a result of the
    /// wrong shape.
    Protocol(String),
    /// Reading the socket or writing the socket or log failed.
    Io(String),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("the agent's ACP socket closed before the turn ended"),
            Self::Malformed(e) => write!(f, "malformed ACP line: {e}"),
            Self::Protocol(e) => write!(f, "ACP protocol error: {e}"),
            Self::Io(e) => write!(f, "ACP i/o: {e}"),
        }
    }
}

impl std::error::Error for TurnError {}

fn io(e: std::io::Error) -> TurnError {
    TurnError::Io(e.to_string())
}

/// One message read from the agent.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The answer to one of our requests.
    Response {
        id: u64,
        outcome: Result<Value, String>,
    },
    /// A request from the agent, to be answered with the same `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// A notification from the agent.
    Notification { method: String, params: Value },
}

/// Parse one line from the agent.
///
/// # Errors
/// When the line is not JSON, not JSON-RPC 2.0, or a response to an id we
/// never send (ours are whole numbers).
pub fn parse(line: &str) -> Result<Incoming, TurnError> {
    let v = Value::parse(line.trim()).map_err(|e| TurnError::Malformed(e.to_string()))?;
    if v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(TurnError::Malformed("not JSON-RPC 2.0".into()));
    }
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match (v.get("method").and_then(Value::as_str), v.get("id")) {
        (Some(method), Some(id)) => Ok(Incoming::Request {
            id: id.clone(),
            method: method.to_owned(),
            params,
        }),
        (Some(method), None) => Ok(Incoming::Notification {
            method: method.to_owned(),
            params,
        }),
        (None, Some(id)) => {
            let id = id
                .as_u64()
                .ok_or_else(|| TurnError::Malformed(format!("response id {id:?}")))?;
            Ok(Incoming::Response {
                id,
                outcome: response_outcome(&v),
            })
        }
        (None, None) => Err(TurnError::Malformed("neither a method nor an id".into())),
    }
}

fn response_outcome(v: &Value) -> Result<Value, String> {
    if let Some(error) = v.get("error") {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("");
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        return Err(format!("{code} {message}"));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// A request line from orch.
pub fn request(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_json_string()
}

/// A notification line from orch.
pub fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_json_string()
}

/// An answer to an agent's request.
pub fn answer(id: &Value, outcome: Result<Value, (i64, &str)>) -> String {
    match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id.clone(), "result": result}),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id.clone(),
            "error": {"code": code, "message": message}
        }),
    }
    .to_json_string()
}

/// The `initialize` params: no fs, no terminal (D101).
pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "clientCapabilities": {
            "fs": {"readTextFile": false, "writeTextFile": false},
            "terminal": false
        }
    })
}

/// The `session/new` params: the worktree as `cwd`, no MCP servers in v0.
pub fn new_session_params(cwd: &Path) -> Value {
    json!({"cwd": cwd.display().to_string(), "mcpServers": []})
}

/// The `session/prompt` params: one text block.
pub fn prompt_params(session: &str, text: &str) -> Value {
    json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]})
}

/// The option orch picks for a permission request: `allow_once`, else
/// `allow_always`, else none (the request is cancelled).
pub fn permission_choice(params: &Value) -> Option<String> {
    let options = params.get("options").and_then(Value::as_array)?;
    let id_of = |kind: &str| {
        options
            .iter()
            .find(|o| o.get("kind").and_then(Value::as_str) == Some(kind))
            .and_then(|o| o.get("optionId").and_then(Value::as_str))
            .map(str::to_owned)
    };
    id_of("allow_once").or_else(|| id_of("allow_always"))
}

/// The result answering a permission request with `choice`.
pub fn permission_result(choice: Option<&str>) -> Value {
    match choice {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

/// Usage from a prompt result, when reported (`usage.inputTokens` and
/// `usage.outputTokens`).
pub fn usage_of(result: &Value) -> Option<Usage> {
    let usage = result.get("usage")?;
    Some(Usage {
        tokens_in: usage.get("inputTokens").and_then(Value::as_u64)?,
        tokens_out: usage.get("outputTokens").and_then(Value::as_u64)?,
    })
}

/// One ACP connection: the agent's lines in, orch's lines out, rendered
/// lines to the log.
pub struct Client<R, W, L> {
    from_agent: R,
    to_agent: W,
    log: L,
    render: Render,
    next_id: u64,
}

impl<R: BufRead, W: Write, L: Write> Client<R, W, L> {
    /// A client over an agent's ends, logging to `log`.
    pub fn new(from_agent: R, to_agent: W, log: L) -> Self {
        Self {
            from_agent,
            to_agent,
            log,
            render: Render::default(),
            next_id: 1,
        }
    }

    /// Run one whole turn in `cwd` with `prompt`.
    ///
    /// # Errors
    /// See [`TurnError`]; each ends the session as failed.
    pub fn turn(&mut self, cwd: &Path, prompt: &str) -> Result<Turn, TurnError> {
        self.call("initialize", initialize_params())?;
        let session = self.call("session/new", new_session_params(cwd))?;
        let session = session
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| TurnError::Protocol("session/new answered no sessionId".into()))?
            .to_owned();
        let result = self.call("session/prompt", prompt_params(&session, prompt))?;
        let stop = result
            .get("stopReason")
            .and_then(Value::as_str)
            .ok_or_else(|| TurnError::Protocol("session/prompt answered no stopReason".into()))?;
        let turn = Turn {
            stop: StopReason(stop.to_owned()),
            usage: usage_of(&result),
        };
        let end = self.render.end(&turn);
        self.log(&end)?;
        Ok(turn)
    }

    /// The log, once the turn is over.
    pub fn into_log(self) -> L {
        self.log
    }

    /// Send a request and serve the agent until it is answered.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, TurnError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&request(id, method, params))?;
        loop {
            match self.read()? {
                Incoming::Response { id: got, outcome } if got == id => {
                    return outcome.map_err(|e| TurnError::Protocol(format!("{method}: {e}")));
                }
                Incoming::Response { id: got, .. } => {
                    return Err(TurnError::Protocol(format!(
                        "answer to {got} while waiting for {id}"
                    )));
                }
                Incoming::Notification { method, params } => self.notified(&method, &params)?,
                Incoming::Request { id, method, params } => {
                    self.requested(&id, &method, &params)?;
                }
            }
        }
    }

    fn notified(&mut self, method: &str, params: &Value) -> Result<(), TurnError> {
        if method != "session/update" {
            return Ok(());
        }
        let Some(update) = params.get("update") else {
            return Ok(());
        };
        let text = self.render.update(update);
        self.log(&text)
    }

    fn requested(&mut self, id: &Value, method: &str, params: &Value) -> Result<(), TurnError> {
        if method != "session/request_permission" {
            let text = self.render.refused(method);
            self.log(&text)?;
            return self.send(&answer(id, Err((METHOD_NOT_FOUND, "not offered"))));
        }
        let choice = permission_choice(params);
        let text = self.render.permission(params, choice.as_deref());
        self.log(&text)?;
        self.send(&answer(id, Ok(permission_result(choice.as_deref()))))
    }

    fn read(&mut self) -> Result<Incoming, TurnError> {
        loop {
            let line = read_line(&mut self.from_agent, MAX_LINE)?.ok_or(TurnError::Closed)?;
            let line = String::from_utf8(line)
                .map_err(|e| TurnError::Malformed(format!("not UTF-8: {e}")))?;
            if !line.trim().is_empty() {
                return parse(&line);
            }
        }
    }

    fn send(&mut self, line: &str) -> Result<(), TurnError> {
        writeln!(self.to_agent, "{line}").map_err(io)?;
        self.to_agent.flush().map_err(io)
    }

    fn log(&mut self, text: &str) -> Result<(), TurnError> {
        self.log.write_all(text.as_bytes()).map_err(io)?;
        self.log.flush().map_err(io)
    }
}

/// One line from `from`, without its newline, read at most `max` bytes at
/// a time so an agent that never ends a line can't grow orch's memory;
/// `None` at end of input.
///
/// # Errors
/// [`TurnError::Malformed`] past `max` bytes, [`TurnError::Io`] on a read
/// error.
pub fn read_line(from: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, TurnError> {
    let mut line = Vec::new();
    loop {
        let buf = from.fill_buf().map_err(io)?;
        if buf.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let (take, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(at) => (at + 1, true),
            None => (buf.len(), false),
        };
        if line.len() + take > max.saturating_add(1) {
            return Err(TurnError::Malformed(format!("a line over {max} bytes")));
        }
        line.extend_from_slice(&buf[..take]);
        from.consume(take);
        if done {
            line.pop();
            return Ok(Some(line));
        }
    }
}

/// `text` with every control character but (when `keep_newlines`) `\n`,
/// and every hidden character (see [`is_hidden`]), written as an escape,
/// so agent text can't drive the viewer's terminal (ESC, CSI, OSC), break
/// a line in an editor, or reorder or hide what it shows.
pub fn clean(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' if keep_newlines => out.push(c),
            c if c.is_control() || is_hidden(c) => out.push_str(&c.escape_default().to_string()),
            c => out.push(c),
        }
    }
    out
}

/// Characters that change what a viewer shows without showing themselves:
/// Unicode's bidi controls (marks, embeddings, overrides, isolates), the
/// line and paragraph separators editors break lines on, and zero-width
/// characters.
fn is_hidden(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// What starts every line of agent text in `acp.log`. orch's own lines
/// start at column zero with `[`, so agent text can't pass for one.
pub const AGENT_INDENT: &str = "  ";

/// Renders ACP traffic as `acp.log` text. Agent text streams in chunks and
/// is written as it comes, each line indented by [`AGENT_INDENT`]; every
/// other event takes a line of its own at column zero.
#[derive(Debug, Default)]
pub struct Render {
    mid_line: bool,
}

impl Render {
    /// The text for one `session/update` payload.
    pub fn update(&mut self, update: &Value) -> String {
        let str_of = |k: &str| clean(update.get(k).and_then(Value::as_str).unwrap_or(""), false);
        match str_of("sessionUpdate").as_str() {
            "agent_message_chunk" => {
                let text = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if text.is_empty() {
                    return String::new();
                }
                self.indented(&clean(text, true))
            }
            "tool_call" => self.line(&format!(
                "[tool] {} ({})",
                str_of("title"),
                status_or(&str_of("status"), "pending")
            )),
            "tool_call_update" if !str_of("status").is_empty() => self.line(&format!(
                "[tool] {} {}",
                str_of("toolCallId"),
                str_of("status")
            )),
            "plan" => {
                let entries = update.get("entries").and_then(Value::as_array);
                let lines: Vec<String> = entries
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let s =
                            |k: &str| clean(e.get(k).and_then(Value::as_str).unwrap_or(""), false);
                        format!("[plan] {} {}", plan_mark(&s("status")), s("content"))
                    })
                    .collect();
                self.line(&lines.join("\n"))
            }
            _ => String::new(),
        }
    }

    /// The lines for a permission request and orch's answer.
    pub fn permission(&mut self, params: &Value, choice: Option<&str>) -> String {
        let title = params
            .get("toolCall")
            .and_then(|t| t.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("a tool call");
        let title = clean(title, false);
        let answer = choice.map_or("cancelled (no allow option)".to_owned(), |c| {
            format!("answered {}", clean(c, false))
        });
        self.line(&format!("[permission] {title}: {answer}"))
    }

    /// The line for an agent request this client does not offer.
    pub fn refused(&mut self, method: &str) -> String {
        self.line(&format!("[refused] {}: not offered", clean(method, false)))
    }

    /// The closing lines: usage (or that none was reported) and the stop.
    pub fn end(&mut self, turn: &Turn) -> String {
        let usage = turn.usage.map_or("[usage] not reported".to_owned(), |u| {
            format!("[usage] in {} out {}", u.tokens_in, u.tokens_out)
        });
        self.line(&format!("{usage}\n[stop] {}", clean(&turn.stop.0, false)))
    }

    /// Agent `text` with [`AGENT_INDENT`] at the start of each non-empty line.
    fn indented(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + AGENT_INDENT.len());
        for c in text.chars() {
            if c != '\n' && !self.mid_line {
                out.push_str(AGENT_INDENT);
            }
            self.mid_line = c != '\n';
            out.push(c);
        }
        out
    }

    /// `text` on lines of its own, after a newline if agent text was mid-line.
    fn line(&mut self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        let lead = if self.mid_line { "\n" } else { "" };
        self.mid_line = false;
        format!("{lead}{text}\n")
    }
}

fn status_or<'a>(status: &'a str, default: &'a str) -> &'a str {
    if status.is_empty() {
        default
    } else {
        status
    }
}

fn plan_mark(status: &str) -> &'static str {
    match status {
        "completed" => "[x]",
        "in_progress" => "[~]",
        _ => "[ ]",
    }
}
