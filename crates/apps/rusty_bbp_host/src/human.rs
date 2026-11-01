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
    refuse_evidence_extension(dir, task, &act)?;
    let op = labelled_op(by)?;
    let mut d = open_driver(dir, task)?;
    d.dispatch(&Command::Human { op, action: act }, now())
        .map_err(|e| format!("{e:?}"))
}

/// An evidence run runs under the budget its task was opened with (owner
/// decision, 2026-11-01), so `extend` refuses, for every field, on a task
/// that carries an evidence marker. Only a specific not-found means there is
/// no marker; any other error reading it refuses too. `bbp pairs` refuses a
/// run whose log holds an extension anyway (one logged before the marker was
/// written, or by an older `bbp`), so this check is the early, operator-facing
/// half and not the only one.
fn refuse_evidence_extension(dir: &Path, task: &TaskId, act: &HumanAction) -> Result<(), String> {
    let mut act = act;
    while let HumanAction::Fenced { action, .. } = act {
        act = action;
    }
    if !matches!(act, HumanAction::BudgetExtended { .. }) {
        return Ok(());
    }
    let marker = crate::moderator::evidence_marker_path(dir, task);
    match std::fs::metadata(&marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "{}: an evidence run's budget cannot be extended; it runs under the budget it was opened with",
            task.0
        )),
        Err(e) => Err(format!(
            "{}: cannot tell whether this is an evidence run ({}: {e}), so the budget is not extended",
            task.0,
            marker.display()
        )),
    }
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
    let action = match action(verb, args, rev) {
        Ok(action) => action,
        Err(detail) if fence.is_some() => {
            return Ok(Response::Rejected(Rejection::new(
                Code::BadHumanInput,
                detail,
            )))
        }
        Err(error) => return Err(error),
    };
    if let Err(detail) = refuse_evidence_extension(dir, task, &action) {
        return match fence {
            Some(_) => Ok(Response::Rejected(Rejection::new(Code::Forbidden, detail))),
            None => Err(detail),
        };
    }
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
    open_driver(dir, task)?
        .dispatch(&Command::Human { op, action }, now())
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
            let fenced = perform_fenced(
                &dir,
                &task,
                "extend",
                &[f, "1000"],
                None,
                "o",
                Some(Rev(1)),
                None,
            );
            assert!(
                matches!(&fenced, Ok(Response::Rejected(r)) if r.detail.contains("evidence")),
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

    #[test]
    fn a_label_that_could_break_the_op_is_refused() {
        for bad in ["", "a/b", "a b", &"x".repeat(33)] {
            assert!(labelled_op(bad).is_err(), "{bad:?}");
        }
    }
}
