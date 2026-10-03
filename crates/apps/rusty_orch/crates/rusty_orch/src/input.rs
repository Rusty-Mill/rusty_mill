//! The goal file: one JSON object holding the goal contract, the task list,
//! and optional routing. Parsed with `rusty_json` into `orch-core` drafts;
//! `orch-core` itself never sees JSON (ADR-0009).
//!
//! ```json
//! {
//!   "goal": "Find how Plan::start prevents self-review",
//!   "done_when": ["one finding citing the file"],
//!   "out_of_scope": ["code changes"],
//!   "wall_clock_secs": 600, "max_calls": 4, "stop": "checkpoint",
//!   "routing": { "research": "codex", "triage": "local", "reviewers": ["local", "codex"] },
//!   "tasks": [
//!     { "role": "research", "instruction": "...", "acceptance": ["..."],
//!       "refs": ["path:crates/apps/rusty_orch/AGENTS.md"], "max_calls": 2 },
//!     { "role": "review", "target": 0, "instruction": "...", "acceptance": ["..."],
//!       "depends_on": [0], "max_calls": 1 }
//!   ]
//! }
//! ```
//!
//! `depends_on` and a review's `target` are indices into `tasks`, and must
//! point at an earlier task. Refs use the adapter protocol's syntax:
//! `path:`, `commit:`, `url:`, or `E-<n>`.

use std::fmt;
use std::num::NonZeroU32;

use orch_core::goal::{Goal, GoalDraft, GoalError, StopRule};
use orch_core::task::{Agent, Plan, PlanError, Role, TaskSpec};
use orch_core::{GoalId, Ref, Text};
use orch_dispatch::RoutingConfig;
use rusty_json::Value;

/// Everything the file said, validated as far as the domain allows without
/// a plan: the goal is a real [`Goal`], tasks are still drafts because their
/// ids do not exist until [`build_plan`] adds them in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub goal: Goal,
    pub tasks: Vec<TaskDraft>,
    pub routing: RoutingConfig,
}

/// One task as written, with dependencies as indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDraft {
    pub role: RoleDraft,
    pub instruction: Text,
    pub acceptance: Vec<Text>,
    pub refs: Vec<Ref>,
    pub depends_on: Vec<usize>,
    pub max_calls: NonZeroU32,
}

/// A role whose review target is still an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleDraft {
    Research,
    Design,
    Implement,
    Triage,
    Review { target: usize },
}

/// Why the file was rejected. Every message names the JSON path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError(pub String);

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InputError {}

impl From<GoalError> for InputError {
    fn from(e: GoalError) -> Self {
        Self(e.to_string())
    }
}

/// Parse the goal file. Rejects unknown roles, agents, stop rules, bad refs,
/// forward or self references, and anything the goal contract refuses.
pub fn parse(json: &str) -> Result<Spec, InputError> {
    let root = Value::parse(json).map_err(|e| InputError(format!("goal file is not JSON: {e}")))?;
    if root.as_object().is_none() {
        return Err(InputError("goal file must be a JSON object".to_owned()));
    }
    let goal = Goal::try_from(goal_draft(&root)?)?;
    let tasks = root
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| InputError("tasks: required, an array".to_owned()))?;
    if tasks.is_empty() {
        return Err(InputError("tasks: must not be empty".to_owned()));
    }
    let tasks = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| task_draft(i, t))
        .collect::<Result<Vec<_>, _>>()?;
    let routing = match root.get("routing") {
        Some(v) => routing(v)?,
        None => default_routing(),
    };
    Ok(Spec {
        goal,
        tasks,
        routing,
    })
}

/// Add every task to a fresh plan in file order, resolving indices to ids.
pub fn build_plan(spec: &Spec, goal: GoalId) -> Result<Plan, InputError> {
    let mut plan = Plan::new(goal);
    let mut ids = Vec::with_capacity(spec.tasks.len());
    for (i, draft) in spec.tasks.iter().enumerate() {
        let id_of = |index: usize, what: &str| {
            ids.get(index).copied().ok_or_else(|| {
                InputError(format!(
                    "tasks[{i}].{what}: index {index} is not an earlier task"
                ))
            })
        };
        let role = match draft.role {
            RoleDraft::Research => Role::Research,
            RoleDraft::Design => Role::Design,
            RoleDraft::Implement => Role::Implement,
            RoleDraft::Triage => Role::Triage,
            RoleDraft::Review { target } => Role::Review {
                target: id_of(target, "target")?,
            },
        };
        let depends_on = draft
            .depends_on
            .iter()
            .map(|d| id_of(*d, "depends_on"))
            .collect::<Result<Vec<_>, _>>()?;
        let id = plan
            .add(TaskSpec {
                role,
                instruction: draft.instruction.clone(),
                acceptance: draft.acceptance.clone(),
                refs: draft.refs.clone(),
                depends_on,
                max_calls: draft.max_calls,
            })
            .map_err(|e: PlanError| InputError(format!("tasks[{i}]: {e}")))?;
        ids.push(id);
    }
    Ok(plan)
}

/// Codex does the thinking roles, the local model triages and reviews
/// first; Codex reviews what the local model wrote. Matches the examples.
pub fn default_routing() -> RoutingConfig {
    RoutingConfig {
        research: Agent::Codex,
        design: Agent::Codex,
        implement: Agent::Codex,
        triage: Agent::Local,
        reviewers: vec![Agent::Local, Agent::Codex],
    }
}

fn goal_draft(root: &Value) -> Result<GoalDraft, InputError> {
    Ok(GoalDraft {
        outcome: opt_string(root, "goal")?,
        done_when: strings(root, "done_when")?,
        in_scope: strings(root, "in_scope")?,
        out_of_scope: match root.get("out_of_scope") {
            Some(_) => Some(strings(root, "out_of_scope")?),
            None => None,
        },
        constraints: strings(root, "constraints")?,
        refs: strings(root, "refs")?,
        wall_clock_secs: opt_u64(root, "wall_clock_secs")?,
        max_calls: opt_u64(root, "max_calls")?
            .map(|n| u32::try_from(n).map_err(|_| InputError("max_calls: too large".to_owned())))
            .transpose()?,
        stop: match root.get("stop").map(Value::as_str) {
            None => None,
            Some(Some("checkpoint")) => Some(StopRule::Checkpoint),
            Some(Some("best_effort")) => Some(StopRule::BestEffort),
            Some(_) => {
                return Err(InputError(
                    "stop: expected \"checkpoint\" or \"best_effort\"".to_owned(),
                ))
            }
        },
    })
}

fn task_draft(i: usize, v: &Value) -> Result<TaskDraft, InputError> {
    let at = |field: &str| format!("tasks[{i}].{field}");
    if v.as_object().is_none() {
        return Err(InputError(format!("tasks[{i}]: must be an object")));
    }
    let role = match v.get("role").and_then(Value::as_str) {
        Some("research") => RoleDraft::Research,
        Some("design") => RoleDraft::Design,
        Some("implement") => RoleDraft::Implement,
        Some("triage") => RoleDraft::Triage,
        Some("review") => RoleDraft::Review {
            target: index(v, "target").map_err(|m| InputError(format!("{}: {m}", at("target"))))?,
        },
        Some(other) => {
            return Err(InputError(format!(
                "{}: unknown role {other:?}",
                at("role")
            )))
        }
        None => return Err(InputError(format!("{}: required", at("role")))),
    };
    let instruction = opt_string(v, "instruction")
        .map_err(|e| InputError(format!("tasks[{i}].{e}")))?
        .as_deref()
        .and_then(Text::new)
        .ok_or_else(|| InputError(format!("{}: required, non-blank", at("instruction"))))?;
    let acceptance = strings(v, "acceptance")
        .map_err(|e| InputError(format!("tasks[{i}].{e}")))?
        .iter()
        .map(|s| {
            Text::new(s).ok_or_else(|| InputError(format!("{}: blank entry", at("acceptance"))))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if acceptance.is_empty() {
        return Err(InputError(format!(
            "{}: required, non-empty",
            at("acceptance")
        )));
    }
    let refs = strings(v, "refs")
        .map_err(|e| InputError(format!("tasks[{i}].{e}")))?
        .iter()
        .map(|s| orch_cli::parse_ref(s).map_err(|m| InputError(format!("{}: {m}", at("refs")))))
        .collect::<Result<Vec<_>, _>>()?;
    let depends_on = match v.get("depends_on") {
        None => Vec::new(),
        Some(d) => d
            .as_array()
            .ok_or_else(|| {
                InputError(format!(
                    "{}: expected an array of indices",
                    at("depends_on")
                ))
            })?
            .iter()
            .map(|x| {
                x.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| InputError(format!("{}: expected an index", at("depends_on"))))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let max_calls = opt_u64(v, "max_calls")
        .map_err(|e| InputError(format!("tasks[{i}].{e}")))?
        .and_then(|n| u32::try_from(n).ok())
        .and_then(NonZeroU32::new)
        .ok_or_else(|| InputError(format!("{}: required, greater than zero", at("max_calls"))))?;
    Ok(TaskDraft {
        role,
        instruction,
        acceptance,
        refs,
        depends_on,
        max_calls,
    })
}

fn routing(v: &Value) -> Result<RoutingConfig, InputError> {
    let base = default_routing();
    let one = |field: &str, fallback: Agent| match v.get(field) {
        None => Ok(fallback),
        Some(a) => agent(a).map_err(|m| InputError(format!("routing.{field}: {m}"))),
    };
    let reviewers = match v.get("reviewers") {
        None => base.reviewers.clone(),
        Some(list) => list
            .as_array()
            .ok_or_else(|| InputError("routing.reviewers: expected an array".to_owned()))?
            .iter()
            .map(|a| agent(a).map_err(|m| InputError(format!("routing.reviewers: {m}"))))
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(RoutingConfig {
        research: one("research", base.research)?,
        design: one("design", base.design)?,
        implement: one("implement", base.implement)?,
        triage: one("triage", base.triage)?,
        reviewers,
    })
}

/// Agent names as the user writes them.
pub fn agent(v: &Value) -> Result<Agent, String> {
    match v.as_str() {
        Some("claude") => Ok(Agent::Claude),
        Some("codex") => Ok(Agent::Codex),
        Some("gemini") => Ok(Agent::Gemini),
        Some("local") => Ok(Agent::Local),
        Some(other) => Err(format!("unknown agent {other:?}")),
        None => Err("expected an agent name".to_owned()),
    }
}

fn index(v: &Value, field: &str) -> Result<usize, String> {
    v.get(field)
        .ok_or_else(|| "required".to_owned())?
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| "expected an index".to_owned())
}

fn opt_string(v: &Value, field: &str) -> Result<Option<String>, InputError> {
    match v.get(field) {
        None => Ok(None),
        Some(s) => s
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| InputError(format!("{field}: expected a string"))),
    }
}

fn strings(v: &Value, field: &str) -> Result<Vec<String>, InputError> {
    match v.get(field) {
        None => Ok(Vec::new()),
        Some(list) => list
            .as_array()
            .ok_or_else(|| InputError(format!("{field}: expected an array of strings")))?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| InputError(format!("{field}: expected an array of strings")))
            })
            .collect(),
    }
}

fn opt_u64(v: &Value, field: &str) -> Result<Option<u64>, InputError> {
    match v.get(field) {
        None => Ok(None),
        Some(n) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| InputError(format!("{field}: expected a non-negative integer"))),
    }
}
