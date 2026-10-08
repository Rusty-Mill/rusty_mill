//! A per-turn MCP server over stdio: newline-delimited JSON-RPC 2.0, the
//! `initialize`, `ping`, `tools/list` and `tools/call` methods, five tools.
//!
//! The process is the invocation. It learns its task, principal and turn id
//! from the environment, derives the execution token for that turn, and once
//! the turn has ended every tool call is refused. Nothing the model sends can
//! change which turn this process speaks for.

use crate::{fresh_op, now, open_driver, response_json};
use rusty_bbp::*;
use rusty_serde::Value;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// What one `bbp mcp` process is bound to.
#[derive(Clone, Debug)]
pub struct Binding {
    pub dir: PathBuf,
    pub task: TaskId,
    pub principal: PrincipalId,
    pub turn: TurnId,
}

impl Binding {
    /// Read `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL`, `BBP_TURN`.
    pub fn from_env() -> Result<Binding, String> {
        let var = |k: &str| std::env::var(k).map_err(|_| format!("missing ${k}"));
        Ok(Binding {
            dir: PathBuf::from(var("BBP_DIR")?),
            task: TaskId(var("BBP_TASK")?),
            principal: PrincipalId(var("BBP_PRINCIPAL")?),
            turn: TurnId(
                var("BBP_TURN")?
                    .parse()
                    .map_err(|_| "BBP_TURN must be a number".to_owned())?,
            ),
        })
    }
}

pub struct Server {
    binding: Binding,
    driver: Driver<FsStore>,
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

/// The tool catalogue, fixed by the protocol.
pub fn tools() -> Value {
    let tool = |name: &str, desc: &str, input: Value| {
        obj(vec![
            ("name", name.into()),
            ("description", desc.into()),
            ("inputSchema", input),
        ])
    };
    Value::Seq(vec![
        tool(
            "task_card",
            "The task's current state, turn, candidate, run and budgets. Free; needs no turn.",
            schema(vec![], &[]),
        ),
        tool(
            "read",
            "Messages visible to your role with id greater than `after`, at most 20. Charged against your turn's read budget.",
            schema(vec![("after", "integer", "Last message id you have seen; omit for the start"), ("limit", "integer", "Page size, at most 20"), ("op", "string", "Operation id; omit to let the host choose")], &[]),
        ),
        tool(
            "get_artifact",
            "An artifact record and its content by id. One charge per artifact per turn.",
            schema(vec![("id", "integer", "Artifact id"), ("op", "string", "Operation id")], &["id"]),
        ),
        tool(
            "post",
            "Append a message. kind: ask, answer, propose, finding, verdict, request_decision, pass. refs and evidence are strings like \"art:3#L10-L20\" or \"msg:7\". A verdict body is an object {subject, run, verdict, blocking[], non_blocking[]}.",
            schema(
                vec![
                    ("op", "string", "Operation id, unique per call; retry with the same op to get the same result"),
                    ("kind", "string", "Message kind"),
                    ("to", "array", "Recipients: role names, \"human\" or \"*\""),
                    ("body", "string", "Prose body, at most 1,200 characters (verdict: see verdict)"),
                    ("verdict", "object", "Verdict body for kind=verdict"),
                    ("refs", "array", "References"),
                    ("evidence", "array", "Evidence references"),
                    ("reply_to", "integer", "Message id this answers or objects to"),
                    ("yield_to", "string", "For pass: role to hand a consultation turn to"),
                    ("gate", "boolean", "For request_decision from the Planner: open the plan gate"),
                ],
                &["op", "kind"],
            ),
        ),
        tool(
            "put_artifact",
            "Store an artifact. kind spec: {brief, body=markdown}; diff: {body}; candidate: {base, diffs[]}. Storing a candidate submits it.",
            schema(
                vec![
                    ("op", "string", "Operation id"),
                    ("kind", "string", "spec, diff or candidate"),
                    ("body", "string", "Markdown for spec, unified diff text for diff"),
                    ("brief", "integer", "spec: the brief artifact id"),
                    ("base", "string", "candidate: full base commit id"),
                    ("diffs", "array", "candidate: diff artifact ids in apply order"),
                ],
                &["op", "kind"],
            ),
        ),
    ])
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
            Some(role) => crate::admin::parse_role(role).map(Recipient::Role),
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
                yield_to: a
                    .get("yield_to")
                    .and_then(Value::as_str)
                    .map(crate::admin::parse_role)
                    .transpose()?,
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
                Some("spec") => ArtifactPayload::Spec {
                    brief: ArtId(u64_of(a.get("brief"), "brief")?),
                },
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
        Ok(Server { binding, driver })
    }

    fn token(&self) -> Token {
        self.driver
            .state
            .token_for(&self.binding.principal, self.binding.turn)
    }

    /// True while this process's turn is the live one.
    fn turn_live(&self) -> bool {
        self.driver.state.turn.as_ref().map(|t| t.id) == Some(self.binding.turn)
    }

    /// Run one tool call end to end.
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Response, String> {
        self.driver.sync().map_err(|e| format!("{e:?}"))?;
        let Translated {
            op,
            action: mut act,
            bytes,
        } = action(name, args)?;
        if !matches!(act, AgentAction::TaskCard) && !self.turn_live() {
            return Err(format!(
                "turn {} has ended; this server speaks for no later turn",
                self.binding.turn.0
            ));
        }
        if let Some((payload, body)) = bytes {
            let encoded = encode_artifact(&payload, &body).map_err(|e| e.to_string())?;
            let blob = self.driver.store.blob_put(&encoded);
            act = AgentAction::PutArtifact { blob, payload };
        }
        let token = (!matches!(act, AgentAction::TaskCard)).then(|| self.token());
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
            "tools/list" => Ok(obj(vec![("tools", tools())])),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Map(vec![]));
                Ok(match self.call(name, &args) {
                    Ok(resp) => {
                        text_result(response_json(&resp), matches!(resp, Response::Rejected(_)))
                    }
                    Err(e) => text_result(e, true),
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
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines() {
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
