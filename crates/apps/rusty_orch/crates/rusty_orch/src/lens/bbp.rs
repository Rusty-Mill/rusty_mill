//! The lens runner's way into BBP (#594, D54): one `bbp mcp` session per
//! lens, speaking for that lens's principal with its session credential.
//!
//! - [`Child`]: the session process, newline-delimited JSON-RPC over its
//!   stdio, every call bounded by a deadline.
//! - [`Session`]: the claims tools over any [`Rpc`]. It keeps what the
//!   lens can see: claim bodies from incremental `read` pages (the card
//!   view carries no stakes, reversibility or question), and the cards,
//!   with their accept lines and evidence notes (D77), from
//!   `claims_delta`. It is the lens's [`Board`]: posts go out under its
//!   credential, with a stable op id so a retry replays.
//!
//! `read` is charged per call (addendum R11), so a session reads only new
//! messages, a page at a time; the card, line and delta views are free.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use orch_lens::card::Card;
use orch_lens::context::{Line, MAX_LINES};
use orch_lens::round::Board;
use orch_lens::{LensConfig, Post};
use rusty_bbp::{
    ArtId, ArtifactKind, ArtifactPayload, ArtifactRecord, Body, ClaimBody, ClaimStatus, ClaimView,
    Code, Direction, EvidenceKind, MsgId, Ref, Response, Rev, Stakes,
};
use rusty_serde::Value;

use super::probes::Candidate;

/// How long one tool call may take.
pub const CALL_DEADLINE: Duration = Duration::from_secs(30);
/// `read`'s largest page.
const PAGE: u64 = 20;

/// One tool call to a claims session: the tool's text result, or why not.
pub trait Rpc {
    fn call(&mut self, tool: &str, args: Value) -> Result<String, CallError>;
}

/// Why a tool call gave no usable result. Only [`CallError::Lost`] may be
/// retried: the call's fate is unknown, and its op id makes a resend replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The session answered with an error (an `isError` result, such as a
    /// BBP rejection, or a JSON-RPC error). The call completed.
    Answered(String),
    /// No answer arrived: a timeout, an exit, a broken pipe or an
    /// unreadable line. The call may or may not have landed.
    Lost(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Answered(e) | Self::Lost(e) => f.write_str(e),
        }
    }
}

impl From<CallError> for String {
    fn from(e: CallError) -> Self {
        e.to_string()
    }
}

/// A `bbp mcp` session process for one lens.
pub struct Child {
    child: std::process::Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    next: u64,
    deadline: Duration,
}

impl Child {
    /// Start `bbp mcp` for `lens` on `task` in `dir` and initialise it.
    /// Inherited BBP, API and push credentials are removed first.
    pub fn spawn(bbp: &Path, dir: &Path, task: &str, lens: &str) -> Result<Self, String> {
        let mut cmd = Command::new(bbp);
        cmd.arg("mcp");
        for var in orch_bbp::SCRUB {
            cmd.env_remove(var);
        }
        let mut child = cmd
            .env("BBP_DIR", dir)
            .env("BBP_TASK", task)
            .env("BBP_PRINCIPAL", lens)
            .env("BBP_SESSION", "yes")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("bbp mcp for {lens}: {e}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(format!("bbp mcp for {lens}: no stdio"));
        };
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut s = Self {
            child,
            stdin,
            lines,
            next: 0,
            deadline: CALL_DEADLINE,
        };
        let init = map(vec![
            ("protocolVersion", "2025-06-18".into()),
            ("capabilities", map(vec![])),
            (
                "clientInfo",
                map(vec![("name", "rusty_orch".into()), ("version", "0".into())]),
            ),
        ]);
        s.request("initialize", init)?;
        s.send(&map(vec![
            ("jsonrpc", "2.0".into()),
            ("method", "notifications/initialized".into()),
        ]))?;
        Ok(s)
    }

    /// Bound each later call by `deadline` instead of [`CALL_DEADLINE`].
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    fn send(&mut self, msg: &Value) -> Result<(), CallError> {
        writeln!(self.stdin, "{msg}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| CallError::Lost(format!("bbp mcp: {e}")))
    }

    /// Send one request and wait for its response's `result`. The deadline
    /// is for the whole request: replies to other ids and notifications do
    /// not extend it.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        self.next += 1;
        let id = self.next;
        let until = Instant::now() + self.deadline;
        self.send(&map(vec![
            ("jsonrpc", "2.0".into()),
            ("id", id.into()),
            ("method", method.into()),
            ("params", params),
        ]))?;
        loop {
            let timeout = || {
                CallError::Lost(format!(
                    "bbp mcp: no answer to {method} in {}ms",
                    self.deadline.as_millis()
                ))
            };
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(timeout());
            }
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line.map_err(|e| CallError::Lost(format!("bbp mcp: {e}")))?,
                Err(RecvTimeoutError::Timeout) => return Err(timeout()),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(CallError::Lost("bbp mcp exited".to_owned()))
                }
            };
            let reply: Value = rusty_serde::json::from_str(&line)
                .map_err(|e| CallError::Lost(format!("bbp mcp sent bad JSON: {e}")))?;
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(e) = reply.get("error") {
                let msg = e.get("message").and_then(Value::as_str).unwrap_or("error");
                return Err(CallError::Answered(format!("bbp mcp {method}: {msg}")));
            }
            return reply
                .get("result")
                .cloned()
                .ok_or_else(|| CallError::Answered(format!("bbp mcp {method}: no result")));
        }
    }
}

impl Rpc for Child {
    fn call(&mut self, tool: &str, args: Value) -> Result<String, CallError> {
        let result = self.request(
            "tools/call",
            map(vec![("name", tool.into()), ("arguments", args)]),
        )?;
        tool_text(&result)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // Best effort: the session holds no state of its own to lose.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The text of a `tools/call` result; an `isError` result is an answered
/// error.
fn tool_text(result: &Value) -> Result<String, CallError> {
    let text = result
        .get("content")
        .and_then(|c| match c {
            Value::Seq(items) => items.first(),
            _ => None,
        })
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::Answered("result has no text".to_owned()))?
        .to_owned();
    match result.get("isError").and_then(Value::as_bool) {
        Some(true) => Err(CallError::Answered(text)),
        _ => Ok(text),
    }
}

/// What one lens sees of a claims task, and its way to post.
pub struct Session<R> {
    lens: String,
    rpc: R,
    /// The last message `read` returned.
    cursor: Option<MsgId>,
    claims: BTreeMap<MsgId, ClaimBody>,
    views: BTreeMap<MsgId, ClaimView>,
    /// The task revision the views are current to.
    rev: Rev,
    brief: Option<ArtId>,
    /// This run's id: with `posted`, it names each logical post.
    run: String,
    posted: u64,
}

/// A fresh run id, so a later run's posts never replay an earlier run's.
pub fn run_id() -> Result<String, String> {
    let bytes = rusty_rand::bytes(16).map_err(|e| format!("run id: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl<R: Rpc> Session<R> {
    /// A session for `lens` in run `run` ([`run_id`]).
    pub fn new(lens: &str, rpc: R, run: &str) -> Self {
        Self {
            run: run.to_owned(),
            posted: 0,
            lens: lens.to_owned(),
            rpc,
            cursor: None,
            claims: BTreeMap::new(),
            views: BTreeMap::new(),
            rev: Rev(0),
            brief: None,
        }
    }

    fn ask(&mut self, tool: &str, args: Value) -> Result<Response, String> {
        answer(self.rpc.call(tool, args), tool)
    }

    /// Bring the lens's view up to date: new messages (claim bodies), then
    /// the cards that changed since the last refresh.
    /// The revision is read first, so a change made meanwhile is seen again
    /// next time rather than missed.
    pub fn refresh(&mut self, config: &LensConfig) -> Result<(), String> {
        let rev = self.refresh_card()?;
        self.read_new()?;
        let domains = self.domains(config);
        if domains.is_empty() {
            self.rev = rev;
            return Ok(());
        }
        let args = map(vec![
            ("after", self.rev.0.into()),
            (
                "domains",
                Value::Seq(domains.into_iter().map(Value::from).collect()),
            ),
        ]);
        match self.ask("claims_delta", args)? {
            Response::Claims(views) => {
                self.views.extend(views.into_iter().map(|v| (v.id, v)));
            }
            other => return Err(format!("claims_delta answered {other:?}")),
        }
        self.rev = rev;
        Ok(())
    }

    /// Read every message after the cursor, a page at a time.
    fn read_new(&mut self) -> Result<(), String> {
        loop {
            let mut args = vec![("limit", PAGE.into())];
            if let Some(c) = self.cursor {
                args.push(("after", c.0.into()));
            }
            let Response::Read {
                messages,
                next,
                more,
            } = self.ask("read", map(args))?
            else {
                return Err("read answered something else".to_owned());
            };
            for m in messages {
                if let Body::Claim(b) = m.draft.body {
                    self.claims.insert(m.id, b);
                }
            }
            self.cursor = next.or(self.cursor);
            if !more {
                return Ok(());
            }
        }
    }

    /// The domains the lens follows; `*` is every domain seen so far. The
    /// alien lens follows every domain (D99).
    fn domains(&self, config: &LensConfig) -> BTreeSet<String> {
        let every = ["*".to_owned()];
        let follows: &[String] = match config.lens(&self.lens) {
            Some(lens) => &lens.domains,
            None if config.alien.as_ref().is_some_and(|a| a.id == self.lens) => &every,
            None => return BTreeSet::new(),
        };
        let seen = self.claims.values().flat_map(|b| b.domains.iter());
        seen.chain(follows)
            .filter(|d| d.as_str() != "*" && follows.iter().any(|f| f == "*" || f == *d))
            .cloned()
            .collect()
    }

    /// Every card the lens can see, by claim id. A claim whose body the
    /// lens has not read is left out.
    pub fn cards(&self, config: &LensConfig) -> Vec<Card> {
        self.views
            .values()
            .filter_map(|v| {
                let body = self.claims.get(&v.id)?;
                Some(Card::from_view(v, body, config))
            })
            .collect()
    }

    /// The task's brief as of the last refresh.
    pub fn brief_id(&self) -> Option<ArtId> {
        self.brief
    }

    /// The task brief's text: the intent every lens prefix starts with.
    /// Charged once per call (R11), so the shell reads it once per run.
    pub fn brief(&mut self) -> Result<String, String> {
        if self.brief.is_none() {
            self.refresh_card()?;
        }
        let id = self.brief.ok_or("the task has no brief")?;
        let text = self
            .rpc
            .call("get_artifact", map(vec![("id", id.0.into())]))?;
        let v: Value = rusty_serde::json::from_str(&text)
            .map_err(|e| format!("get_artifact: unreadable answer: {e}"))?;
        v.get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("get_artifact: brief {id} has no text"))
    }

    /// Code candidate `id`'s base and the text of each of its diffs, in the
    /// candidate's order, so each can be applied on its own.
    /// Each artifact's first read is charged once to the lens (R11), not
    /// to the claim's budget (D98). A role that may not read the candidate
    /// or a diff gives [`Diffs::Forbidden`]; any other failure is an error.
    pub fn candidate_diffs(&mut self, id: ArtId) -> Result<Diffs, String> {
        let (rec, _) = match self.artifact(id)? {
            Got::Read(rec, content) => (rec, content),
            Got::Forbidden(why) => return Ok(Diffs::Forbidden(why)),
        };
        let ArtifactPayload::Candidate(candidate) = rec.payload else {
            return Err(format!("get_artifact: {id} is not a candidate"));
        };
        let mut texts = Vec::with_capacity(candidate.diffs.len());
        for diff in candidate.diffs {
            let (rec, content) = match self.artifact(diff)? {
                Got::Read(rec, content) => (rec, content),
                Got::Forbidden(why) => return Ok(Diffs::Forbidden(why)),
            };
            if rec.kind != ArtifactKind::Diff {
                return Err(format!(
                    "get_artifact: {diff} of candidate {id} is not a diff"
                ));
            }
            texts.push(content);
        }
        Ok(Diffs::Read(Candidate {
            base: candidate.base,
            diffs: texts,
        }))
    }

    /// One artifact's record and text; a `role_forbidden` refusal is
    /// [`Got::Forbidden`].
    fn artifact(&mut self, id: ArtId) -> Result<Got, String> {
        const TOOL: &str = "get_artifact";
        let reply = self.rpc.call(TOOL, map(vec![("id", id.0.into())]));
        if let Err(CallError::Answered(text)) = &reply {
            if let Some(r) = forbidden(text) {
                return Ok(Got::Forbidden(rejected(&format!("{TOOL} {id}"), &r)));
            }
        }
        let text = reply.map_err(|e| failed(e, TOOL))?;
        let v: Value = rusty_serde::json::from_str(&text)
            .map_err(|e| format!("{TOOL}: unreadable answer: {e}"))?;
        // The host puts the text (and version fields) beside the response,
        // so the response is read back on its own.
        let rec = v
            .get("Artifact")
            .cloned()
            .ok_or_else(|| format!("{TOOL}: {id} answered no artifact"))?;
        let only = Value::Map(vec![("Artifact".to_owned(), rec)]).to_string();
        let Response::Artifact(rec) = parse(&only, TOOL)? else {
            return Err(format!("{TOOL}: {id} answered no artifact"));
        };
        let content = v
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{TOOL}: {id} has no text"))?;
        Ok(Got::Read(Box::new(rec), content.to_owned()))
    }

    fn refresh_card(&mut self) -> Result<Rev, String> {
        match self.ask("task_card", map(vec![]))? {
            Response::Card(c) => {
                self.brief = c.brief;
                Ok(c.rev)
            }
            other => Err(format!("task_card answered {other:?}")),
        }
    }

    /// One line per settled claim, newest first, at most [`MAX_LINES`].
    pub fn lines(&mut self, config: &LensConfig) -> Result<Vec<Line>, String> {
        let settled: Vec<(MsgId, Vec<String>)> = self
            .views
            .values()
            .rev()
            .filter(|v| v.status != ClaimStatus::Open)
            .take(MAX_LINES)
            .map(|v| (v.id, v.domains.clone()))
            .collect();
        let mut out = Vec::new();
        for (id, domains) in settled {
            let Response::Line(line) = self.ask("claim_line", map(vec![("claim", id.0.into())]))?
            else {
                return Err("claim_line answered something else".to_owned());
            };
            let class = domains
                .iter()
                .map(|d| config.domain_class(d))
                .max()
                .unwrap_or(orch_lens::DataClass::Internal);
            out.extend(line.map(|text| Line { text, class }));
        }
        Ok(out)
    }
}

impl<R: Rpc> Board for Session<R> {
    /// Each post is its own logical post with its own op id. A call whose
    /// answer was lost is sent once more with the same op, so BBP replays
    /// it if the first one landed; an answered error, such as a
    /// rejection, is not retried.
    fn post(&mut self, lens: &str, posts: &[Post]) -> Result<(), String> {
        if lens != self.lens {
            return Err(format!("{}'s session cannot post for {lens}", self.lens));
        }
        for p in posts {
            self.posted += 1;
            let op = format!("lens-{}-{}", self.run, self.posted);
            let args = post_args(p, self.brief, &op);
            let reply = match self.rpc.call("post", args.clone()) {
                Err(CallError::Lost(_)) => self.rpc.call("post", args),
                first => first,
            };
            match answer(reply, "post")? {
                Response::Posted(_) => {}
                other => return Err(format!("post answered {other:?}")),
            }
        }
        Ok(())
    }
}

/// What a lens may read of a claim's code candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diffs {
    /// The candidate's base and each diff's text, in the candidate's order.
    Read(Candidate),
    /// The lens's role may not read the candidate or one of its diffs
    /// (D98): the claim is not eligible. The refused read itself is not
    /// charged; reads that succeeded before it (the candidate, earlier
    /// diffs) already were.
    Forbidden(String),
}

/// One `get_artifact` answer.
enum Got {
    Read(Box<ArtifactRecord>, String),
    Forbidden(String),
}

/// A call's outcome as a response; a rejection or any other error is an
/// error naming `tool`.
fn answer(reply: Result<String, CallError>, tool: &str) -> Result<Response, String> {
    reply
        .map_err(|e| failed(e, tool))
        .and_then(|text| parse(&text, tool))
}

/// A failed call as an error naming `tool`. An answered error's text is
/// shown as a rejection when it is one (the host's `isError` shape), else
/// as sent.
fn failed(e: CallError, tool: &str) -> String {
    match e {
        CallError::Answered(text) => match rusty_serde::json::from_str(&text) {
            Ok(Response::Rejected(r)) => rejected(tool, &r),
            _ => format!("{tool}: {text}"),
        },
        CallError::Lost(e) => format!("{tool}: {e}"),
    }
}

/// BBP's `role_forbidden` refusal, if this answered error is one.
fn forbidden(text: &str) -> Option<rusty_bbp::Rejection> {
    match rusty_serde::json::from_str(text) {
        Ok(Response::Rejected(r)) if r.code == Code::RoleForbidden => Some(r),
        _ => None,
    }
}

/// A tool's answer; a rejection is an error.
fn parse(text: &str, tool: &str) -> Result<Response, String> {
    let resp: Response =
        rusty_serde::json::from_str(text).map_err(|e| format!("{tool}: unreadable answer: {e}"))?;
    match resp {
        Response::Rejected(r) => Err(rejected(tool, &r)),
        other => Ok(other),
    }
}

fn rejected(tool: &str, r: &rusty_bbp::Rejection) -> String {
    format!("{tool}: {:?}: {}", r.code, r.detail)
}

/// The `post` tool's arguments for `p`, under op id `op`. Every post refs
/// the brief.
fn post_args(p: &Post, brief: Option<ArtId>, op: &str) -> Value {
    let brief_ref = brief.map(Ref::art);
    let mut fields: Vec<(&str, Value)> = match p {
        Post::Claim { body, refs } => {
            let mut v = vec![
                ("kind", "claim".into()),
                ("text", body.text.as_str().into()),
                ("domains", strings(&body.domains)),
                ("stakes", stakes(body.stakes).into()),
                ("reversible", Value::Bool(body.reversible)),
                ("deps", ids(&body.deps)),
                ("refs", refs_of(refs, brief_ref.as_ref())),
            ];
            if let Some(a) = body.answers {
                v.push(("answers", a.0.into()));
            }
            if let Some(s) = body.supersedes {
                v.push(("supersedes", s.0.into()));
            }
            v
        }
        Post::Evidence { body, evidence } => vec![
            ("kind", "evidence".into()),
            ("claim", body.claim.0.into()),
            ("direction", direction(body.direction).into()),
            ("evidence_kind", kind(body.kind).into()),
            ("strength", u64::from(body.strength).into()),
            ("note", body.note.as_str().into()),
            ("evidence", refs_of(evidence, None)),
            ("refs", refs_of(&[], brief_ref.as_ref())),
        ],
    };
    fields.insert(0, ("op", op.into()));
    map(fields)
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn strings(items: &[String]) -> Value {
    Value::Seq(items.iter().map(|s| s.as_str().into()).collect())
}

fn ids(items: &[MsgId]) -> Value {
    Value::Seq(items.iter().map(|m| m.0.into()).collect())
}

fn refs_of(refs: &[Ref], extra: Option<&Ref>) -> Value {
    let mut all: Vec<&Ref> = refs.iter().collect();
    if let Some(e) = extra.filter(|e| !refs.contains(e)) {
        all.push(e);
    }
    Value::Seq(all.into_iter().map(|r| ref_word(r).into()).collect())
}

fn ref_word(r: &Ref) -> String {
    match r {
        Ref::Art { id, fragment: None } => format!("art:{}", id.0),
        Ref::Art {
            id,
            fragment: Some(f),
        } => format!("art:{}#{f}", id.0),
        Ref::Msg(m) => format!("msg:{}", m.0),
    }
}

fn stakes(s: Stakes) -> &'static str {
    match s {
        Stakes::Low => "low",
        Stakes::Medium => "medium",
        Stakes::High => "high",
    }
}

fn direction(d: Direction) -> &'static str {
    match d {
        Direction::Support => "support",
        Direction::Counter => "counter",
    }
}

fn kind(k: EvidenceKind) -> &'static str {
    match k {
        EvidenceKind::Executed => "executed",
        EvidenceKind::Cited => "cited",
        EvidenceKind::Opinion => "opinion",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rusty_bbp::claims::EvidenceView;
    use rusty_bbp::{
        Budget, Draft, EvidenceBody, Message, MessageKind, PrincipalId, Recipient, State,
    };

    use super::*;

    #[derive(Default)]
    struct Fake {
        replies: BTreeMap<String, VecDeque<Result<String, CallError>>>,
        calls: Vec<(String, Value)>,
    }

    impl Fake {
        fn on(mut self, tool: &str, resp: Response) -> Self {
            let text = rusty_serde::json::to_string(&resp).expect("encode");
            self.replies
                .entry(tool.to_owned())
                .or_default()
                .push_back(Ok(text));
            self
        }

        /// The answer is lost: a timeout or an exit.
        fn fail(mut self, tool: &str, error: &str) -> Self {
            self.replies
                .entry(tool.to_owned())
                .or_default()
                .push_back(Err(CallError::Lost(error.to_owned())));
            self
        }

        /// The host refuses, as `bbp mcp` does: an `isError` result whose
        /// text is the rejection.
        fn refuse(mut self, tool: &str, rejection: rusty_bbp::Rejection) -> Self {
            let text =
                rusty_serde::json::to_string(&Response::Rejected(rejection)).expect("encode");
            self.replies
                .entry(tool.to_owned())
                .or_default()
                .push_back(Err(CallError::Answered(text)));
            self
        }

        fn args(&self, tool: &str) -> Vec<&Value> {
            self.calls
                .iter()
                .filter(|(t, _)| t == tool)
                .map(|(_, a)| a)
                .collect()
        }
    }

    impl Rpc for Fake {
        fn call(&mut self, tool: &str, args: Value) -> Result<String, CallError> {
            self.calls.push((tool.to_owned(), args));
            self.replies
                .get_mut(tool)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Err(CallError::Answered(format!("no reply for {tool}"))))
        }
    }

    const CONFIG: &str = r#"{
      "providers": {
        "codex": {"kind": "codex-cli", "context_tokens": 32768,
                  "price": {"input": 0, "output": 0, "cached_input": 0}}
      },
      "ceilings": {"experiment_usd": 0, "task_usd": 0, "task_calls": 10},
      "lenses": [
        {"id": "adversary", "prompt": "p", "domains": ["*"],
         "may_post": {"evidence": ["counter"]},
         "models": {"routine": ["codex"], "contested": ["codex"]}}
      ]
    }"#;

    fn config() -> LensConfig {
        LensConfig::parse(CONFIG).expect("config")
    }

    fn task_card(rev: u64) -> Response {
        let budget = Budget {
            messages: 100,
            bytes: 100_000,
            reads: 100,
            reads_per_turn: 10,
            turns: 10,
            iterations: 3,
        };
        Response::Card(rusty_bbp::Card {
            task: rusty_bbp::TaskId("t".to_owned()),
            rev: Rev(rev),
            state: State::Open,
            turns: vec![],
            iteration: 0,
            brief: Some(ArtId(1)),
            spec: None,
            candidate: None,
            run: None,
            pending_request: None,
            budget,
            spent: rusty_bbp::state::Spent::default(),
            rejections: 0,
        })
    }

    fn msg(id: u64, kind: MessageKind, body: Body) -> Message {
        Message {
            id: MsgId(id),
            from: PrincipalId("architect".to_owned()),
            from_role: None,
            draft: Draft {
                kind,
                to: vec![Recipient::All],
                body,
                refs: vec![],
                evidence: vec![],
                reply_to: None,
                yield_to: None,
                gate: false,
            },
        }
    }

    fn claim(id: u64) -> Message {
        msg(
            id,
            MessageKind::Claim,
            Body::Claim(ClaimBody {
                text: "Hash by size first".to_owned(),
                domains: vec!["dedup".to_owned()],
                deps: vec![],
                answers: None,
                supersedes: None,
                stakes: Stakes::Medium,
                reversible: true,
            }),
        )
    }

    fn evidence(id: u64, claim: u64, note: &str) -> Message {
        msg(
            id,
            MessageKind::Evidence,
            Body::Evidence(EvidenceBody {
                claim: MsgId(claim),
                direction: Direction::Counter,
                kind: EvidenceKind::Cited,
                strength: 2,
                note: note.to_owned(),
            }),
        )
    }

    fn read(messages: Vec<Message>, more: bool) -> Response {
        let next = messages.last().map(|m| m.id);
        Response::Read {
            messages,
            next,
            more,
        }
    }

    fn view(id: u64, status: ClaimStatus, evidence: &[u64]) -> ClaimView {
        ClaimView {
            id: MsgId(id),
            text: "Hash by size first".to_owned(),
            domains: vec!["dedup".to_owned()],
            status,
            score: -2,
            accept_at: 8,
            confidence: 40,
            epoch: 0,
            spent: 2,
            budget: 16,
            evidence: evidence
                .iter()
                .map(|m| EvidenceView {
                    msg: MsgId(*m),
                    author: PrincipalId("operator".to_owned()),
                    direction: Direction::Counter,
                    kind: EvidenceKind::Cited,
                    strength: 2,
                    note: format!("the view's note on {m}"),
                    delta: -2,
                    epoch: 0,
                    scoring: true,
                })
                .collect(),
            deps: vec![],
            candidates: vec![],
            sole_candidate: false,
            current: None,
            failing: vec![],
            tests_changed: vec![],
            endorsement: None,
            holdout_due: false,
            intent: None,
            human_touched: false,
        }
    }

    fn word(v: &Value, k: &str) -> String {
        v.get(k).map(Value::to_string).unwrap_or_default()
    }

    #[test]
    fn the_alien_lens_follows_every_domain_and_an_unknown_role_none() {
        let at = CONFIG.rfind(']').expect("lenses end");
        let text = format!(
            "{}],\n\"alien\": {{\"id\": \"alien\", \"prompt\": \"a\", \"providers\": [\"codex\"]}}}}",
            &CONFIG[..at]
        );
        let cfg = LensConfig::parse(&text).expect("config");
        for (role, deltas) in [("alien", 1), ("stranger", 0)] {
            let fake = Fake::default()
                .on("task_card", task_card(5))
                .on("read", read(vec![claim(7)], false))
                .on(
                    "claims_delta",
                    Response::Claims(vec![view(7, ClaimStatus::Open, &[])]),
                );
            let mut s = Session::new(role, fake, "r1");
            s.refresh(&cfg).expect("refresh");
            let asked = s.rpc.args("claims_delta");
            assert_eq!(asked.len(), deltas, "{role}");
            if let Some(delta) = asked.first() {
                assert_eq!(word(delta, "domains"), r#"["dedup"]"#);
                assert_eq!(s.cards(&cfg).len(), 1);
            }
        }
    }

    #[test]
    fn refresh_reads_new_pages_then_the_delta_and_builds_cards() {
        let fake = Fake::default()
            .on("task_card", task_card(5))
            .on(
                "read",
                read(
                    vec![claim(7), evidence(20, 7, "fails on empty files")],
                    true,
                ),
            )
            .on("read", read(vec![], false))
            .on(
                "claims_delta",
                Response::Claims(vec![view(7, ClaimStatus::Open, &[20])]),
            );
        let cfg = config();
        let mut s = Session::new("adversary", fake, "r1");
        s.refresh(&cfg).expect("refresh");
        let cards = s.cards(&cfg);
        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].evidence[0].note, "the view's note on 20",
            "notes come with the card"
        );
        assert_eq!(cards[0].accept_at, 8);
        assert_eq!(cards[0].stakes, Stakes::Medium);
        let reads = s.rpc.args("read");
        assert_eq!(word(reads[0], "after"), "");
        assert_eq!(word(reads[1], "after"), "20");
        let delta = s.rpc.args("claims_delta")[0].clone();
        assert_eq!(word(&delta, "after"), "0");
        assert_eq!(
            word(&delta, "domains"),
            r#"["dedup"]"#,
            "* is every domain seen"
        );
    }

    #[test]
    fn a_second_refresh_asks_only_for_changes_and_keeps_the_rest() {
        let fake = Fake::default()
            .on("task_card", task_card(5))
            .on("read", read(vec![claim(7), claim(8)], false))
            .on(
                "claims_delta",
                Response::Claims(vec![
                    view(7, ClaimStatus::Open, &[]),
                    view(8, ClaimStatus::Open, &[]),
                ]),
            )
            .on("task_card", task_card(9))
            .on("read", read(vec![], false))
            .on(
                "claims_delta",
                Response::Claims(vec![view(8, ClaimStatus::Accepted, &[])]),
            );
        let cfg = config();
        let mut s = Session::new("adversary", fake, "r1");
        s.refresh(&cfg).expect("first");
        s.refresh(&cfg).expect("second");
        assert_eq!(word(s.rpc.args("claims_delta")[1], "after"), "5");
        assert_eq!(word(s.rpc.args("read")[1], "after"), "8");
        let cards = s.cards(&cfg);
        let status: Vec<ClaimStatus> = cards.iter().map(|c| c.status).collect();
        assert_eq!(status, [ClaimStatus::Open, ClaimStatus::Accepted]);
    }

    #[test]
    fn a_claim_whose_body_was_not_read_is_left_out() {
        let fake = Fake::default()
            .on("task_card", task_card(5))
            .on("read", read(vec![claim(7)], false))
            .on(
                "claims_delta",
                Response::Claims(vec![
                    view(7, ClaimStatus::Open, &[]),
                    view(9, ClaimStatus::Open, &[]),
                ]),
            );
        let cfg = config();
        let mut s = Session::new("adversary", fake, "r1");
        s.refresh(&cfg).expect("refresh");
        let ids: Vec<MsgId> = s.cards(&cfg).iter().map(|c| c.id).collect();
        assert_eq!(ids, [MsgId(7)]);
    }

    #[test]
    fn a_rejection_is_an_error() {
        let rej = Response::Rejected(rusty_bbp::Rejection::new(
            rusty_bbp::Code::BadCredential,
            "reassigned",
        ));
        let mut s = Session::new("adversary", Fake::default().on("task_card", rej), "r1");
        let err = s.refresh(&config()).expect_err("rejected");
        assert!(err.contains("reassigned"), "{err}");
    }

    #[test]
    fn lines_are_settled_claims_newest_first() {
        let fake = Fake::default()
            .on("task_card", task_card(5))
            .on("read", read(vec![claim(7), claim(8), claim(9)], false))
            .on(
                "claims_delta",
                Response::Claims(vec![
                    view(7, ClaimStatus::Accepted, &[]),
                    view(8, ClaimStatus::Open, &[]),
                    view(9, ClaimStatus::Dropped, &[]),
                ]),
            )
            .on(
                "claim_line",
                Response::Line(Some("msg:9 DROPPED".to_owned())),
            )
            .on(
                "claim_line",
                Response::Line(Some("msg:7 ACCEPTED".to_owned())),
            );
        let cfg = config();
        let mut s = Session::new("adversary", fake, "r1");
        s.refresh(&cfg).expect("refresh");
        let lines = s.lines(&cfg).expect("lines");
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text, ["msg:9 DROPPED", "msg:7 ACCEPTED"]);
        let asked: Vec<String> = s
            .rpc
            .args("claim_line")
            .iter()
            .map(|a| word(a, "claim"))
            .collect();
        assert_eq!(asked, ["9", "7"]);
    }

    fn counter(note: &str) -> Post {
        Post::Evidence {
            body: EvidenceBody {
                claim: MsgId(7),
                direction: Direction::Counter,
                kind: EvidenceKind::Cited,
                strength: 2,
                note: note.to_owned(),
            },
            evidence: vec![Ref::Msg(MsgId(3))],
        }
    }

    #[test]
    fn a_post_refs_the_brief_and_carries_its_op() {
        let a = post_args(&counter("x"), Some(ArtId(1)), "lens-r1-1");
        assert_eq!(word(&a, "op"), r#""lens-r1-1""#);
        assert_eq!(word(&a, "refs"), r#"["art:1"]"#);
        assert_eq!(word(&a, "evidence"), r#"["msg:3"]"#);
        assert_eq!(word(&a, "direction"), r#""counter""#);
        let claim = Post::Claim {
            body: ClaimBody {
                text: "Use a size index".to_owned(),
                domains: vec!["dedup".to_owned()],
                deps: vec![MsgId(4)],
                answers: Some(MsgId(3)),
                supersedes: None,
                stakes: Stakes::High,
                reversible: false,
            },
            refs: vec![Ref::art(ArtId(1))],
        };
        let c = post_args(&claim, Some(ArtId(1)), "lens-r1-2");
        assert_eq!(word(&c, "refs"), r#"["art:1"]"#, "the brief once");
        assert_eq!(word(&c, "answers"), "3");
        assert_eq!(word(&c, "stakes"), r#""high""#);
        assert_eq!(word(&c, "reversible"), "false");
        assert!(c.get("supersedes").is_none());
    }

    fn ops(s: &Session<Fake>) -> Vec<String> {
        s.rpc.args("post").iter().map(|a| word(a, "op")).collect()
    }

    #[test]
    fn identical_posts_from_independent_jobs_get_their_own_ops() {
        let fake = Fake::default()
            .on("post", Response::Posted(MsgId(30)))
            .on("post", Response::Posted(MsgId(31)))
            .on("post", Response::Posted(MsgId(32)));
        let mut s = Session::new("architect", fake, "r1");
        s.post("architect", &[counter("same")]).expect("first job");
        s.post("architect", &[counter("same"), counter("same")])
            .expect("second job, two identical posts");
        assert_eq!(
            ops(&s),
            [r#""lens-r1-1""#, r#""lens-r1-2""#, r#""lens-r1-3""#]
        );
        let other_run = Fake::default().on("post", Response::Posted(MsgId(40)));
        let mut t = Session::new("architect", other_run, "r2");
        t.post("architect", &[counter("same")])
            .expect("a later run");
        assert_eq!(ops(&t), [r#""lens-r2-1""#], "a later run never replays");
    }

    #[test]
    fn a_post_lost_in_transport_is_retried_with_the_same_op() {
        let fake = Fake::default()
            .fail("post", "bbp mcp: no answer to tools/call in 30000ms")
            .on("post", Response::Posted(MsgId(30)));
        let mut s = Session::new("adversary", fake, "r1");
        s.post("adversary", &[counter("x")]).expect("replayed");
        assert_eq!(ops(&s), [r#""lens-r1-1""#, r#""lens-r1-1""#]);
        let posts = s.rpc.args("post");
        assert_eq!(posts[0], posts[1], "the retry is the same call");
    }

    #[test]
    fn a_rejected_post_is_not_retried() {
        let fake = Fake::default().refuse("post", reassigned());
        let mut s = Session::new("adversary", fake, "r1");
        let err = s.post("adversary", &[counter("x")]).expect_err("rejected");
        assert!(err.contains("reassigned"), "{err}");
        assert_eq!(s.rpc.args("post").len(), 1);
    }

    #[test]
    fn the_brief_is_read_once_from_its_artifact() {
        let mut fake = Fake::default().on("task_card", task_card(5));
        fake.replies
            .entry("get_artifact".to_owned())
            .or_default()
            .push_back(Ok(
                r#"{"id":1,"content":"Dedup 20 TB of photos."}"#.to_owned()
            ));
        let mut s = Session::new("adversary", fake, "r1");
        assert_eq!(s.brief(), Ok("Dedup 20 TB of photos.".to_owned()));
        assert_eq!(word(s.rpc.args("get_artifact")[0], "id"), "1");
    }

    /// A `get_artifact` answer as the host renders it: the response with
    /// the artifact's text beside it.
    fn artifact(id: u64, payload: ArtifactPayload, content: &str) -> Result<String, CallError> {
        let rec = ArtifactRecord {
            id: ArtId(id),
            author: PrincipalId("builder".to_owned()),
            kind: payload.kind(),
            blob: rusty_bbp::BlobRef {
                sha: rusty_bbp::Sha256::of(content.as_bytes()),
                len: content.len() as u64,
            },
            payload,
            spec: None,
            candidate: None,
            run: None,
        };
        let text = rusty_serde::json::to_string(&Response::Artifact(rec)).expect("encode");
        let Ok(Value::Map(mut m)) = rusty_serde::json::from_str::<Value>(&text) else {
            panic!("not a map: {text}");
        };
        m.push(("content".to_owned(), content.into()));
        Ok(Value::Map(m).to_string())
    }

    fn candidate(diffs: &[u64]) -> ArtifactPayload {
        ArtifactPayload::Candidate(rusty_bbp::Candidate {
            base: "0".repeat(40),
            diffs: diffs.iter().copied().map(ArtId).collect(),
            claim: Some(MsgId(10)),
            suite: rusty_bbp::Suite::Build,
        })
    }

    fn reading(answers: Vec<Result<String, CallError>>) -> Session<Fake> {
        let mut fake = Fake::default();
        fake.replies
            .entry("get_artifact".to_owned())
            .or_default()
            .extend(answers);
        Session::new("adversary", fake, "r1")
    }

    fn forbid() -> Result<String, CallError> {
        let r = rusty_bbp::Rejection::new(Code::RoleForbidden, "role lacks read(diff)");
        let text = rusty_serde::json::to_string(&Response::Rejected(r)).expect("encode");
        Err(CallError::Answered(text))
    }

    #[test]
    fn a_candidates_diffs_are_read_in_order() {
        let mut s = reading(vec![
            artifact(5, candidate(&[3, 4]), "{}"),
            artifact(3, ArtifactPayload::Diff, "--- a/src/x.rs\n+++ b/src/x.rs"),
            artifact(
                4,
                ArtifactPayload::Diff,
                "--- a/README.md\n+++ b/README.md\n",
            ),
        ]);
        let want = [
            "--- a/src/x.rs\n+++ b/src/x.rs",
            "--- a/README.md\n+++ b/README.md\n",
        ];
        assert_eq!(
            s.candidate_diffs(ArtId(5)),
            Ok(Diffs::Read(Candidate {
                base: "0".repeat(40),
                diffs: want.map(str::to_owned).to_vec(),
            }))
        );
        let ids: Vec<String> = s
            .rpc
            .args("get_artifact")
            .iter()
            .map(|a| word(a, "id"))
            .collect();
        assert_eq!(ids, ["5", "3", "4"]);
    }

    #[test]
    fn a_refused_read_is_forbidden_and_stops() {
        let mut s = reading(vec![forbid()]);
        let Ok(Diffs::Forbidden(why)) = s.candidate_diffs(ArtId(5)) else {
            panic!("not forbidden");
        };
        assert!(why.contains("read(diff)"), "{why}");
        assert_eq!(s.rpc.args("get_artifact").len(), 1);

        let mut s = reading(vec![artifact(5, candidate(&[3, 4]), "{}"), forbid()]);
        assert!(matches!(
            s.candidate_diffs(ArtId(5)),
            Ok(Diffs::Forbidden(_))
        ));
        assert_eq!(
            s.rpc.args("get_artifact").len(),
            2,
            "the second diff is not read"
        );
    }

    #[test]
    fn other_failures_reading_a_candidate_are_errors() {
        let refused = rusty_bbp::Rejection::new(Code::RefUnresolved, "art:5");
        let text = rusty_serde::json::to_string(&Response::Rejected(refused)).expect("encode");
        let cases: Vec<(Vec<Result<String, CallError>>, &str)> = vec![
            (vec![Err(CallError::Answered(text))], "RefUnresolved"),
            (
                vec![Err(CallError::Lost("timed out".to_owned()))],
                "timed out",
            ),
            (
                vec![artifact(5, ArtifactPayload::Diff, "x")],
                "not a candidate",
            ),
            (
                vec![
                    artifact(5, candidate(&[3]), "{}"),
                    artifact(3, ArtifactPayload::Log, "x"),
                ],
                "not a diff",
            ),
            (
                vec![Ok(r#"{"id":5,"content":"x"}"#.to_owned())],
                "no artifact",
            ),
        ];
        for (answers, want) in cases {
            let err = reading(answers).candidate_diffs(ArtId(5)).expect_err(want);
            assert!(err.contains(want), "{err} lacks {want}");
        }
    }

    #[test]
    fn a_session_posts_only_for_its_own_lens() {
        let fake = Fake::default().on("post", Response::Posted(MsgId(30)));
        let mut s = Session::new("adversary", fake, "r1");
        assert!(s.post("operator", &[counter("x")]).is_err());
        assert!(s.rpc.calls.is_empty());
        s.post("adversary", &[counter("x")]).expect("posted");
        assert_eq!(s.rpc.args("post").len(), 1);
    }

    #[test]
    fn run_ids_are_fresh() {
        let a = run_id().expect("id");
        assert_eq!(a.len(), 32);
        assert_ne!(a, run_id().expect("id"));
    }

    #[test]
    fn a_tool_error_result_is_an_error() {
        let ok = map(vec![(
            "content",
            Value::Seq(vec![map(vec![
                ("type", "text".into()),
                ("text", "{}".into()),
            ])]),
        )]);
        assert_eq!(tool_text(&ok), Ok("{}".to_owned()));
        let bad = map(vec![
            (
                "content",
                Value::Seq(vec![map(vec![
                    ("type", "text".into()),
                    ("text", "no".into()),
                ])]),
            ),
            ("isError", Value::Bool(true)),
        ]);
        assert_eq!(tool_text(&bad), Err(CallError::Answered("no".to_owned())));
        assert!(tool_text(&map(vec![])).is_err());
    }

    /// A stand-in `bbp mcp`: answers initialize, then one tool call with
    /// a stray id first, then exits.
    #[cfg(unix)]
    const SCRIPT: &str = r#"#!/bin/sh
read -r l
printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'
read -r l
read -r l
printf '{"jsonrpc":"2.0","id":99,"result":{}}\n'
printf '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"%s %s %s"}]}}\n' "$BBP_PRINCIPAL" "$BBP_SESSION" "${GH_TOKEN:-scrubbed}"
"#;

    /// A stand-in that answers initialize, then sends replies to other ids
    /// forever, faster than any deadline.
    #[cfg(unix)]
    const CHATTY: &str = r#"#!/bin/sh
read -r l
printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'
while :; do
  printf '{"jsonrpc":"2.0","id":99,"result":{}}\n'
  printf '{"jsonrpc":"2.0","method":"notifications/progress"}\n'
  sleep 0.02
done
"#;

    /// Spawn the stand-in. Another test thread that forks while
    /// [`stand_in`] holds the script open for writing keeps that descriptor
    /// until its own exec, so exec here can briefly fail with ETXTBSY; that
    /// is retried, nothing else is.
    #[cfg(unix)]
    fn spawn(bin: &std::path::Path, dir: &std::path::Path) -> Result<Child, String> {
        let mut tries = 0;
        loop {
            match Child::spawn(bin, dir, "t", "adversary") {
                Err(e) if e.contains("Text file busy") && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }

    /// Write `script` as an executable stand-in `bbp` in a fresh dir.
    #[cfg(unix)]
    fn stand_in(name: &str, script: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("orch-bbp-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let bin = dir.join("bbp");
        std::fs::write(&bin, script).expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        dir
    }

    #[cfg(unix)]
    #[test]
    fn unrelated_replies_do_not_extend_the_deadline() {
        let dir = stand_in("chatty", CHATTY);
        let mut child = spawn(&dir.join("bbp"), &dir)
            .expect("spawn")
            .with_deadline(Duration::from_millis(300));
        let started = Instant::now();
        let err = child.call("task_card", map(vec![])).expect_err("times out");
        let took = started.elapsed();
        assert!(
            matches!(&err, CallError::Lost(e) if e.contains("no answer")),
            "{err}"
        );
        assert!(took >= Duration::from_millis(300), "{took:?}");
        assert!(took < Duration::from_secs(5), "bounded: {took:?}");
        drop(child);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_session_answers_by_id_and_reports_its_exit() {
        let dir = stand_in("once", SCRIPT);
        let bin = dir.join("bbp");
        let mut child = spawn(&bin, &dir).expect("spawn");
        assert_eq!(
            child.call("task_card", map(vec![])),
            Ok("adversary yes scrubbed".to_owned())
        );
        let gone = child
            .call("task_card", map(vec![]))
            .expect_err("exited")
            .to_string();
        assert!(
            gone.contains("exited") || gone.contains("Broken pipe"),
            "{gone}"
        );
        drop(child);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    fn reassigned() -> rusty_bbp::Rejection {
        rusty_bbp::Rejection::new(rusty_bbp::Code::BadCredential, "reassigned")
    }

    /// A stand-in `bbp mcp` that logs each tool call to `calls` beside
    /// it: it answers the first call with `first` (or never, if `None`),
    /// and the second with `second`.
    #[cfg(unix)]
    fn logging_script(first: Option<Value>, second: Value) -> String {
        let reply = |id: u64, result: Value| {
            map(vec![
                ("jsonrpc", "2.0".into()),
                ("id", id.into()),
                ("result", result),
            ])
            .to_string()
        };
        let first = first.map_or(String::new(), |r| {
            format!("printf '%s\\n' '{}'\n", reply(2, r))
        });
        let next_id = if first.is_empty() { 3 } else { 2 };
        format!(
            "#!/bin/sh\nd=$(dirname \"$0\")\nread -r l\nprintf '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}\\n'\nread -r l\nread -r l\nprintf '%s\\n' \"$l\" >> \"$d/calls\"\n{first}read -r l\nprintf '%s\\n' \"$l\" >> \"$d/calls\"\nprintf '%s\\n' '{}'\n",
            reply(next_id, second)
        )
    }

    /// A `tools/call` result carrying `resp`, rendered as the host does.
    #[cfg(unix)]
    fn rendered(resp: &Response) -> Value {
        let text = rusty_serde::json::to_string(resp).expect("encode");
        map(vec![
            (
                "content",
                Value::Seq(vec![map(vec![
                    ("type", "text".into()),
                    ("text", text.as_str().into()),
                ])]),
            ),
            (
                "isError",
                Value::Bool(matches!(resp, Response::Rejected(_))),
            ),
        ])
    }

    /// The `post` calls the stand-in in `dir` logged, as their arguments.
    #[cfg(unix)]
    fn logged_posts(dir: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(|l| rusty_serde::json::from_str::<Value>(l).expect("logged call"))
            .filter_map(|v| v.get("params").and_then(|p| p.get("arguments")).cloned())
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn a_hosts_rejection_through_the_child_is_sent_once() {
        let posted = rendered(&Response::Posted(MsgId(30)));
        let script = logging_script(Some(rendered(&Response::Rejected(reassigned()))), posted);
        let dir = stand_in("refused", &script);
        let child = spawn(&dir.join("bbp"), &dir)
            .expect("spawn")
            .with_deadline(Duration::from_millis(300));
        let mut s = Session::new("adversary", child, "r1");
        let err = s.post("adversary", &[counter("x")]).expect_err("rejected");
        assert!(err.contains("reassigned"), "{err}");
        drop(s);
        assert_eq!(logged_posts(&dir).len(), 1, "a rejection is final");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn a_lost_answer_through_the_child_is_resent_with_the_same_op() {
        let script = logging_script(None, rendered(&Response::Posted(MsgId(30))));
        let dir = stand_in("lost", &script);
        let child = spawn(&dir.join("bbp"), &dir)
            .expect("spawn")
            .with_deadline(Duration::from_millis(300));
        let mut s = Session::new("adversary", child, "r1");
        s.post("adversary", &[counter("x")]).expect("replayed");
        drop(s);
        let posts = logged_posts(&dir);
        assert_eq!(posts.len(), 2, "{posts:?}");
        assert_eq!(posts[0], posts[1], "the resend is the same call");
        assert_eq!(
            posts[0].get("op").map(ToString::to_string).as_deref(),
            Some(r#""lens-r1-1""#)
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
