//! Strict parsing of the model's reply into board outputs.
//!
//! Unknown kinds, blank bodies, missing confidence, and malformed refs are
//! errors, never guesses. Ref *syntax* is checked here; whether `E-<n>`
//! exists is left to `Board::append`, which already validates it.

use orch_core::board::{Confidence, EntryKind, Verdict};
use orch_core::task::Role;
use orch_core::{EntryId, Ref, Text};
use orch_dispatch::{AgentError, Output};
use rusty_json::Value;

/// Replies with more entries than this are rejected whole.
pub const MAX_ENTRIES: usize = 8;
/// Bodies longer than this, in characters, are rejected.
pub const MAX_BODY_CHARS: usize = 500;

/// Entry kinds a card of `role` may write. Empty means the adapter does
/// not serve that role. Never `decision`: agents propose decisions as
/// findings and the board refuses unreviewed ones (ADR-0005). A review
/// card writes exactly one `review`, optionally with findings beside it.
pub fn allowed_kinds(role: Role) -> &'static [&'static str] {
    match role {
        Role::Research | Role::Triage => &["finding", "question", "assumption"],
        Role::Design => &["finding", "question", "assumption"],
        Role::Review { .. } => &["review", "finding"],
        Role::Implement => &[],
    }
}

/// Parse one JSON reply for a card of `role`. Tolerates a single ``` fence
/// around the object and nothing else.
pub fn parse(stdout: &str, role: Role) -> Result<Vec<Output>, AgentError> {
    let allowed = allowed_kinds(role);
    if allowed.is_empty() {
        return Err(fail(format!("role {role:?} is not served by this adapter")));
    }
    let json =
        Value::parse(unfence(stdout)).map_err(|e| fail(format!("reply is not JSON: {e}")))?;
    let entries = json
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("reply has no \"entries\" array".to_owned()))?;
    if entries.is_empty() {
        return Err(fail("reply has zero entries".to_owned()));
    }
    if entries.len() > MAX_ENTRIES {
        return Err(fail(format!(
            "{} entries exceeds the cap of {MAX_ENTRIES}",
            entries.len()
        )));
    }
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| entry(e, role, allowed).map_err(|m| fail(format!("entry {i}: {m}"))))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|outputs| exactly_one_review(role, outputs))
}

/// A review card must deliver its verdict once: no review entry means the
/// reviewer dodged, two means the verdict is ambiguous.
fn exactly_one_review(role: Role, outputs: Vec<Output>) -> Result<Vec<Output>, AgentError> {
    if !matches!(role, Role::Review { .. }) {
        return Ok(outputs);
    }
    let reviews = outputs
        .iter()
        .filter(|o| matches!(o.kind, EntryKind::Review { .. }))
        .count();
    match reviews {
        1 => Ok(outputs),
        0 => Err(fail(
            "a review card must include one review entry".to_owned(),
        )),
        n => Err(fail(format!("{n} review entries; exactly one is allowed"))),
    }
}

fn unfence(s: &str) -> &str {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let body = rest.split_once('\n').map_or("", |(_, b)| b);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

fn entry(v: &Value, role: Role, allowed: &[&str]) -> Result<Output, String> {
    let kind_name = string(v, "kind")?;
    if !allowed.contains(&kind_name) {
        return Err(format!("kind {kind_name:?} is not allowed here"));
    }
    let kind = match (kind_name, role) {
        ("finding", _) => EntryKind::Finding {
            confidence: confidence(v)?,
        },
        ("question", _) => EntryKind::Question,
        ("assumption", _) => EntryKind::Assumption,
        ("review", Role::Review { target }) => EntryKind::Review {
            of: target,
            verdict: verdict(v)?,
        },
        (other, _) => return Err(format!("kind {other:?} is unknown")),
    };
    let body = string(v, "body")?;
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("body exceeds {MAX_BODY_CHARS} characters"));
    }
    let body = Text::new(body).ok_or_else(|| "body is blank".to_owned())?;
    let refs = v
        .get("refs")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing \"refs\" array".to_owned())?
        .iter()
        .map(|r| {
            r.as_str()
                .ok_or_else(|| "ref is not a string".to_owned())
                .and_then(parse_ref)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Output {
        kind,
        body,
        refs,
        supersedes: None,
    })
}

fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {key:?}"))
}

fn confidence(v: &Value) -> Result<Confidence, String> {
    match string(v, "confidence")? {
        "low" => Ok(Confidence::Low),
        "medium" => Ok(Confidence::Medium),
        "high" => Ok(Confidence::High),
        other => Err(format!("confidence {other:?} is not low, medium, or high")),
    }
}

fn verdict(v: &Value) -> Result<Verdict, String> {
    match string(v, "verdict")? {
        "approve" => Ok(Verdict::Approve),
        "changes_requested" => Ok(Verdict::ChangesRequested),
        other => Err(format!(
            "verdict {other:?} is not approve or changes_requested"
        )),
    }
}

/// `E-<n>`, or `path:`, `commit:`, `url:` followed by non-blank text.
fn parse_ref(s: &str) -> Result<Ref, String> {
    if let Some(n) = s.strip_prefix("E-") {
        return n
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|n| Ref::Entry(EntryId::from_raw(n)))
            .ok_or_else(|| format!("malformed entry ref {s:?}"));
    }
    let (scheme, rest) = s
        .split_once(':')
        .ok_or_else(|| format!("malformed ref {s:?}"))?;
    let text = Text::new(rest).ok_or_else(|| format!("ref {s:?} has no target"))?;
    match scheme {
        "path" => Ok(Ref::Path(text)),
        "commit" => Ok(Ref::Commit(text)),
        "url" => Ok(Ref::Url(text)),
        other => Err(format!("unknown ref scheme {other:?}")),
    }
}

fn fail(message: String) -> AgentError {
    AgentError(format!("ollama reply: {message}"))
}
