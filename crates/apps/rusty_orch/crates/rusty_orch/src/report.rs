//! Two views of a [`Summary`]: a text page for people and one JSON object
//! for tools. Both are built here from `orch-core` values; the domain types
//! carry no serialisation of their own.

use orch_core::board::{Author, Confidence, Entry, EntryKind, Verdict};
use orch_core::task::{Agent, Role, Status, Task, TaskState};
use orch_core::Ref;
use rusty_json::{Map, Value};

use crate::run::{Ended, Summary};

/// Human-readable: goal, every card, the live board, the ledger.
pub fn text(s: &Summary) -> String {
    let mut out = String::new();
    out.push_str(&format!("goal: {}\n", s.goal.outcome()));
    out.push_str(&format!("ended: {}\n", s.ended));
    out.push_str(&format!("calls: {}\n\ncards:\n", s.ledger.calls()));
    for t in s.plan.tasks() {
        out.push_str(&format!(
            "  {} {:<9} {:<8} {}\n",
            t.id(),
            role_name(&t.spec().role),
            status_name(t.state().status()),
            state_detail(t)
        ));
    }
    out.push_str("\nboard (live):\n");
    for e in s.board.live() {
        let c = e.content();
        out.push_str(&format!(
            "  {} [{}] {}: {}\n",
            e.id(),
            kind_name(&c.kind),
            author_name(c.author),
            c.body
        ));
        for r in &c.refs {
            out.push_str(&format!("      -> {}\n", ref_text(r)));
        }
    }
    out
}

/// One JSON object: `ended`, `calls`, `tasks`, `entries` (all, with
/// `superseded` flags, so nothing the agents wrote is hidden).
pub fn json(s: &Summary) -> Value {
    let mut root = Map::new();
    root.insert("goal".into(), s.goal.outcome().as_str().into());
    root.insert("ended".into(), ended_json(&s.ended));
    root.insert("calls".into(), Value::from(u64::from(s.ledger.calls())));
    root.insert(
        "tasks".into(),
        s.plan
            .tasks()
            .iter()
            .map(task_json)
            .collect::<Vec<_>>()
            .into(),
    );
    root.insert(
        "entries".into(),
        s.board
            .entries()
            .iter()
            .map(|e| entry_json(e, s.board.is_superseded(e.id())))
            .collect::<Vec<_>>()
            .into(),
    );
    root.into()
}

fn ended_json(e: &Ended) -> Value {
    let mut m = Map::new();
    let kind = match e {
        Ended::Finished => "finished",
        Ended::Blocked(_) => "blocked",
        Ended::Failed(_) => "failed",
        Ended::WallClock(_) => "wall_clock",
    };
    m.insert("kind".into(), kind.into());
    match e {
        Ended::Blocked(ids) => {
            m.insert(
                "tasks".into(),
                ids.iter()
                    .map(|id| id.to_string().into())
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        Ended::Failed(err) => {
            m.insert("error".into(), err.to_string().into());
        }
        Ended::WallClock(d) => {
            m.insert("limit_secs".into(), Value::from(d.as_secs()));
        }
        Ended::Finished => {}
    }
    m.into()
}

fn task_json(t: &Task) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), t.id().to_string().into());
    m.insert("role".into(), role_name(&t.spec().role).into());
    if let Role::Review { target } = t.spec().role {
        m.insert("target".into(), target.to_string().into());
    }
    m.insert("status".into(), status_name(t.state().status()).into());
    match t.state() {
        TaskState::Pending => {}
        TaskState::Running { agent } => {
            m.insert("agent".into(), agent_name(*agent).into());
        }
        TaskState::Blocked { agent, question } => {
            m.insert("agent".into(), agent_name(*agent).into());
            m.insert("question".into(), question.to_string().into());
        }
        TaskState::Done { agent, outputs } => {
            m.insert("agent".into(), agent_name(*agent).into());
            m.insert(
                "outputs".into(),
                outputs
                    .iter()
                    .map(|id| id.to_string().into())
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        TaskState::Failed { agent, reason } => {
            m.insert("agent".into(), agent_name(*agent).into());
            m.insert("reason".into(), reason.as_str().into());
        }
    }
    m.into()
}

fn entry_json(e: &Entry, superseded: bool) -> Value {
    let c = e.content();
    let mut m = Map::new();
    m.insert("id".into(), e.id().to_string().into());
    m.insert("kind".into(), kind_name(&c.kind).into());
    match c.kind {
        EntryKind::Finding { confidence } => {
            m.insert("confidence".into(), confidence_name(confidence).into());
        }
        EntryKind::Answer { to } => {
            m.insert("to".into(), to.to_string().into());
        }
        EntryKind::Review { of, verdict } => {
            m.insert("of".into(), of.to_string().into());
            m.insert("verdict".into(), verdict_name(verdict).into());
        }
        EntryKind::Decision | EntryKind::Assumption | EntryKind::Question | EntryKind::Artifact => {
        }
    }
    m.insert("author".into(), author_name(c.author).into());
    if let Some(task) = c.task {
        m.insert("task".into(), task.to_string().into());
    }
    m.insert("body".into(), c.body.as_str().into());
    m.insert(
        "refs".into(),
        c.refs
            .iter()
            .map(|r| ref_text(r).into())
            .collect::<Vec<_>>()
            .into(),
    );
    if let Some(s) = c.supersedes {
        m.insert("supersedes".into(), s.to_string().into());
    }
    m.insert("superseded".into(), superseded.into());
    m.into()
}

fn state_detail(t: &Task) -> String {
    match t.state() {
        TaskState::Pending => String::new(),
        TaskState::Running { agent } => agent_name(*agent).to_owned(),
        TaskState::Blocked { agent, question } => {
            format!("{} waits on {question}", agent_name(*agent))
        }
        TaskState::Done { agent, outputs } => {
            let ids: Vec<String> = outputs.iter().map(ToString::to_string).collect();
            format!("{} -> {}", agent_name(*agent), ids.join(", "))
        }
        TaskState::Failed { agent, reason } => format!("{}: {reason}", agent_name(*agent)),
    }
}

/// `path:x`, `commit:x`, `url:x`, `E-n`: the same syntax the goal file and
/// the adapter protocol accept, so output can be pasted back in.
pub fn ref_text(r: &Ref) -> String {
    match r {
        Ref::Path(t) => format!("path:{t}"),
        Ref::Commit(t) => format!("commit:{t}"),
        Ref::Url(t) => format!("url:{t}"),
        Ref::Entry(id) => id.to_string(),
    }
}

pub fn role_name(r: &Role) -> &'static str {
    match r {
        Role::Research => "research",
        Role::Design => "design",
        Role::Implement => "implement",
        Role::Triage => "triage",
        Role::Review { .. } => "review",
    }
}

pub fn status_name(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Blocked => "blocked",
        Status::Done => "done",
        Status::Failed => "failed",
    }
}

pub fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Gemini => "gemini",
        Agent::Local => "local",
    }
}

fn author_name(a: Author) -> &'static str {
    match a {
        Author::Human => "human",
        Author::Agent(agent) => agent_name(agent),
    }
}

fn kind_name(k: &EntryKind) -> &'static str {
    match k {
        EntryKind::Finding { .. } => "finding",
        EntryKind::Decision => "decision",
        EntryKind::Assumption => "assumption",
        EntryKind::Question => "question",
        EntryKind::Answer { .. } => "answer",
        EntryKind::Artifact => "artifact",
        EntryKind::Review { .. } => "review",
    }
}

fn confidence_name(c: Confidence) -> &'static str {
    match c {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Approve => "approve",
        Verdict::ChangesRequested => "changes_requested",
    }
}
