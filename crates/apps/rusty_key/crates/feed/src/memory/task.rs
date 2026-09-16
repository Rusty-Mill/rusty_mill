//! Task State — the working-memory tier (PRD 03; data-model §8). A single
//! current goal + success criteria + declared scope, persisted to `task.json`.
//! The agent maintains it via the `set_task` / `complete_task` tools; it is
//! rendered into the per-turn oriented context (NOT the static system prompt).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool::ToolFn;
use rk_observe::ToolOutcome;

/// Task lifecycle (data-model §8): `idle | active | done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// No active task.
    #[default]
    Idle,
    /// A task is in progress.
    Active,
    /// The task was completed.
    Done,
}

/// The working-memory task record (PRD 03).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskState {
    /// Schema version (ADR-0027).
    #[serde(default = "one")]
    pub v: u32,
    /// The current goal (empty when idle).
    #[serde(default)]
    pub goal: String,
    /// Success criteria — all must be satisfied for completion.
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// Declared file/dir/crate scope (drift + BoundaryViolation heuristic).
    #[serde(default)]
    pub scope: Vec<String>,
    /// Lifecycle status.
    #[serde(default)]
    pub status: TaskStatus,
    /// Epoch seconds of the last change.
    #[serde(default)]
    pub updated_ts: f64,
}

fn one() -> u32 {
    1
}

/// Holds the current [`TaskState`], persisted to `task.json`.
pub struct TaskStore {
    state: Mutex<TaskState>,
    path: PathBuf,
}

impl TaskStore {
    /// Open (loading `task.json` if present) under `dir`.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("task.json");
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            state: Mutex::new(state),
            path,
        }
    }

    /// Set the active task. Returns the rendered confirmation.
    pub fn set_task(&self, goal: &str, criteria: Vec<String>, scope: Vec<String>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            s.v = 1;
            s.goal = goal.to_string();
            s.success_criteria = criteria;
            s.scope = scope;
            s.status = TaskStatus::Active;
            s.updated_ts = now();
        }
        self.save();
    }

    /// Mark the active task done.
    pub fn complete_task(&self) {
        {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            s.status = TaskStatus::Done;
            s.updated_ts = now();
        }
        self.save();
    }

    /// A snapshot of the current state.
    pub fn snapshot(&self) -> TaskState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The current goal (empty when idle/done with no goal).
    pub fn goal(&self) -> String {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .goal
            .clone()
    }

    /// The declared scope (for the BoundaryViolation heuristic, PRD 04).
    pub fn scope(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scope
            .clone()
    }

    /// The success criteria (for the CriteriaJudge, PRD 05).
    pub fn success_criteria(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .success_criteria
            .clone()
    }

    /// Render the compact task block for oriented context (empty when idle).
    pub fn render(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.status != TaskStatus::Active || s.goal.is_empty() {
            return String::new();
        }
        let mut out = format!("## Current task\nGoal: {}\n", s.goal);
        if !s.success_criteria.is_empty() {
            out.push_str("Success criteria:\n");
            for c in &s.success_criteria {
                out.push_str(&format!("- {c}\n"));
            }
        }
        if !s.scope.is_empty() {
            out.push_str(&format!("Scope: {}\n", s.scope.join(", ")));
        }
        out.push_str("Status: active");
        out
    }

    fn save(&self) {
        let snapshot = self.snapshot();
        if let Ok(json) = serde_json::to_string(&snapshot) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The `set_task` tool (PRD 03): sets goal + criteria + optional scope.
pub struct SetTaskTool {
    store: std::sync::Arc<TaskStore>,
}

/// The `complete_task` tool (PRD 03): marks the active task done.
pub struct CompleteTaskTool {
    store: std::sync::Arc<TaskStore>,
}

/// Register `set_task` / `complete_task` into the registry, backed by `store`.
pub fn register_task_tools(
    registry: &mut crate::tool::ToolRegistry,
    store: std::sync::Arc<TaskStore>,
) {
    registry.insert(Box::new(SetTaskTool {
        store: store.clone(),
    }));
    registry.insert(Box::new(CompleteTaskTool { store }));
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl ToolFn for SetTaskTool {
    fn name(&self) -> &str {
        "set_task"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string"},
                "success_criteria": {"type": "array", "items": {"type": "string"}},
                "scope": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["goal", "success_criteria"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let Some(goal) = args.get("goal").and_then(Value::as_str) else {
            return ToolOutcome::error("set_task: missing 'goal'");
        };
        let criteria = string_array(args.get("success_criteria"));
        let scope = string_array(args.get("scope"));
        self.store.set_task(goal, criteria, scope);
        ToolOutcome::ok(format!("task set: {goal}"))
    }
}

#[async_trait]
impl ToolFn for CompleteTaskTool {
    fn name(&self) -> &str {
        "complete_task"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"summary": {"type": "string"}},
            "required": ["summary"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let summary = args.get("summary").and_then(Value::as_str).unwrap_or("");
        self.store.complete_task();
        ToolOutcome::ok(format!("task complete: {summary}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn render_is_empty_when_idle_and_populated_when_active() {
        let dir = std::env::temp_dir().join(format!("rk-task-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TaskStore::open(&dir);
        assert!(store.render().is_empty());

        store.set_task(
            "add validation",
            vec!["rejects empty".into()],
            vec!["src/auth.rs".into()],
        );
        let block = store.render();
        assert!(block.contains("Goal: add validation"));
        assert!(block.contains("- rejects empty"));
        assert!(block.contains("Scope: src/auth.rs"));

        store.complete_task();
        assert!(store.render().is_empty()); // not active ⇒ empty
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("rk-task-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let store = TaskStore::open(&dir);
            store.set_task("goal x", vec!["c1".into()], vec![]);
        }
        let reopened = TaskStore::open(&dir);
        assert_eq!(reopened.goal(), "goal x");
        assert_eq!(reopened.success_criteria(), vec!["c1".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_task_tool_updates_store() {
        let dir = std::env::temp_dir().join(format!("rk-task-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(TaskStore::open(&dir));
        let tool = SetTaskTool {
            store: store.clone(),
        };
        let outcome = tool
            .call(serde_json::json!({"goal": "fix parser", "success_criteria": ["tests pass"]}))
            .await;
        assert_eq!(outcome.status, rk_observe::ToolStatus::Ok);
        assert_eq!(store.goal(), "fix parser");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
