//! Open a task and assign roles: moderator operations a host runs once.

use crate::profiles::{self, ProfileSet};
use crate::{now, open_driver};
use rusty_bbp::*;
use std::path::Path;

pub fn parse_role(s: &str) -> Result<Role, String> {
    match s {
        "planner" => Ok(Role::Planner),
        "coder" => Ok(Role::Coder),
        "tester" => Ok(Role::Tester),
        "reviewer" => Ok(Role::Reviewer),
        other => Err(format!("unknown role {other}")),
    }
}

pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Planner => "planner",
        Role::Coder => "coder",
        Role::Tester => "tester",
        Role::Reviewer => "reviewer",
    }
}

/// Open `task` in the store at `dir` with `brief` as its brief. The profile
/// set is frozen first; its digest is the task's `profile_digest`. A task
/// that is already open keeps its frozen file: the core's `AlreadyOpen`
/// rejection is returned before anything is written.
pub fn open_task(
    dir: &Path,
    task: &TaskId,
    repo: &str,
    brief: &[u8],
    human: &PrincipalId,
    set: &ProfileSet,
) -> Result<Response, String> {
    let store = FsStore::open(dir).map_err(|e| e.to_string())?;
    let mut d = Driver::new(store, task.clone());
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
        brief: blob,
        human: human.clone(),
    });
    d.dispatch(&cmd, now()).map_err(|e| format!("{e:?}"))
}

pub fn assign(
    dir: &Path,
    task: &TaskId,
    role: Role,
    principal: &PrincipalId,
    vendor: &str,
) -> Result<Response, String> {
    let mut d = open_driver(dir, task)?;
    let cmd = Command::Assign {
        role,
        principal: Principal {
            id: principal.clone(),
            kind: PrincipalKind::Agent {
                role,
                vendor: vendor.to_owned(),
            },
        },
    };
    d.dispatch(&cmd, now()).map_err(|e| format!("{e:?}"))
}

pub fn card(dir: &Path, task: &TaskId) -> Result<Card, String> {
    Ok(open_driver(dir, task)?.state.card())
}

pub fn tick(dir: &Path, task: &TaskId) -> Result<Response, String> {
    open_driver(dir, task)?
        .dispatch(&Command::Tick, now())
        .map_err(|e| format!("{e:?}"))
}
