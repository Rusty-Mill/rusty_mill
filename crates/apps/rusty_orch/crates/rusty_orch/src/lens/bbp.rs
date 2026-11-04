//! The lens runner's way into BBP (#594, D54): one `bbp mcp` session per
//! lens, speaking for that lens's principal with its session credential.
//!
//! - [`Child`]: the session process, newline-delimited JSON-RPC over its
//!   stdio, every call bounded by a deadline.
//! - [`Session`]: the claims tools over any [`Rpc`]. It keeps what the
//!   lens can see: claim bodies and evidence notes from incremental `read`
//!   pages (the card view carries neither stakes nor notes), and the cards
//!   from `claims_delta`. It is the lens's [`Board`]: posts go out under
//!   its credential, with a stable op id so a retry replays.
//!
//! `read` is charged per call (addendum R11), so a session reads only new
//! messages, a page at a time; the card, line and delta views are free.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use orch_lens::card::Card;
use orch_lens::context::{Line, MAX_LINES};
use orch_lens::round::Board;
use orch_lens::{LensConfig, Post};
use rusty_bbp::{
    ArtId, Body, ClaimBody, ClaimStatus, ClaimView, ClaimsConfig, Direction, EvidenceKind, MsgId,
    Ref, Response, Rev, Sha256, Stakes,
};
use rusty_serde::Value;

/// How long one tool call may take.
pub const CALL_DEADLINE: Duration = Duration::from_secs(30);
/// `read`'s largest page.
const PAGE: u64 = 20;

/// One tool call to a claims session: the tool's text result, or why not.
pub trait Rpc {
    fn call(&mut self, tool: &str, args: Value) -> Result<String, String>;
}

/// A `bbp mcp` session process for one lens.
pub struct Child {
    child: std::process::Child,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    next: u64,
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

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{msg}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| format!("bbp mcp: {e}"))
    }

    /// Send one request and wait for its response's `result`.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        self.send(&map(vec![
            ("jsonrpc", "2.0".into()),
            ("id", id.into()),
            ("method", method.into()),
            ("params", params),
        ]))?;
        loop {
            let line = match self.lines.recv_timeout(CALL_DEADLINE) {
                Ok(line) => line.map_err(|e| format!("bbp mcp: {e}"))?,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "bbp mcp: no answer to {method} in {}s",
                        CALL_DEADLINE.as_secs()
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => return Err("bbp mcp exited".to_owned()),
            };
            let reply: Value = rusty_serde::json::from_str(&line)
                .map_err(|e| format!("bbp mcp sent bad JSON: {e}"))?;
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(e) = reply.get("error") {
                let msg = e.get("message").and_then(Value::as_str).unwrap_or("error");
                return Err(format!("bbp mcp {method}: {msg}"));
            }
            return reply
                .get("result")
                .cloned()
                .ok_or_else(|| format!("bbp mcp {method}: no result"));
        }
    }
}

impl Rpc for Child {
    fn call(&mut self, tool: &str, args: Value) -> Result<String, String> {
        let result = self.request(
            "tools/call",
            map(vec![("name", tool.into()), ("arguments", args)]),
        )?;
        tool_text(&result).map_err(|e| format!("{tool}: {e}"))
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // Best effort: the session holds no state of its own to lose.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The text of a `tools/call` result; an `isError` result is an error.
fn tool_text(result: &Value) -> Result<String, String> {
    let text = result
        .get("content")
        .and_then(|c| match c {
            Value::Seq(items) => items.first(),
            _ => None,
        })
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .ok_or("result has no text")?
        .to_owned();
    match result.get("isError").and_then(Value::as_bool) {
        Some(true) => Err(text),
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
    notes: BTreeMap<MsgId, String>,
    views: BTreeMap<MsgId, ClaimView>,
    /// The task revision the views are current to.
    rev: Rev,
    brief: Option<ArtId>,
}

impl<R: Rpc> Session<R> {
    pub fn new(lens: &str, rpc: R) -> Self {
        Self {
            lens: lens.to_owned(),
            rpc,
            cursor: None,
            claims: BTreeMap::new(),
            notes: BTreeMap::new(),
            views: BTreeMap::new(),
            rev: Rev(0),
            brief: None,
        }
    }

    fn ask(&mut self, tool: &str, args: Value) -> Result<Response, String> {
        let text = self.rpc.call(tool, args)?;
        let resp: Response = rusty_serde::json::from_str(&text)
            .map_err(|e| format!("{tool}: unreadable answer: {e}"))?;
        match resp {
            Response::Rejected(r) => Err(format!("{tool}: {:?}: {}", r.code, r.detail)),
            other => Ok(other),
        }
    }

    /// Bring the lens's view up to date: new messages (claim bodies and
    /// evidence notes), then the cards that changed since the last refresh.
    /// The revision is read first, so a change made meanwhile is seen again
    /// next time rather than missed.
    pub fn refresh(&mut self, config: &LensConfig) -> Result<(), String> {
        let rev = match self.ask("task_card", map(vec![]))? {
            Response::Card(c) => {
                self.brief = c.brief;
                c.rev
            }
            other => return Err(format!("task_card answered {other:?}")),
        };
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
                match m.draft.body {
                    Body::Claim(b) => {
                        self.claims.insert(m.id, b);
                    }
                    Body::Evidence(e) => {
                        self.notes.insert(m.id, e.note);
                    }
                    _ => {}
                }
            }
            self.cursor = next.or(self.cursor);
            if !more {
                return Ok(());
            }
        }
    }

    /// The domains the lens follows; `*` is every domain seen so far.
    fn domains(&self, config: &LensConfig) -> BTreeSet<String> {
        let Some(lens) = config.lens(&self.lens) else {
            return BTreeSet::new();
        };
        let seen = self.claims.values().flat_map(|b| b.domains.iter());
        seen.chain(lens.domains.iter())
            .filter(|d| d.as_str() != "*" && lens.follows(d))
            .cloned()
            .collect()
    }

    /// Every card the lens can see, by claim id. A claim whose body the
    /// lens has not read is left out.
    pub fn cards(&self, config: &LensConfig, claims: &ClaimsConfig) -> Vec<Card> {
        let notes = |m: MsgId| self.notes.get(&m).cloned().unwrap_or_default();
        self.views
            .values()
            .filter_map(|v| {
                let body = self.claims.get(&v.id)?;
                Some(Card::from_view(v, body, &notes, config, claims))
            })
            .collect()
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
    fn post(&mut self, lens: &str, posts: &[Post]) -> Result<(), String> {
        if lens != self.lens {
            return Err(format!("{}'s session cannot post for {lens}", self.lens));
        }
        for p in posts {
            let args = post_args(p, self.brief);
            match self.ask("post", args)? {
                Response::Posted(_) => {}
                other => return Err(format!("post answered {other:?}")),
            }
        }
        Ok(())
    }
}

/// The `post` tool's arguments for `p`. Every post refs the brief. The op
/// id is a digest of the rest, so posting the same thing again replays
/// BBP's first answer instead of adding a copy.
fn post_args(p: &Post, brief: Option<ArtId>) -> Value {
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
    let digest = Sha256::of(map(fields.clone()).to_string().as_bytes()).hex();
    fields.insert(0, ("op", format!("lens-{}", &digest[..32]).into()));
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
        replies: BTreeMap<String, VecDeque<Result<String, String>>>,
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

        fn args(&self, tool: &str) -> Vec<&Value> {
            self.calls
                .iter()
                .filter(|(t, _)| t == tool)
                .map(|(_, a)| a)
                .collect()
        }
    }

    impl Rpc for Fake {
        fn call(&mut self, tool: &str, args: Value) -> Result<String, String> {
            self.calls.push((tool.to_owned(), args));
            self.replies
                .get_mut(tool)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Err(format!("no reply for {tool}")))
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
        }
    }

    fn word(v: &Value, k: &str) -> String {
        v.get(k).map(Value::to_string).unwrap_or_default()
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
        let mut s = Session::new("adversary", fake);
        s.refresh(&cfg).expect("refresh");
        let cards = s.cards(&cfg, &ClaimsConfig::default());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].evidence[0].note, "fails on empty files");
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
        let mut s = Session::new("adversary", fake);
        s.refresh(&cfg).expect("first");
        s.refresh(&cfg).expect("second");
        assert_eq!(word(s.rpc.args("claims_delta")[1], "after"), "5");
        assert_eq!(word(s.rpc.args("read")[1], "after"), "8");
        let cards = s.cards(&cfg, &ClaimsConfig::default());
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
        let mut s = Session::new("adversary", fake);
        s.refresh(&cfg).expect("refresh");
        let ids: Vec<MsgId> = s
            .cards(&cfg, &ClaimsConfig::default())
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, [MsgId(7)]);
    }

    #[test]
    fn a_rejection_is_an_error() {
        let rej = Response::Rejected(rusty_bbp::Rejection::new(
            rusty_bbp::Code::BadCredential,
            "reassigned",
        ));
        let mut s = Session::new("adversary", Fake::default().on("task_card", rej));
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
        let mut s = Session::new("adversary", fake);
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
    fn a_post_refs_the_brief_and_its_op_is_a_digest() {
        let a = post_args(&counter("x"), Some(ArtId(1)));
        assert_eq!(word(&a, "refs"), r#"["art:1"]"#);
        assert_eq!(word(&a, "evidence"), r#"["msg:3"]"#);
        assert_eq!(word(&a, "direction"), r#""counter""#);
        assert_eq!(
            word(&a, "op"),
            word(&post_args(&counter("x"), Some(ArtId(1))), "op")
        );
        assert_ne!(
            word(&a, "op"),
            word(&post_args(&counter("y"), Some(ArtId(1))), "op")
        );
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
        let c = post_args(&claim, Some(ArtId(1)));
        assert_eq!(word(&c, "refs"), r#"["art:1"]"#, "the brief once");
        assert_eq!(word(&c, "answers"), "3");
        assert_eq!(word(&c, "stakes"), r#""high""#);
        assert_eq!(word(&c, "reversible"), "false");
        assert!(c.get("supersedes").is_none());
    }

    #[test]
    fn a_session_posts_only_for_its_own_lens() {
        let fake = Fake::default().on("post", Response::Posted(MsgId(30)));
        let mut s = Session::new("adversary", fake);
        assert!(s.post("operator", &[counter("x")]).is_err());
        assert!(s.rpc.calls.is_empty());
        s.post("adversary", &[counter("x")]).expect("posted");
        assert_eq!(s.rpc.args("post").len(), 1);
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
        assert_eq!(tool_text(&bad), Err("no".to_owned()));
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

    #[cfg(unix)]
    #[test]
    fn a_child_session_answers_by_id_and_reports_its_exit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("orch-bbp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let bin = dir.join("bbp");
        std::fs::write(&bin, SCRIPT).expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        let mut child = Child::spawn(&bin, &dir, "t", "adversary").expect("spawn");
        assert_eq!(
            child.call("task_card", map(vec![])),
            Ok("adversary yes scrubbed".to_owned())
        );
        let gone = child.call("task_card", map(vec![])).expect_err("exited");
        assert!(
            gone.contains("exited") || gone.contains("Broken pipe"),
            "{gone}"
        );
        drop(child);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
