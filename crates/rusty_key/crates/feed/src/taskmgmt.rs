//! Task-management tools (PRD 03; Phase 6, #11). An in-session registry of work
//! items the agent initiates (e.g. tracking subagent / long-running operations).
//! Distinct from the working-memory `TaskStore` (goal + criteria). Tasks live
//! for the session lifetime only — not persisted across sessions.
//!
//! v1: a record registry (create/get/list/update/stop/output). Actually
//! *spawning* background `tokio` work with a `CancellationToken` is the
//! multi-agent-orchestration follow-up (BACKLOG post-phase); `task_stop` marks
//! the record stopped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rk_observe::ToolOutcome;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool::{ToolFn, ToolRegistry};

/// Lifecycle of a background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRunStatus {
    /// Created, not yet finished.
    Running,
    /// Completed.
    Done,
    /// Cancelled via `task_stop`.
    Stopped,
}

/// One tracked task.
#[derive(Debug, Clone)]
struct TaskRecord {
    id: String,
    description: String,
    status: TaskRunStatus,
    output: Vec<String>,
}

/// In-session registry of background tasks.
#[derive(Default)]
pub struct BackgroundTaskStore {
    tasks: Mutex<HashMap<String, TaskRecord>>,
    counter: AtomicUsize,
}

impl BackgroundTaskStore {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskRecord>> {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn create(&self, description: &str) -> String {
        let id = format!("task_{}", self.counter.fetch_add(1, Ordering::Relaxed));
        self.lock().insert(
            id.clone(),
            TaskRecord {
                id: id.clone(),
                description: description.to_string(),
                status: TaskRunStatus::Running,
                output: Vec::new(),
            },
        );
        id
    }

    fn update(&self, id: &str, note: &str) -> bool {
        match self.lock().get_mut(id) {
            Some(t) => {
                t.output.push(note.to_string());
                true
            }
            None => false,
        }
    }

    fn set_status(&self, id: &str, status: TaskRunStatus) -> bool {
        match self.lock().get_mut(id) {
            Some(t) => {
                t.status = status;
                true
            }
            None => false,
        }
    }

    fn get_line(&self, id: &str) -> Option<String> {
        self.lock().get(id).map(|t| {
            format!(
                "{} [{}]: {}\n{}",
                t.id,
                status_str(t.status),
                t.description,
                t.output.join("\n")
            )
        })
    }

    fn output(&self, id: &str) -> Option<String> {
        self.lock().get(id).map(|t| t.output.join("\n"))
    }

    fn list(&self) -> String {
        let tasks = self.lock();
        let mut lines: Vec<String> = tasks
            .values()
            .map(|t| format!("{} [{}]: {}", t.id, status_str(t.status), t.description))
            .collect();
        lines.sort();
        lines.join("\n")
    }
}

fn status_str(s: TaskRunStatus) -> &'static str {
    match s {
        TaskRunStatus::Running => "running",
        TaskRunStatus::Done => "done",
        TaskRunStatus::Stopped => "stopped",
    }
}

/// Which task-management operation a [`TaskMgmtTool`] performs.
#[derive(Clone, Copy)]
enum Op {
    Create,
    Get,
    List,
    Update,
    Stop,
    Output,
}

struct TaskMgmtTool {
    op: Op,
    store: Arc<BackgroundTaskStore>,
}

fn arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

#[async_trait]
impl ToolFn for TaskMgmtTool {
    fn name(&self) -> &str {
        match self.op {
            Op::Create => "task_create",
            Op::Get => "task_get",
            Op::List => "task_list",
            Op::Update => "task_update",
            Op::Stop => "task_stop",
            Op::Output => "task_output",
        }
    }

    fn schema(&self) -> Value {
        match self.op {
            Op::Create => serde_json::json!({
                "type": "object",
                "properties": {"description": {"type": "string"}},
                "required": ["description"]
            }),
            Op::List => serde_json::json!({"type": "object", "properties": {}}),
            Op::Update => serde_json::json!({
                "type": "object",
                "properties": {"id": {"type": "string"}, "note": {"type": "string"}},
                "required": ["id", "note"]
            }),
            // get / stop / output
            _ => serde_json::json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let missing = || ToolOutcome::error(format!("{}: missing required argument", self.name()));
        match self.op {
            Op::Create => match arg(&args, "description") {
                Some(d) => ToolOutcome::ok(self.store.create(&d)),
                None => missing(),
            },
            Op::List => ToolOutcome::ok(self.store.list()),
            Op::Get => match arg(&args, "id").and_then(|id| self.store.get_line(&id)) {
                Some(line) => ToolOutcome::ok(line),
                None => ToolOutcome::error("task_get: unknown id"),
            },
            Op::Output => match arg(&args, "id").and_then(|id| self.store.output(&id)) {
                Some(out) => ToolOutcome::ok(out),
                None => ToolOutcome::error("task_output: unknown id"),
            },
            Op::Update => match (arg(&args, "id"), arg(&args, "note")) {
                (Some(id), Some(note)) if self.store.update(&id, &note) => {
                    ToolOutcome::ok(format!("updated {id}"))
                }
                (Some(_), Some(_)) => ToolOutcome::error("task_update: unknown id"),
                _ => missing(),
            },
            Op::Stop => match arg(&args, "id") {
                Some(id) if self.store.set_status(&id, TaskRunStatus::Stopped) => {
                    ToolOutcome::ok(format!("stopped {id}"))
                }
                Some(_) => ToolOutcome::error("task_stop: unknown id"),
                None => missing(),
            },
        }
    }
}

/// Register the six task-management tools, backed by a shared registry.
pub fn register_task_management_tools(
    registry: &mut ToolRegistry,
    store: Arc<BackgroundTaskStore>,
) {
    for op in [
        Op::Create,
        Op::Get,
        Op::List,
        Op::Update,
        Op::Stop,
        Op::Output,
    ] {
        registry.insert(Box::new(TaskMgmtTool {
            op,
            store: store.clone(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rk_observe::ToolStatus;
    use serde_json::json;

    #[tokio::test]
    async fn create_update_get_stop_lifecycle() {
        let store = Arc::new(BackgroundTaskStore::new());
        let create = TaskMgmtTool {
            op: Op::Create,
            store: store.clone(),
        };
        let id = create
            .call(json!({"description": "index the repo"}))
            .await
            .payload;

        let update = TaskMgmtTool {
            op: Op::Update,
            store: store.clone(),
        };
        assert_eq!(
            update
                .call(json!({"id": id, "note": "scanned 10 files"}))
                .await
                .status,
            ToolStatus::Ok
        );

        let get = TaskMgmtTool {
            op: Op::Get,
            store: store.clone(),
        };
        let view = get.call(json!({"id": id})).await;
        assert!(view.payload.contains("running"));
        assert!(view.payload.contains("scanned 10 files"));

        let stop = TaskMgmtTool {
            op: Op::Stop,
            store: store.clone(),
        };
        stop.call(json!({"id": id})).await;
        let after = get.call(json!({"id": id})).await;
        assert!(after.payload.contains("stopped"));

        let list = TaskMgmtTool {
            op: Op::List,
            store,
        };
        assert!(list
            .call(json!({}))
            .await
            .payload
            .contains("index the repo"));
    }

    #[tokio::test]
    async fn unknown_id_is_an_error() {
        let store = Arc::new(BackgroundTaskStore::new());
        let get = TaskMgmtTool { op: Op::Get, store };
        assert_eq!(
            get.call(json!({"id": "nope"})).await.status,
            ToolStatus::Error
        );
    }
}
