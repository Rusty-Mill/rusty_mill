//! The claims-mode tool catalogue and its translation into agent actions.
//!
//! A claims task grants no turns (C-R1): every call is authorised by the
//! principal's session credential, which the server derives itself and never
//! shows the model. So no tool takes a ticket, and there is no `wait`; the
//! host (orch) decides when a lens runs.

use super::{obj, op_of, recipients, refs, schema, translated, u64_of, Translated};
use crate::fresh_op;
use rusty_bbp::*;
use rusty_serde::Value;

/// The tools a claims session offers.
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
            "The task's current state and budgets. Free.",
            schema(vec![], &[]),
        ),
        tool(
            "read",
            "Messages visible to your role with id greater than `after`, at most 20, including claims and evidence. Charged per call.",
            schema(
                vec![
                    ("after", "integer", "Last message id you have seen; omit for the start"),
                    ("limit", "integer", "Page size, at most 20"),
                    ("op", "string", "Operation id; omit to let the host choose"),
                ],
                &[],
            ),
        ),
        tool(
            "get_artifact",
            "An artifact record and its content by id. Charged once per artifact.",
            schema(
                vec![("id", "integer", "Artifact id"), ("op", "string", "Operation id")],
                &["id"],
            ),
        ),
        tool(
            "post",
            "Post a claim or evidence on one. kind=claim: {text, domains, stakes: low|medium|high, reversible, deps?, answers?, supersedes?}. kind=evidence: {claim, direction: support|counter, evidence_kind: executed|cited|opinion, strength 1-3, note?, evidence}: executed evidence cites the runner's test report (\"art:N\"), cited evidence cites a source artifact or message. refs are strings like \"art:3\" or \"msg:7\"; every post refs the brief.",
            schema(
                vec![
                    ("op", "string", "Operation id, unique for your role across the task; retry with the same op to get the same result"),
                    ("kind", "string", "claim or evidence"),
                    ("to", "array", "Recipients: role names, \"human\" or \"*\" (default)"),
                    ("refs", "array", "References"),
                    ("evidence", "array", "Evidence references"),
                    ("text", "string", "claim: the claim, one sentence"),
                    ("domains", "array", "claim: domain names, at most 8"),
                    ("stakes", "string", "claim: low, medium or high"),
                    ("reversible", "boolean", "claim: false if acting on it cannot be undone"),
                    ("deps", "array", "claim: message ids of claims this one rests on"),
                    ("answers", "integer", "claim: the question claim this answers"),
                    ("supersedes", "integer", "claim: the claim this replaces once accepted"),
                    ("claim", "integer", "evidence: the claim's message id"),
                    ("direction", "string", "evidence: support or counter"),
                    ("evidence_kind", "string", "evidence: executed, cited or opinion"),
                    ("strength", "integer", "evidence: 1 to 3 (executed is always 1)"),
                    ("note", "string", "evidence: why, briefly"),
                ],
                &["op", "kind"],
            ),
        ),
        tool(
            "put_artifact",
            "Store an artifact. kind diff: {body}; candidate: {base, diffs[], claim} (submits a run, debits the claim's budget); source: {origin, retrieved_at, excerpt} (outside material for cited evidence, excerpt at most 4,000 characters).",
            schema(
                vec![
                    ("op", "string", "Operation id"),
                    ("kind", "string", "diff, candidate or source"),
                    ("body", "string", "diff: unified diff text"),
                    ("base", "string", "candidate: full base commit id"),
                    ("diffs", "array", "candidate: diff artifact ids in apply order"),
                    ("claim", "integer", "candidate: the claim it tests"),
                    ("origin", "string", "source: where it came from (a URL or citation)"),
                    ("retrieved_at", "string", "source: when it was retrieved"),
                    ("excerpt", "string", "source: the quoted material"),
                ],
                &["op", "kind"],
            ),
        ),
        tool(
            "claims_delta",
            "Cards of the claims whose status, score or evidence changed after revision `after` in `domains`, plus their dependents. Free.",
            schema(
                vec![
                    ("after", "integer", "Revision you last saw; 0 for everything"),
                    ("domains", "array", "Domain names to follow"),
                ],
                &["domains"],
            ),
        ),
        tool(
            "claim_card",
            "One claim's card: status, score, evidence, dependencies, candidates, failing profiles. Free.",
            schema(vec![("claim", "integer", "The claim's message id")], &["claim"]),
        ),
        tool(
            "claim_line",
            "One claim as a single line, for compact context. Free.",
            schema(vec![("claim", "integer", "The claim's message id")], &["claim"]),
        ),
    ])
}

fn word<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    a.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{k} required"))
}

fn ids(v: Option<&Value>, what: &str) -> Result<Vec<u64>, String> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|i| i.as_u64().ok_or_else(|| format!("{what} must be integers")))
            .collect(),
        Some(_) => Err(format!("{what} must be an array")),
    }
}

fn strings(v: Option<&Value>, what: &str) -> Result<Vec<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{what} must be strings"))
            })
            .collect(),
        Some(_) => Err(format!("{what} must be an array")),
    }
}

fn claim_body(a: &Value) -> Result<ClaimBody, String> {
    let stakes = match word(a, "stakes")? {
        "low" => Stakes::Low,
        "medium" => Stakes::Medium,
        "high" => Stakes::High,
        other => return Err(format!("stakes must be low, medium or high, got {other}")),
    };
    let msg = |k: &str| -> Result<Option<MsgId>, String> {
        match a.get(k) {
            None | Some(Value::Null) => Ok(None),
            v => u64_of(v, k).map(|i| Some(MsgId(i))),
        }
    };
    Ok(ClaimBody {
        text: word(a, "text")?.to_owned(),
        domains: strings(a.get("domains"), "domains")?,
        deps: ids(a.get("deps"), "deps")?.into_iter().map(MsgId).collect(),
        answers: msg("answers")?,
        supersedes: msg("supersedes")?,
        stakes,
        reversible: a
            .get("reversible")
            .and_then(Value::as_bool)
            .ok_or("reversible required (true or false)")?,
    })
}

fn evidence_body(a: &Value) -> Result<EvidenceBody, String> {
    let direction = match word(a, "direction")? {
        "support" => Direction::Support,
        "counter" => Direction::Counter,
        other => return Err(format!("direction must be support or counter, got {other}")),
    };
    let kind = match word(a, "evidence_kind")? {
        "executed" => EvidenceKind::Executed,
        "cited" => EvidenceKind::Cited,
        "opinion" => EvidenceKind::Opinion,
        other => {
            return Err(format!(
                "evidence_kind must be executed, cited or opinion, got {other}"
            ))
        }
    };
    let strength = u64_of(a.get("strength"), "strength")?;
    Ok(EvidenceBody {
        claim: MsgId(u64_of(a.get("claim"), "claim")?),
        direction,
        kind,
        strength: u8::try_from(strength).map_err(|_| "strength must be 1 to 3".to_owned())?,
        note: a
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    })
}

fn post(a: &Value) -> Result<Translated, String> {
    let (kind, body) = match word(a, "kind")? {
        "claim" => (MessageKind::Claim, Body::Claim(claim_body(a)?)),
        "evidence" => (MessageKind::Evidence, Body::Evidence(evidence_body(a)?)),
        other => {
            return Err(format!(
                "a claims task posts claim or evidence, not {other}"
            ))
        }
    };
    let mut cites = refs(a.get("refs"))?;
    // Evidence refs its claim (R2); add it rather than refuse a model that forgot.
    if let Body::Evidence(e) = &body {
        if !cites.contains(&Ref::Msg(e.claim)) {
            cites.push(Ref::Msg(e.claim));
        }
    }
    let draft = Draft {
        kind,
        to: recipients(a.get("to"))?,
        body,
        refs: cites,
        evidence: refs(a.get("evidence"))?,
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    Ok(translated(op_of(a), AgentAction::Post(draft), None))
}

fn put(a: &Value) -> Result<Translated, String> {
    let (payload, body) = match word(a, "kind")? {
        "diff" => (
            ArtifactPayload::Diff,
            a.get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
        ),
        "candidate" => (
            ArtifactPayload::Candidate(Candidate {
                base: word(a, "base")?.to_owned(),
                diffs: ids(a.get("diffs"), "diffs")?
                    .into_iter()
                    .map(ArtId)
                    .collect(),
                claim: Some(MsgId(u64_of(a.get("claim"), "claim")?)),
            }),
            vec![],
        ),
        "source" => {
            let excerpt = word(a, "excerpt")?.to_owned();
            (
                ArtifactPayload::Source(Source {
                    origin: word(a, "origin")?.to_owned(),
                    retrieved_at: word(a, "retrieved_at")?.to_owned(),
                    digest: Sha256::of(excerpt.as_bytes()),
                    excerpt,
                }),
                vec![],
            )
        }
        other => {
            return Err(format!(
                "put_artifact kind must be diff, candidate or source, got {other}"
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

/// Translate a claims-mode tool call into an agent action.
pub fn action(name: &str, a: &Value) -> Result<Translated, String> {
    let view = |action| Ok(translated(fresh_op(), action, None));
    match name {
        "task_card" => view(AgentAction::TaskCard),
        "read" | "get_artifact" => super::action(name, a),
        "post" => post(a),
        "put_artifact" => put(a),
        "claims_delta" => view(AgentAction::ClaimsDelta {
            after: Rev(a.get("after").and_then(Value::as_u64).unwrap_or(0)),
            domains: strings(a.get("domains"), "domains")?,
        }),
        "claim_card" => view(AgentAction::ClaimCard {
            claim: MsgId(u64_of(a.get("claim"), "claim")?),
        }),
        "claim_line" => view(AgentAction::ClaimLine {
            claim: MsgId(u64_of(a.get("claim"), "claim")?),
        }),
        other => Err(format!("unknown tool {other}")),
    }
}
