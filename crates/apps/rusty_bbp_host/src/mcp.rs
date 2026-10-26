//! An MCP server over stdio: newline-delimited JSON-RPC 2.0, the
//! `initialize`, `ping`, `tools/list` and `tools/call` methods, five tools
//! (six for a session).
//!
//! A per-turn server is the invocation. It learns its task, principal and turn
//! id from the environment, derives the execution token for that turn, and
//! once the turn has ended every tool call is refused, except the free card
//! and the exact replay of an operation the turn already accepted. Nothing the
//! model sends can change which turn this process speaks for.
//!
//! A session server (`BBP_SESSION=yes`, no `BBP_TURN`) lives for the whole
//! task and speaks for each turn of its principal's role, but never for the
//! process as a whole: `wait` shows a turn with a ticket, an unguessable value
//! derived from that turn's token, and every call that acts names the ticket
//! of the turn it acts in. The ticket is the invocation context. An inference
//! can carry only a ticket it has seen, so a call produced before a grant
//! (pipelined with `wait`, or queued behind it) holds no ticket or an earlier
//! one, and is refused with that earlier turn's token, never relabelled with
//! the new grant, however the requests are timed on the stream.

use crate::{fresh_op, now, open_driver, response_json};
use rusty_bbp::*;
use rusty_serde::Value;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long `wait` blocks when the model names no timeout, and at most.
const WAIT_DEFAULT: Duration = Duration::from_secs(60);
const WAIT_MAX: Duration = Duration::from_secs(600);
/// The longest a `wait` sleeps between log checks: the log watch wakes it
/// sooner, so this only bounds a platform or setup without one.
const WAIT_RECHECK: Duration = Duration::from_secs(1);

/// What one `bbp mcp` process is bound to.
#[derive(Clone, Debug)]
pub struct Binding {
    pub dir: PathBuf,
    pub task: TaskId,
    pub principal: PrincipalId,
    /// The one turn a per-turn server speaks for; `None` for a session.
    pub turn: Option<TurnId>,
}

impl Binding {
    /// Read `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL`, and either `BBP_TURN` or
    /// `BBP_SESSION=yes`.
    pub fn from_env() -> Result<Binding, String> {
        let var = |k: &str| std::env::var(k).map_err(|_| format!("missing ${k}"));
        let turn = match (
            std::env::var("BBP_TURN").ok(),
            std::env::var("BBP_SESSION").ok(),
        ) {
            (Some(t), None) => Some(TurnId(
                t.parse()
                    .map_err(|_| "BBP_TURN must be a number".to_owned())?,
            )),
            (None, Some(s)) if s == "yes" => None,
            _ => return Err("set exactly one of $BBP_TURN or $BBP_SESSION=yes".into()),
        };
        Ok(Binding {
            dir: PathBuf::from(var("BBP_DIR")?),
            task: TaskId(var("BBP_TASK")?),
            principal: PrincipalId(var("BBP_PRINCIPAL")?),
            turn,
        })
    }
}

pub struct Server {
    binding: Binding,
    driver: Driver<FsStore>,
    /// The turn `wait` last showed, to tell a new grant from a held one.
    shown: Option<TurnId>,
    /// A session's watch on the task log, made on its first `wait`.
    waker: Option<crate::waker::Waker>,
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn text_result(text: String, is_error: bool) -> Value {
    obj(vec![
        (
            "content",
            Value::Seq(vec![obj(vec![
                ("type", "text".into()),
                ("text", text.into()),
            ])]),
        ),
        ("isError", Value::Bool(is_error)),
    ])
}

fn schema(props: Vec<(&str, &str, &str)>, required: &[&str]) -> Value {
    let properties = props
        .into_iter()
        .map(|(name, ty, desc)| {
            (
                name.to_owned(),
                obj(vec![("type", ty.into()), ("description", desc.into())]),
            )
        })
        .collect();
    obj(vec![
        ("type", "object".into()),
        ("properties", Value::Map(properties)),
        (
            "required",
            Value::Seq(required.iter().map(|r| Value::from(*r)).collect()),
        ),
    ])
}

/// The tool catalogue, fixed by the protocol; a session adds `wait`.
pub fn tools(session: bool) -> Value {
    let tool = |name: &str, desc: &str, input: Value| {
        obj(vec![
            ("name", name.into()),
            ("description", desc.into()),
            ("inputSchema", input),
        ])
    };
    // A session's acting tools name the turn they act in by its ticket.
    let schema = |mut props: Vec<(&str, &str, &str)>, required: &[&str]| {
        let mut required = required.to_vec();
        if session {
            props.push((
                "ticket",
                "string",
                "The ticket wait returned for the turn you act in",
            ));
            required.push("ticket");
        }
        schema(props, &required)
    };
    Value::Seq(
        vec![
        tool(
            "task_card",
            "The task's current state, turn, candidate, run and budgets. Free; needs no turn.",
            self::schema(vec![], &[]),
        ),
        tool(
            "read",
            "Messages visible to your role with id greater than `after`, at most 20. Charged against your turn's read budget.",
            schema(vec![("after", "integer", "Last message id you have seen; omit for the start"), ("limit", "integer", "Page size, at most 20"), ("op", "string", "Operation id; omit to let the host choose")], &[]),
        ),
        tool(
            "get_artifact",
            "An artifact record and its content by id (`content`: brief, diff or log text, spec Markdown, candidate or report JSON). One charge per artifact per turn.",
            schema(vec![("id", "integer", "Artifact id"), ("op", "string", "Operation id")], &["id"]),
        ),
        tool(
            "post",
            "Append a message. kind: ask, answer, propose, finding, verdict, request_decision, pass. refs and evidence are strings like \"art:3#L10-L20\" or \"msg:7\". A verdict body is {subject: candidate id, run: run id, verdict: \"approve\"|\"revise\", blocking: [item], non_blocking: [item]} where every item is {id: string, ref: \"art:N\" or \"msg:N\", issue: string, fix: string}; approve needs a passed run. When the run failed for a reason no code change can fix (the toolchain or sandbox itself), do not post revise or resubmit: post kind=request_decision to [\"human\"] with gate=false, which ends your turn and waits for the human without spending an iteration.",
            schema(
                vec![
                    ("op", "string", "Operation id, unique for your role across the whole task, not just this turn (prefix it with your turn number); retry with the same op to get the same result, reuse with a different payload is refused"),
                    ("kind", "string", "Message kind"),
                    ("to", "array", "Recipients: role names, \"human\" or \"*\""),
                    ("body", "string", "Prose body, at most 1,200 characters (verdict: see verdict)"),
                    ("verdict", "object", "Verdict body for kind=verdict"),
                    ("refs", "array", "References"),
                    ("evidence", "array", "Evidence references"),
                    ("reply_to", "integer", "Message id this answers or objects to"),
                    ("yield_to", "string", "For pass: role to hand a consultation turn to"),
                    ("gate", "boolean", "For request_decision from a role that may write the spec: open the plan gate"),
                ],
                &["op", "kind"],
            ),
        ),
        tool(
            "put_artifact",
            "Store an artifact. kind spec: {brief, body=markdown}, or a co-edited version {brief, edit}; diff: {body}; candidate: {base, diffs[]}. Storing a candidate submits it. edit is {base: version id you read (omit to start a co-edited spec), ops: [{insert: {after: line id (omit for the start), lines: [text]}} or {delete: line id}]}; line ids are \"art:N#M\" as get_artifact lists them. An edit on a line someone changed since your base is refused stale_base: read the head and redo it.",
            schema(
                vec![
                    ("op", "string", "Operation id"),
                    ("kind", "string", "spec, diff or candidate"),
                    ("body", "string", "Markdown for spec, unified diff text for diff"),
                    ("brief", "integer", "spec: the brief artifact id"),
                    ("edit", "object", "spec: a co-edited version instead of body"),
                    ("base", "string", "candidate: full base commit id"),
                    ("diffs", "array", "candidate: diff artifact ids in apply order"),
                ],
                &["op", "kind"],
            ),
        ),
    ]
    .into_iter()
    .chain(session.then(|| {
        tool(
            "wait",
            "Block until your role is granted a turn, the task ends, or timeout_s passes, then say which (woke: turn, held, ended or timeout). Free. A turn comes with a ticket: pass it as `ticket` to every other tool except task_card while you act in that turn, and call wait again whenever you have nothing to do.",
            self::schema(
                vec![("timeout_s", "integer", "Longest wait in seconds; default 60, at most 600")],
                &[],
            ),
        )
    }))
    .collect())
}

fn parse_ref(s: &str) -> Result<Ref, String> {
    if let Some(rest) = s.strip_prefix("art:") {
        let (id, frag) = match rest.split_once('#') {
            Some((i, f)) => (i, Some(f.to_owned())),
            None => (rest, None),
        };
        let id = id.parse().map_err(|_| format!("bad artifact id in {s}"))?;
        return Ok(Ref::Art {
            id: ArtId(id),
            fragment: frag,
        });
    }
    if let Some(rest) = s.strip_prefix("msg:") {
        return rest
            .parse()
            .map(|i| Ref::Msg(MsgId(i)))
            .map_err(|_| format!("bad message id in {s}"));
    }
    Err(format!("reference must start with art: or msg:, got {s}"))
}

fn refs(v: Option<&Value>) -> Result<Vec<Ref>, String> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .ok_or_else(|| "reference must be a string".to_owned())
                    .and_then(parse_ref)
            })
            .collect(),
        Some(_) => Err("refs must be an array".into()),
    }
}

fn recipients(v: Option<&Value>) -> Result<Vec<Recipient>, String> {
    let Some(Value::Seq(items)) = v else {
        return Ok(vec![Recipient::All]);
    };
    items
        .iter()
        .map(|i| match i.as_str() {
            Some("*") => Ok(Recipient::All),
            Some("human") => Ok(Recipient::Human),
            Some(role) => Ok(Recipient::Role(RoleId::from(role))),
            None => Err("recipient must be a string".to_owned()),
        })
        .collect()
}

fn kind(s: &str) -> Result<MessageKind, String> {
    Ok(match s {
        "ask" => MessageKind::Ask,
        "answer" => MessageKind::Answer,
        "propose" => MessageKind::Propose,
        "finding" => MessageKind::Finding,
        "verdict" => MessageKind::Verdict,
        "request_decision" => MessageKind::RequestDecision,
        "decision" => MessageKind::Decision,
        "pass" => MessageKind::Pass,
        other => return Err(format!("unknown kind {other}")),
    })
}

fn u64_of(v: Option<&Value>, what: &str) -> Result<u64, String> {
    v.and_then(Value::as_u64)
        .ok_or_else(|| format!("{what} must be a non-negative integer"))
}

fn items(v: Option<&Value>, what: &str) -> Result<Vec<BlockingItem>, String> {
    let Some(Value::Seq(list)) = v else {
        return Ok(vec![]);
    };
    list.iter()
        .map(|i| {
            let s = |k: &str| {
                i.get(k)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{what} item needs {k}"))
            };
            Ok(BlockingItem {
                id: s("id")?,
                reference: parse_ref(&s("ref")?)?,
                issue: s("issue")?,
                fix: s("fix")?,
            })
        })
        .collect()
}

fn verdict_body(v: &Value) -> Result<Body, String> {
    let kind = match v.get("verdict").and_then(Value::as_str) {
        Some("approve") => VerdictKind::Approve,
        Some("revise") => VerdictKind::Revise,
        _ => return Err("verdict must be approve or revise".into()),
    };
    Ok(Body::Verdict(Verdict {
        subject: ArtId(u64_of(v.get("subject"), "subject")?),
        run: RunId(u64_of(v.get("run"), "run")?),
        verdict: kind,
        blocking: items(v.get("blocking"), "blocking")?,
        non_blocking: items(v.get("non_blocking"), "non_blocking")?,
    }))
}

/// A line id as get_artifact lists it: `art:N#M`.
fn line_id(v: Option<&Value>) -> Result<LineId, String> {
    let bad = || "a line id is a string \"art:N#M\"".to_owned();
    let s = v.and_then(Value::as_str).ok_or_else(bad)?;
    let (art, n) = s
        .strip_prefix("art:")
        .and_then(|r| r.split_once('#'))
        .ok_or_else(bad)?;
    Ok(LineId {
        art: ArtId(art.parse().map_err(|_| bad())?),
        n: n.parse().map_err(|_| bad())?,
    })
}

/// One op, naming exactly one variant: an op that also named another would
/// lose it silently (a replace sent as one object).
fn line_op(v: &Value) -> Result<LineOp, String> {
    const SHAPE: &str = "an op is exactly one of {insert: {after, lines}} or {delete: line id}";
    let (key, body) = match v {
        Value::Map(m) if m.len() == 1 => (m[0].0.as_str(), &m[0].1),
        _ => return Err(SHAPE.into()),
    };
    let ins = match key {
        "delete" => {
            return Ok(LineOp::Delete {
                line: line_id(Some(body))?,
            })
        }
        "insert" => body,
        _ => return Err(SHAPE.into()),
    };
    let after = match ins.get("after") {
        None | Some(Value::Null) => None,
        a => Some(line_id(a)?),
    };
    let Some(Value::Seq(lines)) = ins.get("lines") else {
        return Err("insert needs lines, an array of strings".into());
    };
    let lines = lines
        .iter()
        .map(|l| l.as_str().map(str::to_owned).ok_or("lines must be strings"))
        .collect::<Result<_, _>>()?;
    Ok(LineOp::Insert { after, lines })
}

/// The `edit` of a co-edited spec version; the core validates it.
fn spec_edit(brief: ArtId, e: &Value) -> Result<SpecEdit, String> {
    let base = match e.get("base") {
        None | Some(Value::Null) => None,
        b => Some(ArtId(u64_of(b, "edit.base")?)),
    };
    let Some(Value::Seq(ops)) = e.get("ops") else {
        return Err("edit needs ops, an array".into());
    };
    let ops = ops.iter().map(line_op).collect::<Result<_, _>>()?;
    Ok(SpecEdit { brief, base, ops })
}

fn op_of(a: &Value) -> OpId {
    a.get("op")
        .and_then(Value::as_str)
        .map(|s| OpId(s.to_owned()))
        .unwrap_or_else(fresh_op)
}

/// A tool call translated into an agent action, plus the artifact bytes to
/// store first when the call is `put_artifact`.
pub struct Translated {
    pub op: OpId,
    pub action: AgentAction,
    pub bytes: Option<(ArtifactPayload, Vec<u8>)>,
}

fn translated(
    op: OpId,
    action: AgentAction,
    bytes: Option<(ArtifactPayload, Vec<u8>)>,
) -> Translated {
    Translated { op, action, bytes }
}

/// Translate a tool call into an agent action.
pub fn action(name: &str, a: &Value) -> Result<Translated, String> {
    match name {
        "task_card" => Ok(translated(fresh_op(), AgentAction::TaskCard, None)),
        "read" => Ok(translated(
            op_of(a),
            AgentAction::Read {
                after: a.get("after").and_then(Value::as_u64).map(MsgId),
                limit: a.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize,
            },
            None,
        )),
        "get_artifact" => Ok(translated(
            op_of(a),
            AgentAction::GetArtifact {
                id: ArtId(u64_of(a.get("id"), "id")?),
            },
            None,
        )),
        "post" => {
            let k = kind(
                a.get("kind")
                    .and_then(Value::as_str)
                    .ok_or("kind required")?,
            )?;
            let body = match (k, a.get("verdict")) {
                (MessageKind::Verdict, Some(v)) => verdict_body(v)?,
                (MessageKind::Verdict, None) => return Err("verdict needs a verdict object".into()),
                _ => Body::Prose(
                    a.get("body")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                ),
            };
            let draft = Draft {
                kind: k,
                to: recipients(a.get("to"))?,
                body,
                refs: refs(a.get("refs"))?,
                evidence: refs(a.get("evidence"))?,
                reply_to: a.get("reply_to").and_then(Value::as_u64).map(MsgId),
                yield_to: a.get("yield_to").and_then(Value::as_str).map(RoleId::from),
                gate: a.get("gate").and_then(Value::as_bool).unwrap_or(false),
            };
            Ok(translated(op_of(a), AgentAction::Post(draft), None))
        }
        "put_artifact" => {
            let body = a
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            let payload = match a.get("kind").and_then(Value::as_str) {
                Some("spec") => {
                    let brief = ArtId(u64_of(a.get("brief"), "brief")?);
                    match a.get("edit") {
                        Some(e) => ArtifactPayload::SpecEdit(spec_edit(brief, e)?),
                        None => ArtifactPayload::Spec { brief },
                    }
                }
                Some("diff") => ArtifactPayload::Diff,
                Some("candidate") => {
                    let diffs = match a.get("diffs") {
                        Some(Value::Seq(d)) => d
                            .iter()
                            .map(|x| {
                                x.as_u64()
                                    .map(ArtId)
                                    .ok_or_else(|| "diffs must be integers".to_owned())
                            })
                            .collect::<Result<_, _>>()?,
                        _ => vec![],
                    };
                    ArtifactPayload::Candidate(Candidate {
                        base: a
                            .get("base")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        diffs,
                    })
                }
                other => {
                    return Err(format!(
                        "put_artifact kind must be spec, diff or candidate, got {other:?}"
                    ))
                }
            };
            // The blob is filled in by the server once the bytes are stored.
            let placeholder = BlobRef {
                sha: Sha256([0; 32]),
                len: 0,
            };
            Ok(translated(
                op_of(a),
                AgentAction::PutArtifact {
                    blob: placeholder,
                    payload: payload.clone(),
                },
                Some((payload, body)),
            ))
        }
        other => Err(format!("unknown tool {other}")),
    }
}

impl Server {
    pub fn new(binding: Binding) -> Result<Server, String> {
        let driver = open_driver(&binding.dir, &binding.task)?;
        Ok(Server {
            binding,
            driver,
            shown: None,
            waker: None,
        })
    }

    fn token(&self, turn: TurnId) -> Token {
        self.driver.state.token_for(&self.binding.principal, turn)
    }

    /// True while `turn` is live.
    fn live(&self, turn: TurnId) -> bool {
        self.driver.state.turns.iter().any(|t| t.id == turn)
    }

    /// The live turn of this principal's role, if it holds one.
    fn held(&self) -> Option<TurnId> {
        let role = self.driver.state.role_of(&self.binding.principal)?;
        self.driver.state.turn_of(&role).map(|t| t.id)
    }

    /// The ticket for `turn`: its number and a digest of its token, which
    /// names the turn without revealing the token.
    fn ticket(&self, turn: TurnId) -> String {
        let mut bytes = self.token(turn).0 .0.to_vec();
        bytes.extend_from_slice(b"|ticket");
        format!("{}.{}", turn.0, &Sha256::of(&bytes).hex()[..32])
    }

    /// The turn a session call acts in, from its `ticket` argument.
    fn ticketed(&self, args: &Value) -> Result<TurnId, String> {
        let ticket = args
            .get("ticket")
            .and_then(Value::as_str)
            .ok_or("a session acts only with the ticket wait returned; call wait")?;
        ticket
            .split_once('.')
            .and_then(|(n, _)| n.parse().ok())
            .map(TurnId)
            .filter(|&t| self.ticket(t) == ticket)
            .ok_or_else(|| "unknown ticket; call wait for yours".to_owned())
    }

    /// Why a session's `wait` should return now, if it should.
    fn woke(&mut self) -> Option<Value> {
        let (woke, turn) = if self.driver.state.state.terminal() {
            ("ended", None)
        } else {
            let held = self.held()?;
            let seen = self.shown.replace(held) == Some(held);
            (if seen { "held" } else { "turn" }, Some(held))
        };
        Some(self.wake_reply(woke, turn))
    }

    /// A `wait` answer: why it returned, and the turn and ticket it shows.
    fn wake_reply(&self, woke: &str, turn: Option<TurnId>) -> Value {
        obj(vec![
            ("woke", woke.into()),
            ("turn", turn.map_or(Value::Null, |t| Value::from(t.0))),
            (
                "ticket",
                turn.map_or(Value::Null, |t| self.ticket(t).into()),
            ),
            ("state", format!("{:?}", self.driver.state.state).into()),
        ])
    }

    /// `wait`: block until [`Server::woke`] has a reason or the timeout passes.
    fn wait(&mut self, args: &Value) -> Result<Value, String> {
        if self.binding.turn.is_some() {
            return Err("wait is for a session; this server speaks for one turn".into());
        }
        let timeout = args
            .get("timeout_s")
            .and_then(Value::as_u64)
            .map_or(WAIT_DEFAULT, Duration::from_secs)
            .min(WAIT_MAX);
        let deadline = Instant::now() + timeout;
        loop {
            self.driver.sync().map_err(|e| format!("{e:?}"))?;
            if let Some(reply) = self.woke() {
                return Ok(reply);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(self.wake_reply("timeout", None));
            }
            let log = log_path_for(&self.binding.dir, &self.binding.task);
            self.waker
                .get_or_insert_with(|| crate::waker::Waker::new(&log))
                .wait(&[], left.min(WAIT_RECHECK))?;
        }
    }

    /// True when this principal already had `op` accepted. Its exact replay
    /// stays answerable after the turn ends (R15); the core refuses a
    /// different payload under the same op.
    fn accepted(&self, op: &OpId) -> bool {
        self.driver
            .state
            .ops
            .contains_key(&(self.binding.principal.clone(), op.clone()))
    }

    /// The content of an artifact the core let this principal read: raw
    /// bytes as text for brief, diff and log; the Markdown for a spec; the
    /// JSON for a candidate or test report.
    fn content(&self, rec: &ArtifactRecord) -> Result<String, String> {
        let bytes = self
            .driver
            .store
            .blob_get(&rec.blob.sha)
            .map_err(|e| format!("{e:?}"))?;
        if rec.kind != ArtifactKind::Spec {
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        if let ArtifactPayload::SpecEdit(_) = rec.payload {
            return self
                .version(rec.id)
                .map(|(_, v)| v.doc.text())
                .ok_or_else(|| format!("{} did not apply", rec.id));
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        rusty_serde::json::from_str::<SpecFile>(text)
            .map(|f| f.markdown)
            .map_err(|e| e.to_string())
    }

    /// A co-edited version and its spec's head.
    fn version(&self, id: ArtId) -> Option<(ArtId, &Version)> {
        let st = &self.driver.state;
        Some((st.head(id)?, st.versions.get(&id)?))
    }

    /// What a co-edited version adds to `get_artifact`: its spec's head, its
    /// snapshot and its visible lines with their ids, which edits name.
    fn version_fields(&self, id: ArtId) -> Vec<(String, Value)> {
        let Some((head, v)) = self.version(id) else {
            return vec![];
        };
        let lines = v
            .doc
            .lines()
            .map(|(l, text)| {
                Value::Map(vec![
                    ("id".to_owned(), format!("{}#{}", l.art, l.n).into()),
                    ("text".to_owned(), text.into()),
                ])
            })
            .collect();
        vec![
            ("head".to_owned(), Value::UInt(head.0)),
            ("snapshot".to_owned(), v.snapshot.hex().into()),
            ("lines".to_owned(), Value::Seq(lines)),
        ]
    }

    /// Render a response for the model, attaching the artifact content a
    /// successful `get_artifact` promises.
    fn render(&self, resp: &Response) -> Value {
        let is_error = matches!(resp, Response::Rejected(_));
        let text = response_json(resp);
        let Response::Artifact(rec) = resp else {
            return text_result(text, is_error);
        };
        let content = match self.content(rec) {
            Ok(c) => c,
            Err(e) => return text_result(format!("artifact content unreadable: {e}"), true),
        };
        match rusty_serde::json::from_str::<Value>(&text) {
            Ok(Value::Map(mut m)) => {
                m.push(("content".to_owned(), content.into()));
                m.extend(self.version_fields(rec.id));
                text_result(Value::Map(m).to_string(), false)
            }
            Ok(other) => text_result(other.to_string(), true),
            Err(e) => text_result(e.to_string(), true),
        }
    }

    /// Run one tool call end to end.
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Response, String> {
        self.driver.sync().map_err(|e| format!("{e:?}"))?;
        let Translated {
            op,
            action: mut act,
            bytes,
        } = action(name, args)?;
        let free = matches!(act, AgentAction::TaskCard);
        let turn = match self.binding.turn {
            Some(t) => Some(t),
            None if free => None,
            None => Some(self.ticketed(args)?),
        };
        let live = turn.is_some_and(|t| self.live(t));
        if !free && !live && !self.accepted(&op) {
            return Err(match (self.binding.turn, turn) {
                (Some(t), _) => format!(
                    "turn {} has ended; this server speaks for no later turn",
                    t.0
                ),
                (None, Some(t)) => format!("turn {} has ended; call wait for your next turn", t.0),
                (None, None) => "you hold no turn; call wait".to_owned(),
            });
        }
        if let Some((payload, body)) = bytes {
            let encoded = encode_artifact(&payload, &body).map_err(|e| e.to_string())?;
            let blob = self.driver.store.blob_put(&encoded);
            act = AgentAction::PutArtifact { blob, payload };
        }
        // A replay after the turn ended still names the turn it was made in.
        let token = turn
            .filter(|_| !matches!(act, AgentAction::TaskCard))
            .map(|t| self.token(t));
        let cmd = Command::Agent {
            principal: self.binding.principal.clone(),
            token,
            op,
            action: act,
        };
        self.driver
            .dispatch(&cmd, now())
            .map_err(|e| format!("{e:?}"))
    }

    /// Handle one JSON-RPC request; `None` for notifications.
    pub fn handle(&mut self, req: &Value) -> Option<Value> {
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        if id.is_none() || id.as_ref().map(Value::is_null).unwrap_or(true) {
            return None;
        }
        let empty = Value::Map(vec![]);
        let params = req.get("params").unwrap_or(&empty);
        let result = match method {
            "initialize" => Ok(obj(vec![
                (
                    "protocolVersion",
                    params
                        .get("protocolVersion")
                        .cloned()
                        .unwrap_or_else(|| "2025-06-18".into()),
                ),
                ("capabilities", obj(vec![("tools", obj(vec![]))])),
                (
                    "serverInfo",
                    obj(vec![
                        ("name", "bbp".into()),
                        ("version", env!("CARGO_PKG_VERSION").into()),
                    ]),
                ),
            ])),
            "ping" => Ok(obj(vec![])),
            "tools/list" => Ok(obj(vec![("tools", tools(self.binding.turn.is_none()))])),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Map(vec![]));
                Ok(match name {
                    "wait" => match self.wait(&args) {
                        Ok(reply) => text_result(reply.to_string(), false),
                        Err(e) => text_result(e, true),
                    },
                    _ => match self.call(name, &args) {
                        Ok(resp) => self.render(&resp),
                        Err(e) => text_result(e, true),
                    },
                })
            }
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let mut out = vec![
            ("jsonrpc", Value::from("2.0")),
            ("id", id.unwrap_or(Value::Null)),
        ];
        match result {
            Ok(r) => out.push(("result", r)),
            Err((code, message)) => out.push((
                "error",
                obj(vec![
                    ("code", Value::Int(code)),
                    ("message", message.into()),
                ]),
            )),
        }
        Some(obj(out))
    }

    /// Serve stdin to stdout until EOF.
    pub fn serve(&mut self) -> io::Result<()> {
        self.serve_on(io::stdin().lock(), io::stdout().lock())
    }

    /// Serve newline-delimited requests from `input` to `output` until EOF.
    pub fn serve_on(&mut self, input: impl BufRead, mut stdout: impl Write) -> io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let req: Value = match rusty_serde::json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let err = obj(vec![
                        ("jsonrpc", "2.0".into()),
                        ("id", Value::Null),
                        (
                            "error",
                            obj(vec![
                                ("code", Value::Int(-32700)),
                                ("message", format!("parse error: {e}").into()),
                            ]),
                        ),
                    ]);
                    writeln!(stdout, "{err}")?;
                    stdout.flush()?;
                    continue;
                }
            };
            if let Some(resp) = self.handle(&req) {
                writeln!(stdout, "{resp}")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }
}
