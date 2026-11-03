//! The human channel as CLI actions. Every action carries a fresh `op`;
//! destructive ones (`reject`, `rerun`, `resume`, `cancel`) carry the card
//! revision the human saw (`--rev`, R15), never the one the CLI happens to
//! load, so a delayed command meets newer work with `stale_rev`.

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
        // #597 G3: adopt a staged holdout suite that changes or removes tests.
        "holdout-updated" => HumanAction::HoldoutUpdated {
            digest: Sha256::from_hex(a(0)?).ok_or("holdout digest must be 64 hex digits")?,
        },
        other => return Err(format!("unknown human action {other}")),
    })
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
    let op = labelled_op(by)?;
    let mut d = open_driver(dir, task)?;
    d.dispatch(&Command::Human { op, action: act }, now())
        .map_err(|e| format!("{e:?}"))
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
) -> Result<Preview, String> {
    let d = open_driver(dir, task)?;
    let rev_now = d.state.rev;
    let cmd = match command(verb, args, rev, by, fence, op)? {
        Ok(cmd) => cmd,
        Err(rej) => {
            return Ok(Preview {
                rev: rev_now,
                response: Response::Rejected(rej),
                events: vec![],
            })
        }
    };
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
) -> Result<Response, String> {
    let cmd = match command(verb, args, rev, by, fence, op)? {
        Ok(cmd) => cmd,
        Err(rej) => return Ok(Response::Rejected(rej)),
    };
    open_driver(dir, task)?
        .dispatch(&cmd, now())
        .map_err(|e| format!("{e:?}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_label_round_trips_through_the_op() {
        let op = labelled_op("owner").unwrap();
        assert_eq!(answered_by(&op), "owner");
        assert_eq!(answered_by(&OpId("auto-00".into())), UNLABELLED);
        assert_eq!(answered_by(&OpId("by:x".into())), UNLABELLED);
    }

    #[test]
    fn a_label_that_could_break_the_op_is_refused() {
        for bad in ["", "a/b", "a b", &"x".repeat(33)] {
            assert!(labelled_op(bad).is_err(), "{bad:?}");
        }
    }
}
