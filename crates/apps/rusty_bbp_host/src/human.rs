//! The human channel as CLI actions. Every action carries a fresh `op`;
//! destructive ones (`reject`, `rerun`, `resume`, `cancel`) carry the card
//! revision the human saw (`--rev`, R15), never the one the CLI happens to
//! load, so a delayed command meets newer work with `stale_rev`.

use crate::approval::Verified;
use crate::{fresh_op, now, open_driver};
use rusty_bbp::*;
use std::path::Path;

fn art(s: &str) -> Result<ArtId, String> {
    s.trim_start_matches("art:")
        .parse()
        .map(ArtId)
        .map_err(|_| format!("bad artifact id {s}"))
}

fn msg(s: &str) -> Result<MsgId, String> {
    s.trim_start_matches("msg:")
        .parse()
        .map(MsgId)
        .map_err(|_| format!("bad message id {s}"))
}

fn run(s: &str) -> Result<RunId, String> {
    s.trim_start_matches("run:")
        .parse()
        .map(RunId)
        .map_err(|_| format!("bad run id {s}"))
}

pub fn parse_state(s: &str) -> Result<State, String> {
    Ok(match s {
        "planning" => State::Planning,
        "plan_gate" => State::PlanGate,
        "build" => State::Build,
        "test" => State::Test,
        "review" => State::Review,
        "merge_gate" => State::MergeGate,
        "approved" => State::Approved,
        "escalated" => State::Escalated,
        "open" => State::Open,
        other => return Err(format!("unknown state {other}")),
    })
}

fn field(s: &str) -> Result<BudgetField, String> {
    Ok(match s {
        "messages" => BudgetField::Messages,
        "bytes" => BudgetField::Bytes,
        "reads" => BudgetField::Reads,
        "reads_per_turn" => BudgetField::ReadsPerTurn,
        "turns" => BudgetField::Turns,
        "iterations" => BudgetField::Iterations,
        other => return Err(format!("unknown budget field {other}")),
    })
}

/// Build the action from `verb` and its arguments. `rev` is the card revision
/// the human saw; the actions that fence on it refuse to run without one.
pub fn action(verb: &str, args: &[&str], rev: Option<Rev>) -> Result<HumanAction, String> {
    let rev =
        || rev.ok_or_else(|| format!("{verb}: --rev REV (the card revision you saw) required"));
    let a = |i: usize| {
        args.get(i)
            .copied()
            .ok_or_else(|| format!("{verb}: missing argument {i}"))
    };
    let rest = |i: usize| args.get(i..).unwrap_or(&[]).join(" ");
    Ok(match verb {
        "approve-plan" => HumanAction::Approve {
            gate: Gate::Plan,
            subject: art(a(0)?)?,
            run: None,
        },
        "approve-merge" => HumanAction::Approve {
            gate: Gate::Merge,
            subject: art(a(0)?)?,
            run: Some(run(a(1)?)?),
        },
        "reject" => HumanAction::Reject {
            expected_rev: rev()?,
            target: parse_state(a(0)?)?,
            reason: rest(1),
        },
        "decision" => {
            let outcome = match a(1)? {
                "accept" => Outcome::Accept,
                "reject" => Outcome::Reject,
                other => {
                    return Err(format!(
                        "decision outcome must be accept or reject, got {other}"
                    ))
                }
            };
            let subject = msg(a(0)?)?;
            let mut d = Draft::new(MessageKind::Decision, "");
            d.body = Body::Decision(Decision {
                subject,
                outcome,
                note: rest(2),
            });
            d.refs = vec![Ref::Msg(subject)];
            HumanAction::Post(d)
        }
        "ask" => HumanAction::Post(
            Draft::new(MessageKind::Ask, rest(1)).to(Recipient::Role(RoleId::from(a(0)?))),
        ),
        "answer" => {
            HumanAction::Post(Draft::new(MessageKind::Answer, rest(1)).reply_to(msg(a(0)?)?))
        }
        // Claims mode (C-R10): the human's clarification of the intent. It refs
        // the brief; `task` targets the whole task, `msg:N` replies to a claim.
        "refine" => {
            let d = Draft::new(MessageKind::Answer, rest(2)).refs(vec![Ref::art(art(a(1)?)?)]);
            HumanAction::Post(match a(0)? {
                "task" => d,
                target => d.reply_to(msg(target)?),
            })
        }
        "rerun" => HumanAction::Rerun {
            candidate: art(a(0)?)?,
            expected_rev: rev()?,
        },
        "receipt" => HumanAction::MergeReceipt {
            candidate: art(a(0)?)?,
            revision: a(1)?.to_owned(),
        },
        "resume" => HumanAction::Resume {
            target: parse_state(a(0)?)?,
            expected_rev: rev()?,
        },
        "extend" => HumanAction::BudgetExtended {
            field: field(a(0)?)?,
            limit: a(1)?
                .parse()
                .map_err(|_| "limit must be a number".to_owned())?,
        },
        "cancel" => HumanAction::Cancel {
            expected_rev: rev()?,
            reason: rest(0),
        },
        // Claims mode (C-R8, C-R9).
        "claim" => HumanAction::ClaimDecision {
            claim: msg(a(0)?)?,
            outcome: match a(1)? {
                "accept" => ClaimOutcome::Accept,
                "drop" => ClaimOutcome::Drop,
                "reopen" => ClaimOutcome::Reopen,
                other => {
                    return Err(format!(
                        "claim outcome must be accept, drop or reopen, got {other}"
                    ))
                }
            },
        },
        "choose" => HumanAction::Choose {
            question: msg(a(0)?)?,
            chosen: msg(a(1)?)?,
        },
        "extend-claim" => HumanAction::ClaimBudgetExtended { claim: msg(a(0)?)? },
        "withdraw-rerun" => HumanAction::WithdrawRerun {
            candidate: art(a(0)?)?,
        },
        "action-result" => HumanAction::ClaimActionResult {
            mark: a(0)?
                .trim_start_matches("mark:")
                .parse()
                .map_err(|_| "mark must be a number".to_owned())?,
            outcome: match a(1)? {
                "done" => MarkOutcome::Done,
                "aborted" => MarkOutcome::Aborted,
                other => {
                    return Err(format!(
                        "action result must be done or aborted, got {other}"
                    ))
                }
            },
        },
        "close" => HumanAction::Close {
            expected_rev: rev()?,
        },
        // D104: take over one agent session for one mux client; the reason is
        // the rest of the line.
        "takeover" => HumanAction::Takeover {
            claim: msg(a(0)?)?,
            session: a(1)?.to_owned(),
            client: a(2)?.to_owned(),
            minutes: a(3)?
                .parse()
                .map_err(|_| "minutes must be a number".to_owned())?,
            reason: rest(4),
        },
        "takeover-release" => HumanAction::TakeoverRelease {
            takeover: takeover_id(a(0)?)?,
        },
        // #597 G3: adopt a staged holdout suite that changes or removes tests.
        "holdout-updated" => HumanAction::HoldoutUpdated {
            digest: Sha256::from_hex(a(0)?).ok_or("holdout digest must be 64 hex digits")?,
        },
        // #598 C-R11: `intent FILE [claims=msg:1,msg:2] [domains=a,b]`. The
        // caller stores FILE first ([`stage_brief`]), so here it is `SHA:LEN`.
        "intent" => HumanAction::IntentRevised {
            brief: blob_ref(a(0)?)?,
            affects: affects(args.get(1..).unwrap_or(&[]))?,
        },
        other => return Err(format!("unknown human action {other}")),
    })
}

/// A stored blob as `SHA:LEN`, which [`stage_brief`] writes.
fn blob_ref(s: &str) -> Result<BlobRef, String> {
    let bad = || format!("intent: brief must be a file (staged as SHA:LEN), got {s}");
    let (sha, len) = s.split_once(':').ok_or_else(bad)?;
    Ok(BlobRef {
        sha: Sha256::from_hex(sha).ok_or_else(bad)?,
        len: len.parse().map_err(|_| bad())?,
    })
}

/// `claims=msg:1,msg:2` and `domains=a,b`, each at most once; neither is
/// every claim. One argument per list, comma-separated, because repeated
/// flags would keep only the last and narrow what the human stated.
fn affects(args: &[&str]) -> Result<Affects, String> {
    if args.is_empty() {
        return Ok(Affects::All);
    }
    let (mut claims, mut domains) = (None, None);
    for arg in args {
        let (key, list) = arg
            .split_once('=')
            .ok_or_else(|| format!("intent: expected claims=… or domains=…, got {arg}"))?;
        let items = || list.split(',').filter(|x| !x.is_empty());
        let twice = match key {
            "claims" => claims
                .replace(items().map(msg).collect::<Result<Vec<_>, _>>()?)
                .is_some(),
            "domains" => domains
                .replace(items().map(str::to_owned).collect::<Vec<_>>())
                .is_some(),
            other => return Err(format!("intent: unknown list {other}")),
        };
        if twice {
            return Err(format!("intent: {key}= given twice"));
        }
    }
    Ok(Affects::Some {
        claims: claims.unwrap_or_default(),
        domains: domains.unwrap_or_default(),
    })
}

/// `intent FILE ...` read and hashed: the args with FILE replaced by its
/// `SHA:LEN` blob ref, and the bytes to store before that ref is committed.
/// Reads only; any other verb passes through with no bytes.
fn brief_ref(verb: &str, args: &[&str]) -> Result<(Vec<String>, Option<Vec<u8>>), String> {
    let mut out: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    if verb != "intent" {
        return Ok((out, None));
    }
    let file = args.first().ok_or("intent: missing the brief FILE")?;
    let bytes = std::fs::read(file).map_err(|e| format!("intent: {file}: {e}"))?;
    out[0] = format!("{}:{}", Sha256::of(&bytes).hex(), bytes.len());
    Ok((out, Some(bytes)))
}

/// A takeover id as `bbp human` and `bbp takeover-*` take it: `N` or
/// `takeover:N`.
pub fn takeover_id(s: &str) -> Result<u64, String> {
    s.trim_start_matches("takeover:")
        .parse()
        .map_err(|_| "takeover must be a number".to_owned())
}

/// Who answered when the op carries no label.
pub const UNLABELLED: &str = "unlabelled";

/// The op id for a human action answered by `by`: `by:<by>/<fresh>`. The log
/// records only that the task's human principal acted, so the answerer is
/// written into the op, which the log keeps and replay checks.
pub fn labelled_op(by: &str) -> Result<OpId, String> {
    let ok = !by.is_empty()
        && by.len() <= 32
        && by
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        return Err("--as must be 1 to 32 characters of letters, digits, - _ .".into());
    }
    Ok(OpId(format!("by:{by}/{}", fresh_op().0)))
}

/// What marks a caller-chosen decision id in an op: `by:<by>/dec:<id>`.
const DECISION_MARK: &str = "dec:";

/// The op id for a human action sent as decision `id`: `by:<by>/dec:<id>`.
/// The caller chooses `id` (1 to 64 of `[A-Za-z0-9._-]`), so a retry of the
/// same send replays by R15 instead of applying twice, and
/// [`crate::decision`] can tell afterwards whether it was applied.
pub fn decision_op(by: &str, id: &str) -> Result<OpId, String> {
    let OpId(labelled) = labelled_op(by)?;
    let by = labelled
        .strip_prefix("by:")
        .and_then(|r| r.split_once('/'))
        .map_or(by, |(by, _)| by);
    if !valid_decision(id) {
        return Err("--decision must be 1 to 64 characters of letters, digits, - _ .".into());
    }
    Ok(OpId(format!("by:{by}/{DECISION_MARK}{id}")))
}

/// The op `bbp human` runs under, from its `--as`, `--op`, `--decision` and `--fence`: none
/// (a fresh label), the caller's own `--op`, or [`decision_op`]. A decision id needs a
/// fence: an unfenced send can still land after a lookup finds nothing, so its absence
/// would never be final.
pub fn op_for(
    by: &str,
    op: Option<&str>,
    decision: Option<&str>,
    fenced: bool,
) -> Result<Option<OpId>, String> {
    match (op, decision) {
        (Some(_), Some(_)) => Err("--op and --decision are exclusive".into()),
        // A label comes only from `--as` or a receipt (D191): a caller op may not claim one.
        (Some(op), None) if op.starts_with("by:") => {
            Err("--op may not carry a by: label; use --as, or --approval on an owned task".into())
        }
        (Some(op), None) => Ok(Some(OpId(op.to_owned()))),
        (None, Some(_)) if !fenced => Err("--decision needs --fence".into()),
        (None, Some(id)) => decision_op(by, id).map(Some),
        (None, None) => Ok(None),
    }
}

/// Whether `id` is a decision id [`decision_op`] takes.
pub fn valid_decision(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The decision id an op carries, whoever answered: the `id` of [`decision_op`].
pub fn decision_of(op: &OpId) -> Option<&str> {
    op.0.strip_prefix("by:")
        .and_then(|r| r.split_once('/'))
        .and_then(|(_, rest)| rest.strip_prefix(DECISION_MARK))
}

/// Who an op says answered: the label of [`labelled_op`], else [`UNLABELLED`].
pub fn answered_by(op: &OpId) -> &str {
    op.0.strip_prefix("by:")
        .and_then(|r| r.split_once('/'))
        .map_or(UNLABELLED, |(by, _)| by)
}

/// Run one human action against the store. `rev` is the card revision the
/// human saw, required by the actions that fence on it.
pub fn perform(
    dir: &Path,
    task: &TaskId,
    verb: &str,
    args: &[&str],
    rev: Option<Rev>,
) -> Result<Response, String> {
    perform_as(dir, task, verb, args, rev, UNLABELLED)
}

/// [`perform`], with the answer attributed to `by` (see [`labelled_op`]).
pub fn perform_as(
    dir: &Path,
    task: &TaskId,
    verb: &str,
    args: &[&str],
    rev: Option<Rev>,
    by: &str,
) -> Result<Response, String> {
    let act = action(verb, args, rev)?;
    if let Some(rej) = unstaged(dir, task, &act) {
        return Ok(Response::Rejected(rej));
    }
    let _admission = refuse_evidence_extension(dir, task, &act)?.map_err(|r| r.detail)?;
    let op = labelled_op(by)?;
    let mut d = open_driver(dir, task)?;
    // No receipt reaches this path, so an owned task's plan or merge approval is refused here.
    if let Err(rej) = owner_gate(&d.state, &act, None) {
        return Ok(Response::Rejected(rej));
    }
    d.dispatch(&Command::Human { op, action: act }, now())
        .map_err(|e| format!("{e:?}"))
}

/// An evidence run runs under the budget its task was opened with (owner
/// decision, 2026-11-01), so `extend` (every field) and `extend-claim`
/// refuse on a task that carries an evidence marker: `Ok(Err(..))`, a `BadHumanInput`
/// rejection. Only a specific not-found means there is no marker; any other
/// error reading it, or taking the admission lock, is `Err`, a host error
/// that refuses too but is not the human's fault. `bbp pairs` counts a
/// run whose log holds a task-budget extension anyway (one logged by an older `bbp`) as
/// a failed run, so this check is the early, operator-facing half and not the
/// only one.
///
/// An extend that may go ahead returns `Ok(Ok(Some(..)))`, the evidence admission lock (see
/// [`crate::moderator::evidence_admission`]); the caller holds it through its
/// append, so a capped first start cannot write the marker in between.
fn refuse_evidence_extension(
    dir: &Path,
    task: &TaskId,
    act: &HumanAction,
) -> Result<Result<Option<std::fs::File>, Rejection>, String> {
    let mut act = act;
    while let HumanAction::Fenced { action, .. } = act {
        act = action;
    }
    // `extend-claim` gives a claims-mode run more budget just as `extend`
    // does (owner decision, 2026-11-04), so it is refused the same way.
    if !matches!(
        act,
        HumanAction::BudgetExtended { .. } | HumanAction::ClaimBudgetExtended { .. }
    ) {
        return Ok(Ok(None));
    }
    let admission = crate::moderator::evidence_admission(dir, task)?;
    let marker = crate::moderator::evidence_marker_path(dir, task);
    match std::fs::metadata(&marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ok(Some(admission))),
        Ok(_) => Ok(Err(Rejection::new(
            Code::BadHumanInput,
            format!(
                "{}: an evidence run's budget cannot be extended; it runs under the budget it was opened with",
                task.0
            ),
        ))),
        Err(e) => Err(format!(
            "{}: cannot tell whether this is an evidence run ({}: {e}), so the budget is not extended",
            task.0,
            marker.display()
        )),
    }
}

/// #597 G3: `holdout-updated` adopts only a suite staged for this task and
/// intact on disk; the core sees just a digest, so a typo or another task's
/// digest would otherwise retire the current suite for one no run can load.
fn unstaged(dir: &Path, task: &TaskId, act: &HumanAction) -> Option<Rejection> {
    let digest = match act {
        HumanAction::HoldoutUpdated { digest } => *digest,
        HumanAction::Fenced { action, .. } => return unstaged(dir, task, action),
        _ => return None,
    };
    let loaded = crate::holdout::load(dir, task, digest).and_then(|hs| hs.verify_files());
    loaded.err().map(|e| {
        Rejection::new(
            Code::HoldoutMismatch,
            format!("holdout {}: {e}", digest.hex()),
        )
    })
}

/// What `bbp human … --dry-run` prints: the events the action would append
/// and the response it would return at `rev`. Nothing is committed.
#[derive(Clone, Debug, PartialEq, Eq, rusty_serde::Serialize)]
pub struct Preview {
    pub rev: Rev,
    pub response: Response,
    pub events: Vec<rusty_bbp::event::Event>,
}

/// The fenced human command `perform_fenced` would dispatch.
fn command(
    verb: &str,
    args: &[&str],
    rev: Option<Rev>,
    by: &str,
    fence: Option<Rev>,
    op: Option<OpId>,
) -> Result<Result<Command, Rejection>, String> {
    let action = match action(verb, args, rev) {
        Ok(action) => action,
        Err(detail) if fence.is_some() => {
            return Ok(Err(Rejection::new(Code::BadHumanInput, detail)))
        }
        Err(error) => return Err(error),
    };
    let action = match fence {
        Some(expected_rev) => HumanAction::Fenced {
            expected_rev,
            action: Box::new(action),
        },
        None => action,
    };
    let op = match op {
        Some(op) => op,
        None => labelled_op(by)?,
    };
    Ok(Ok(Command::Human { op, action }))
}

/// Who may answer this action on `state`'s task (#447, D193): on an owned task a plan or merge
/// approval needs a board receipt bound to it, and answers under the receipt's op; anything
/// else, and every answer on a scratch task, keeps its own op (`Ok(None)`).
///
/// # Errors
/// A `BadHumanInput` rejection: an owned approval without a receipt, a receipt that does not
/// bind to this fenced approval, or a receipt on any other action.
pub fn owner_gate(
    state: &TaskState,
    action: &HumanAction,
    approval: Option<&Verified>,
) -> Result<Option<OpId>, Rejection> {
    let refuse = |why: String| Rejection::new(Code::BadHumanInput, why);
    let Some((approve, fence)) = crate::approval::approval_in(action) else {
        return match approval {
            Some(_) => Err(refuse(
                "--approval: only approve-plan and approve-merge take a receipt".into(),
            )),
            None => Ok(None),
        };
    };
    match (approval, state.gates) {
        (None, Some(Gates::Scratch)) => Ok(None),
        (None, None) => Err(refuse(format!(
            "{}: an owned task's plan and merge approvals need --approval, a receipt the board signs for an approve holder",
            state.task.0
        ))),
        (Some(v), _) => {
            crate::approval::binds(v.receipt(), &state.task, fence, approve).map_err(refuse)?;
            Ok(Some(crate::approval::op(v)))
        }
    }
}

/// `cmd` under [`owner_gate`]: its op replaced by the receipt's when one applies.
fn receipted(
    state: &TaskState,
    cmd: Command,
    approval: Option<&Verified>,
) -> Result<Command, Rejection> {
    let Command::Human { op, action } = cmd else {
        return Ok(cmd);
    };
    let op = owner_gate(state, &action, approval)?.unwrap_or(op);
    Ok(Command::Human { op, action })
}

/// [`perform_fenced`] without committing: the pure engine applied to the
/// task's current state, so the preview is what the commit would do at that
/// revision (#595 dry run).
#[allow(clippy::too_many_arguments)]
pub fn preview_fenced(
    dir: &Path,
    task: &TaskId,
    verb: &str,
    args: &[&str],
    rev: Option<Rev>,
    by: &str,
    fence: Option<Rev>,
    op: Option<OpId>,
    approval: Option<&Verified>,
) -> Result<Preview, String> {
    let d = open_driver(dir, task)?;
    let rev_now = d.state.rev;
    // A preview stores nothing: the brief is hashed, never written.
    let (args, _) = brief_ref(verb, args)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let cmd = match command(verb, &args, rev, by, fence, op)? {
        Ok(cmd) => cmd,
        Err(rej) => {
            return Ok(Preview {
                rev: rev_now,
                response: Response::Rejected(rej),
                events: vec![],
            })
        }
    };
    let cmd = match receipted(&d.state, cmd, approval) {
        Ok(cmd) => cmd,
        Err(rej) => {
            return Ok(Preview {
                rev: rev_now,
                response: Response::Rejected(rej),
                events: vec![],
            })
        }
    };
    if let Command::Human { action, .. } = &cmd {
        let refused = match unstaged(dir, task, action) {
            Some(rej) => Some(rej),
            None => refuse_evidence_extension(dir, task, action)?.err(),
        };
        if let Some(rej) = refused {
            return Ok(Preview {
                rev: rev_now,
                response: Response::Rejected(rej),
                events: vec![],
            });
        }
    }
    let out = handle(&d.state, &cmd, now());
    Ok(Preview {
        rev: rev_now,
        response: out.response,
        events: out.events,
    })
}

/// Human action with an optional transactional revision fence and retry op.
/// The host's existing unfenced CLI behavior remains available to direct users.
#[allow(clippy::too_many_arguments)]
pub fn perform_fenced(
    dir: &Path,
    task: &TaskId,
    verb: &str,
    args: &[&str],
    rev: Option<Rev>,
    by: &str,
    fence: Option<Rev>,
    op: Option<OpId>,
    approval: Option<&Verified>,
) -> Result<Response, String> {
    let (args, brief) = brief_ref(verb, args)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let cmd = match command(verb, &args, rev, by, fence, op)? {
        Ok(cmd) => cmd,
        Err(rej) => return Ok(Response::Rejected(rej)),
    };
    let Command::Human { action, .. } = &cmd else {
        return Err("not a human command".into());
    };
    if let Some(rej) = unstaged(dir, task, action) {
        return Ok(Response::Rejected(rej));
    }
    let _admission = match refuse_evidence_extension(dir, task, action)? {
        Ok(admission) => admission,
        Err(rej) if fence.is_some() => return Ok(Response::Rejected(rej)),
        Err(rej) => return Err(rej.detail),
    };
    let mut d = open_driver(dir, task)?;
    let cmd = match receipted(&d.state, cmd, approval) {
        Ok(cmd) => cmd,
        Err(rej) => return Ok(Response::Rejected(rej)),
    };
    let blobs: Vec<&[u8]> = [
        brief.as_deref(),
        approval.map(crate::approval::Verified::bytes),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !blobs.is_empty() {
        // Judge the command before its blobs (a brief, a receipt) are published: a refusal (bad
        // affects, a stale fence, the wrong state) and a replay of an op
        // already answered store nothing. Blobs before references: what
        // passes stores them just before the command that names them.
        // Only a write racing in between can still refuse it, and then the
        // stored blob is unreferenced, never a reference without its blob.
        let judged = handle(&d.state, &cmd, now());
        if matches!(judged.response, Response::Rejected(_)) || judged.events.is_empty() {
            return Ok(judged.response);
        }
        for bytes in blobs {
            d.store.blob_put(bytes);
        }
    }
    d.dispatch(&cmd, now()).map_err(|e| format!("{e:?}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn takeover_verbs_parse_and_refuse_bad_numbers() {
        let a = action(
            "takeover",
            &["msg:4", "agent:t:4:1", "nano", "15", "fix", "the", "path"],
            None,
        )
        .unwrap();
        assert_eq!(
            a,
            HumanAction::Takeover {
                claim: MsgId(4),
                session: "agent:t:4:1".into(),
                client: "nano".into(),
                minutes: 15,
                reason: "fix the path".into(),
            }
        );
        assert_eq!(
            action("takeover-release", &["takeover:2"], None).unwrap(),
            HumanAction::TakeoverRelease { takeover: 2 }
        );
        assert!(action("takeover", &["msg:4", "s", "c", "ten", "r"], None).is_err());
        assert!(action("takeover-release", &["x"], None).is_err());
    }

    #[test]
    fn the_label_round_trips_through_the_op() {
        let op = labelled_op("owner").unwrap();
        assert_eq!(answered_by(&op), "owner");
        assert_eq!(answered_by(&OpId("auto-00".into())), UNLABELLED);
        assert_eq!(answered_by(&OpId("by:x".into())), UNLABELLED);
    }

    /// A scratch directory holding task T1, opened as `admin::open_task` opens
    /// one; `evidence` also writes its evidence marker, as a capped `bbp mod`
    /// does on its first start.
    fn task(tag: &str, evidence: bool) -> (std::path::PathBuf, TaskId) {
        let dir = std::env::temp_dir().join(format!("bbp_human_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let task = TaskId("T1".into());
        crate::admin::open_task(
            &dir,
            &task,
            "local",
            b"brief",
            &PrincipalId("human".into()),
            &crate::profiles::ProfileSet::shell("test", "true"),
            crate::admin::read_roster(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/proving/roster.json"),
            )
            .unwrap(),
        )
        .unwrap();
        if evidence {
            crate::moderator::write_evidence_marker(&dir, &task, 5.0, "sha").unwrap();
        }
        (dir, task)
    }

    fn log_len(dir: &Path, task: &TaskId) -> usize {
        open_driver(dir, task)
            .unwrap()
            .store
            .events(task, 0)
            .unwrap()
            .len()
    }

    #[test]
    fn extend_on_an_evidence_task_is_refused_for_every_field_and_logs_nothing() {
        let (dir, task) = task("evidence-extend", true);
        let before = log_len(&dir, &task);
        for f in [
            "messages",
            "bytes",
            "reads",
            "reads_per_turn",
            "turns",
            "iterations",
        ] {
            let err = perform(&dir, &task, "extend", &[f, "1000"], None).unwrap_err();
            assert!(err.contains("evidence"), "{f}: {err}");
            let cli = perform_fenced(
                &dir,
                &task,
                "extend",
                &[f, "1000"],
                None,
                "o",
                None,
                None,
                None,
            )
            .unwrap_err();
            assert!(cli.contains("evidence"), "{f}: {cli}");
            let fenced = perform_fenced(
                &dir,
                &task,
                "extend",
                &[f, "1000"],
                None,
                "o",
                Some(Rev(1)),
                None,
                None,
            );
            assert!(
                matches!(&fenced, Ok(Response::Rejected(r))
                    if r.code == Code::BadHumanInput && r.detail.contains("evidence")),
                "{f}: {fenced:?}"
            );
        }
        assert_eq!(
            log_len(&dir, &task),
            before,
            "a refused extend appends nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dry_run_extend_on_an_evidence_task_previews_the_refusal() {
        let (dir, task) = task("evidence-preview", true);
        let p = preview_fenced(
            &dir,
            &task,
            "extend",
            &["messages", "1000"],
            None,
            "o",
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            matches!(&p.response, Response::Rejected(r)
                if r.code == Code::BadHumanInput && r.detail.contains("evidence")),
            "{p:?}"
        );
        assert!(p.events.is_empty(), "{p:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extend_on_a_task_with_no_evidence_marker_still_applies() {
        let (dir, task) = task("plain-extend", false);
        let before = log_len(&dir, &task);
        perform(&dir, &task, "extend", &["messages", "1000"], None).unwrap();
        let events = open_driver(&dir, &task)
            .unwrap()
            .store
            .events(&task, before)
            .unwrap();
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::BudgetExtended {
                    field: BudgetField::Messages,
                    limit: 1000
                }
            )),
            "{events:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_extend_racing_a_capped_first_start_waits_for_it_and_is_refused() {
        // The first capped start checks the log and writes the marker under
        // the evidence admission lock; an extend that arrives meanwhile must
        // see the marker, not append behind the start's back.
        let (dir, task) = task("racing-extend", false);
        let before = log_len(&dir, &task);
        let admission = crate::moderator::evidence_admission(&dir, &task).unwrap();
        let racer = {
            let (dir, task) = (dir.clone(), task.clone());
            std::thread::spawn(move || perform(&dir, &task, "extend", &["messages", "1000"], None))
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        crate::moderator::write_evidence_marker(&dir, &task, 5.0, "sha").unwrap();
        drop(admission);
        let err = racer.join().unwrap().unwrap_err();
        assert!(err.contains("evidence"), "{err}");
        assert_eq!(
            log_len(&dir, &task),
            before,
            "the racing extend appends nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn extend_refuses_when_the_evidence_marker_cannot_be_read() {
        // Unknown is never "not an evidence run": only a specific not-found
        // is. A file where the marker's directory belongs makes the lookup
        // fail with ENOTDIR.
        let (dir, task) = task("unreadable-marker", false);
        let marker = crate::moderator::evidence_marker_path(&dir, &task);
        std::fs::write(marker.parent().unwrap(), "not a directory").unwrap();
        let err = perform(&dir, &task, "extend", &["messages", "1000"], None).unwrap_err();
        assert!(err.contains("evidence"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_fenced_extend_reports_marker_and_lock_io_errors_as_host_errors() {
        // A confirmed evidence task is a policy refusal (BadHumanInput); an
        // I/O failure is not, so Orch sees a host error rather than "Answer
        // rejected" (README: I/O errors stay process failures).
        let fenced = |dir: &Path, task: &TaskId| {
            perform_fenced(
                dir,
                task,
                "extend",
                &["messages", "1000"],
                None,
                "o",
                Some(Rev(1)),
                None,
                None,
            )
        };
        let (dir, task) = task("fenced-marker-io", false);
        let marker = crate::moderator::evidence_marker_path(&dir, &task);
        std::fs::write(marker.parent().unwrap(), "not a directory").unwrap();
        let before = log_len(&dir, &task);
        let err = fenced(&dir, &task).unwrap_err();
        assert!(
            err.contains("cannot tell whether this is an evidence run"),
            "{err}"
        );
        let p = preview_fenced(
            &dir,
            &task,
            "extend",
            &["messages", "1000"],
            None,
            "o",
            Some(Rev(1)),
            None,
            None,
        );
        assert!(p.is_err(), "{p:?}");
        assert_eq!(log_len(&dir, &task), before);
        let _ = std::fs::remove_dir_all(&dir);

        let (dir2, task2) = super::tests::task("fenced-lock-io", false);
        let lock = crate::moderator::lock_path(&dir2, &task2);
        let _ = std::fs::remove_dir_all(lock.parent().unwrap());
        std::fs::write(lock.parent().unwrap(), "not a directory").unwrap();
        let before = log_len(&dir2, &task2);
        assert!(fenced(&dir2, &task2).is_err());
        assert_eq!(log_len(&dir2, &task2), before);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn a_decision_id_rides_the_op_and_keeps_the_label() {
        let op = decision_op("owner", "ab-12.3_x").unwrap();
        assert_eq!(op.0, "by:owner/dec:ab-12.3_x");
        assert_eq!(answered_by(&op), "owner");
        assert_eq!(decision_of(&op), Some("ab-12.3_x"));
        // Whoever answered, and never a fresh op.
        assert_eq!(decision_of(&OpId("by:other/dec:ab".into())), Some("ab"));
        assert_eq!(decision_of(&labelled_op("owner").unwrap()), None);
        assert_eq!(decision_of(&fresh_op()), None);
        assert_eq!(decision_of(&OpId("dec:ab".into())), None);
    }

    #[test]
    fn a_decision_id_that_could_break_the_op_is_refused() {
        for bad in ["", "a/b", "a b", "a,b", &"x".repeat(65)] {
            assert!(decision_op("owner", bad).is_err(), "{bad:?}");
        }
        assert!(decision_op("a/b", "ok").is_err());
        assert!(decision_op("owner", &"x".repeat(64)).is_ok());
    }

    fn decision_log(dir: &Path, task: &TaskId, id: &str) -> crate::decision::Decision {
        let events = open_driver(dir, task)
            .unwrap()
            .store
            .events(task, 0)
            .unwrap();
        crate::decision::lookup(&events, &PrincipalId("human".into()), &[id.to_owned()]).remove(0)
    }

    /// A send whose answer was lost is sent again with the same id and fence:
    /// it replays and logs nothing. A send the log moved past is refused, and
    /// the same id then reads as not applied, whatever was fenced.
    #[test]
    fn a_decision_id_settles_a_lost_answer_and_a_losing_send() {
        let (dir, task) = task("decision-id", false);
        let send = |id: &str, fence: u64, limit: &str| {
            let op = decision_op("owner", id).unwrap();
            perform_fenced(
                &dir,
                &task,
                "extend",
                &["messages", limit],
                None,
                "owner",
                Some(Rev(fence)),
                Some(op),
                None,
            )
            .unwrap()
        };
        let rev = open_driver(&dir, &task).unwrap().state.rev.0;
        assert!(!decision_log(&dir, &task, "d1").applied);

        assert!(!matches!(send("d1", rev, "1000"), Response::Rejected(_)));
        let applied = decision_log(&dir, &task, "d1");
        assert!(applied.applied && applied.rev > Some(rev), "{applied:?}");
        let len = log_len(&dir, &task);

        // The retry after a lost answer: same id, same fence.
        assert!(!matches!(send("d1", rev, "1000"), Response::Rejected(_)));
        assert_eq!(log_len(&dir, &task), len, "a replay logs nothing");
        assert_eq!(decision_log(&dir, &task, "d1"), applied);

        // A losing concurrent send fenced on the old rev: refused, not applied.
        let lost = send("d2", rev, "2000");
        assert!(
            matches!(&lost, Response::Rejected(r) if r.code == Code::StaleRev),
            "{lost:?}"
        );
        assert!(!decision_log(&dir, &task, "d2").applied);

        // The same id with another payload is a conflict, never a second apply.
        let again = send("d1", rev, "3000");
        assert!(
            matches!(&again, Response::Rejected(r) if r.code == Code::OpConflict),
            "{again:?}"
        );
        assert_eq!(decision_log(&dir, &task, "d1"), applied);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_op_comes_from_op_or_a_fenced_decision_never_both() {
        let id = |op: Option<OpId>| op.map(|o| o.0);
        assert_eq!(id(op_for("o", None, None, false).unwrap()), None);
        assert_eq!(
            id(op_for("o", Some("mine"), None, false).unwrap()),
            Some("mine".into())
        );
        assert_eq!(
            id(op_for("o", None, Some("d1"), true).unwrap()),
            Some("by:o/dec:d1".into())
        );
        assert!(op_for("o", Some("mine"), Some("d1"), true).is_err());
        let unfenced = op_for("o", None, Some("d1"), false).unwrap_err();
        assert!(unfenced.contains("--fence"), "{unfenced}");
        assert!(op_for("o", None, Some("a/b"), true).is_err());
    }

    /// `bbp decision` answers one row per id, in order; an empty, malformed or unknown id is
    /// a refusal or "not applied", never a guess.
    #[test]
    fn the_decision_lookup_answers_per_id_and_refuses_bad_ids() {
        let (dir, task) = task("decision-json", false);
        let rev = open_driver(&dir, &task).unwrap().state.rev.0;
        let op = decision_op("owner", "d1").unwrap();
        perform_fenced(
            &dir,
            &task,
            "extend",
            &["messages", "1000"],
            None,
            "owner",
            Some(Rev(rev)),
            Some(op),
            None,
        )
        .unwrap();
        let rows = crate::decision::json(&dir, &task, "d1,nope,d1").unwrap();
        let applied: Vec<bool> = rows
            .match_indices("\"applied\":")
            .map(|(i, m)| rows[i + m.len()..].starts_with("true"))
            .collect();
        assert_eq!(applied, [true, false, true], "{rows}");
        for bad in ["", "a/b", "d1,", ",d1", "x y"] {
            assert!(crate::decision::json(&dir, &task, bad).is_err(), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_label_that_could_break_the_op_is_refused() {
        for bad in ["", "a/b", "a b", &"x".repeat(33)] {
            assert!(labelled_op(bad).is_err(), "{bad:?}");
        }
    }
}
