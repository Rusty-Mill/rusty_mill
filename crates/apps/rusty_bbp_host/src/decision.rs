//! `bbp decision`: was a caller-chosen decision id applied? A pure read.
//!
//! `bbp human --decision ID` writes its op as `by:<by>/dec:ID` (see
//! [`crate::human::decision_op`]), and every accepted human command is logged
//! as an `op_accepted` carrying that op. So the log already answers, with no
//! new event: an id is applied when an `op_accepted` carries it, and a send
//! that was refused wrote only `rejected`. Old logs hold no such op and read
//! as "not applied".

use crate::{human, open_driver};
use rusty_bbp::event::Event;
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::path::Path;

/// One decision id and what the log says of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub decision: String,
    pub applied: bool,
    /// The task revision just after the `op_accepted` event that carries the
    /// id (its position in the log, counted from 1): fence the next command
    /// on at least this. `None` when not applied.
    pub rev: Option<u64>,
}

/// What `events` say of each of `ids`, in order. The first `op_accepted` of an
/// id counts; a replay writes none. Only the task's `human` principal counts:
/// an agent chooses its own op strings, so one written as `by:x/dec:ID` is no
/// answer of the human's.
pub fn lookup(events: &[Event], human: &PrincipalId, ids: &[String]) -> Vec<Decision> {
    ids.iter()
        .map(|id| {
            let at = events.iter().position(|e| {
                matches!(e, Event::OpAccepted { principal, op, .. }
                    if principal == human && human::decision_of(op) == Some(id))
            });
            Decision {
                decision: id.clone(),
                applied: at.is_some(),
                rev: at.map(|i| i as u64 + 1),
            }
        })
        .collect()
}

/// The JSON `bbp decision` prints: one row per id in `ids` (comma-separated).
pub fn json(dir: &Path, task: &TaskId, ids: &str) -> Result<String, String> {
    let ids: Vec<String> = ids.split(',').map(str::to_owned).collect();
    if let Some(bad) = ids.iter().find(|i| !human::valid_decision(i)) {
        return Err(format!("bad decision id {bad:?}"));
    }
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    rusty_serde::json::to_string(&lookup(&events, &d.state.human, &ids)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn human() -> PrincipalId {
        PrincipalId("human".into())
    }

    fn accepted(op: &str) -> Event {
        accepted_by(human(), op)
    }

    fn accepted_by(principal: PrincipalId, op: &str) -> Event {
        Event::OpAccepted {
            principal,
            op: OpId(op.into()),
            payload: Sha256::of(b"x"),
            response: Response::Ok,
        }
    }

    #[test]
    fn an_id_is_applied_at_the_position_after_its_op_whoever_answered() {
        let log = [
            accepted("auto-00"),
            accepted("by:nano/dec:a.0"),
            accepted("by:other/dec:b"),
        ];
        let ids = ["a.0", "b", "c"].map(str::to_owned);
        let got = lookup(&log, &human(), &ids);
        assert_eq!(
            got.iter().map(|d| (d.applied, d.rev)).collect::<Vec<_>>(),
            [(true, Some(2)), (true, Some(3)), (false, None)]
        );
    }

    #[test]
    fn an_old_log_without_ids_reads_as_not_applied() {
        let log = [accepted("auto-00"), accepted("by:nano/ff00")];
        assert!(!lookup(&log, &human(), &["auto-00".to_owned()])[0].applied);
    }

    /// An agent picks its own op strings, so an op shaped like the human's decision
    /// op, written by another principal, is not an answer.
    #[test]
    fn an_agent_op_that_imitates_a_decision_id_does_not_count() {
        let log = [accepted_by(PrincipalId("agent:x".into()), "by:nano/dec:a")];
        assert!(!lookup(&log, &human(), &["a".to_owned()])[0].applied);
    }
}
