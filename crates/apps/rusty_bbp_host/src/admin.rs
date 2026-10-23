//! Open a task and assign roles: moderator operations a host runs once.

use crate::profiles::{self, ProfileSet};
use crate::{now, open_driver};
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::path::Path;

/// Read a roster file: a JSON array of roles (see the spec's Role table).
/// The roles are the orchestrator's to define; the host only passes them on,
/// and the core validates them when the task opens.
pub fn read_roster(file: &Path) -> Result<Vec<Role>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
}

/// A policy name as `bbp open --policy` spells it.
pub fn parse_policy(name: &str) -> Result<SchedulingPolicy, String> {
    match name {
        "turn-based" => Ok(SchedulingPolicy::TurnBased),
        "event-driven" => Ok(SchedulingPolicy::EventDriven),
        other => Err(format!(
            "unknown --policy {other}: turn-based or event-driven"
        )),
    }
}

/// The policy a task opens under when none is named: event-driven, which
/// allows one live turn, and turn-based for a task that allows several.
pub fn default_policy(max_live_turns: u32) -> SchedulingPolicy {
    if max_live_turns > 1 {
        SchedulingPolicy::TurnBased
    } else {
        SchedulingPolicy::EventDriven
    }
}

/// Open `task` in the store at `dir` with `brief` as its brief. The profile
/// set is frozen first; its digest is the task's `profile_digest`. A task
/// that is already open keeps its frozen file: the core's `AlreadyOpen`
/// rejection is returned before anything is written. `roster` is recorded in
/// the log and cannot change; an invalid one is refused with `BadRoster`.
pub fn open_task(
    dir: &Path,
    task: &TaskId,
    repo: &str,
    brief: &[u8],
    human: &PrincipalId,
    set: &ProfileSet,
    roster: Vec<Role>,
) -> Result<Response, String> {
    open_task_live(
        dir,
        task,
        repo,
        brief,
        human,
        set,
        roster,
        1,
        default_policy(1),
    )
}

/// As [`open_task`], allowing `max_live_turns` concurrent turns (1 to the
/// roster size; the core refuses any other value with `BadLiveTurns`) and
/// scheduling under `policy`. The moderator launches one harness per live turn.
#[allow(clippy::too_many_arguments)]
pub fn open_task_live(
    dir: &Path,
    task: &TaskId,
    repo: &str,
    brief: &[u8],
    human: &PrincipalId,
    set: &ProfileSet,
    roster: Vec<Role>,
    max_live_turns: u32,
    policy: SchedulingPolicy,
) -> Result<Response, String> {
    let master = crate::master::ensure(dir)?;
    let store = FsStore::open(dir).map_err(|e| e.to_string())?;
    let mut d = Driver::new(store, task.clone()).with_master(master);
    let _ = d.reload();
    if d.state.opened {
        return Ok(Response::Rejected(Rejection::new(
            Code::AlreadyOpen,
            "task already opened",
        )));
    }
    let profile_digest = profiles::freeze(dir, task, set)?;
    let blob = d.store.blob_put(brief);
    let cmd = Command::Open(OpenTask {
        task: task.clone(),
        roster,
        repo: repo.to_owned(),
        profile_digest,
        budget: Budget {
            messages: 40,
            bytes: 5_000_000,
            reads: 150,
            reads_per_turn: 30,
            turns: 60,
            iterations: 4,
        },
        turn_ms: 600_000,
        request_ms: 86_400_000,
        max_live_turns,
        brief: blob,
        human: human.clone(),
        policy,
    });
    d.dispatch(&cmd, now()).map_err(|e| format!("{e:?}"))
}

pub fn assign(
    dir: &Path,
    task: &TaskId,
    role: &RoleId,
    principal: &PrincipalId,
    vendor: &str,
) -> Result<Response, String> {
    let mut d = open_driver(dir, task)?;
    let cmd = Command::Assign {
        role: role.clone(),
        principal: Principal {
            id: principal.clone(),
            kind: PrincipalKind::Agent {
                role: role.clone(),
                vendor: vendor.to_owned(),
            },
        },
    };
    d.dispatch(&cmd, now()).map_err(|e| format!("{e:?}"))
}

pub fn card(dir: &Path, task: &TaskId) -> Result<Card, String> {
    Ok(open_driver(dir, task)?.state.card())
}

/// What `bbp card` prints: the core's card and the task's scheduling policy,
/// which the card does not carry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CardView {
    #[rusty_serde(flatten)]
    pub card: Card,
    pub policy: SchedulingPolicy,
}

pub fn card_view(dir: &Path, task: &TaskId) -> Result<CardView, String> {
    let d = open_driver(dir, task)?;
    Ok(CardView {
        card: d.state.card(),
        policy: d.state.policy,
    })
}

pub fn tick(dir: &Path, task: &TaskId) -> Result<Response, String> {
    open_driver(dir, task)?
        .dispatch(&Command::Tick, now())
        .map_err(|e| format!("{e:?}"))
}

/// Human gate details and terminal facts from a single log prefix.
/// A missing task is represented by null, distinct from an I/O failure.
pub fn show(dir: &Path, task: &TaskId) -> Result<Option<TaskView>, String> {
    if !log_path_for(dir, task).exists() {
        return Ok(None);
    }
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    Ok(Some(TaskView::from_events(task.clone(), &events)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_names_parse_and_anything_else_is_refused() {
        assert_eq!(parse_policy("turn-based"), Ok(SchedulingPolicy::TurnBased));
        assert_eq!(
            parse_policy("event-driven"),
            Ok(SchedulingPolicy::EventDriven)
        );
        assert!(matches!(parse_policy("EventDriven"), Err(e) if e.contains("--policy")));
        assert!(parse_policy("").is_err());
    }

    #[test]
    fn new_tasks_default_to_event_driven_unless_several_turns_are_live() {
        assert_eq!(default_policy(1), SchedulingPolicy::EventDriven);
        assert_eq!(default_policy(2), SchedulingPolicy::TurnBased);
        // Replay of a log without the field stays turn-based.
        assert_eq!(SchedulingPolicy::default(), SchedulingPolicy::TurnBased);
    }
}
