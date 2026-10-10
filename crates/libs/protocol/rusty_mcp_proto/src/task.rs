//! The tasks extension (`io.modelcontextprotocol/tasks`): a `tools/call`
//! may answer with a task; the client polls `tasks/get`, answers requests
//! with `tasks/update` and may `tasks/cancel`.

use crate::codec::{object, opt_string, opt_u64, opt_value, string, Obj, Wire};
use crate::input::{InputRequests, InputRequiredResult};
use crate::page::ResultType;
use crate::{Error, Result};
use rusty_json::Value;

/// The extension identifier a server lists under `capabilities.extensions`.
pub const EXTENSION_ID: &str = "io.modelcontextprotocol/tasks";

/// Method names for tasks.
pub mod method {
    /// Read a task's state.
    pub const GET: &str = "tasks/get";
    /// Answer a task's pending input requests.
    pub const UPDATE: &str = "tasks/update";
    /// Ask a task to stop.
    pub const CANCEL: &str = "tasks/cancel";
    /// A task changed state.
    pub const STATUS_NOTIFICATION: &str = "notifications/tasks";
}

/// Where a task is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    /// Running.
    Working,
    /// Waiting for the client to answer requests.
    InputRequired,
    /// Done; the payload carries `result`.
    Completed,
    /// Failed; the payload carries `error`.
    Failed,
    /// Stopped on request.
    Cancelled,
}

impl TaskStatus {
    /// Whether the task will not change again.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::InputRequired => "input_required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        match s {
            "working" => Ok(Self::Working),
            "input_required" => Ok(Self::InputRequired),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(Error::decode("task status", format!("unknown {other:?}"))),
        }
    }
}

/// The fields every task has.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// Identifier chosen by the server.
    pub task_id: String,
    /// Current state.
    pub status: TaskStatus,
    /// A status line.
    pub status_message: Option<String>,
    /// ISO 8601 creation time.
    pub created_at: String,
    /// ISO 8601 time of the last change.
    pub last_updated_at: String,
    /// How long the server keeps the task; `None` means unlimited. Always
    /// present on the wire, as `null` when unlimited.
    pub ttl_ms: Option<u64>,
    /// Suggested polling interval.
    pub poll_interval_ms: Option<u64>,
}

impl Task {
    /// A task with only the required fields.
    pub fn new(
        task_id: impl Into<String>,
        status: TaskStatus,
        created_at: impl Into<String>,
        last_updated_at: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            status,
            status_message: None,
            created_at: created_at.into(),
            last_updated_at: last_updated_at.into(),
            ttl_ms: None,
            poll_interval_ms: None,
        }
    }

    fn encode(&self, obj: Obj) -> Obj {
        obj.set("taskId", self.task_id.as_str())
            .set("status", self.status.as_str())
            .opt("statusMessage", self.status_message.clone())
            .set("createdAt", self.created_at.as_str())
            .set("lastUpdatedAt", self.last_updated_at.as_str())
            .set("ttlMs", self.ttl_ms.map_or(Value::Null, Value::from))
            .opt("pollIntervalMs", self.poll_interval_ms)
    }

    fn decode(v: &Value) -> Result<Self> {
        const W: &str = "Task";
        Ok(Self {
            task_id: string(v, W, "taskId")?,
            status: TaskStatus::parse(&string(v, W, "status")?)?,
            status_message: opt_string(v, W, "statusMessage")?,
            created_at: string(v, W, "createdAt")?,
            last_updated_at: string(v, W, "lastUpdatedAt")?,
            ttl_ms: opt_u64(v, W, "ttlMs")?,
            poll_interval_ms: opt_u64(v, W, "pollIntervalMs")?,
        })
    }
}

/// What a task's state carries besides the common fields. The status
/// decides which payload member must be present.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskPayload {
    /// Running.
    Working,
    /// Waiting on the client.
    InputRequired {
        /// What to answer through `tasks/update`.
        input_requests: InputRequests,
    },
    /// Done.
    Completed {
        /// The final result of the original call, a JSON object.
        result: Value,
    },
    /// Failed.
    Failed {
        /// Why, a JSON object.
        error: Value,
    },
    /// Stopped.
    Cancelled,
}

impl TaskPayload {
    /// The status this payload implies.
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Working => TaskStatus::Working,
            Self::InputRequired { .. } => TaskStatus::InputRequired,
            Self::Completed { .. } => TaskStatus::Completed,
            Self::Failed { .. } => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

/// A task with its payload: what `tasks/get` returns and `notifications/tasks`
/// carries. Built so the status and the payload cannot disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailedTask {
    /// The common fields; `status` always equals `payload.status()`.
    task: Task,
    /// The status-specific member.
    payload: TaskPayload,
}

impl DetailedTask {
    /// A task in the state `payload` describes (overrides `task.status`).
    pub fn new(mut task: Task, payload: TaskPayload) -> Self {
        task.status = payload.status();
        Self { task, payload }
    }

    /// The common fields.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// The status-specific member.
    pub fn payload(&self) -> &TaskPayload {
        &self.payload
    }

    fn encode(&self, obj: Obj) -> Obj {
        let obj = self.task.encode(obj);
        match &self.payload {
            TaskPayload::Working | TaskPayload::Cancelled => obj,
            TaskPayload::InputRequired { input_requests } => {
                let mut requests = Value::object();
                for (k, r) in input_requests {
                    requests.insert(k.as_str(), r.to_value());
                }
                obj.set("inputRequests", requests)
            }
            TaskPayload::Completed { result } => obj.set("result", result.clone()),
            TaskPayload::Failed { error } => obj.set("error", error.clone()),
        }
    }

    fn decode(v: &Value) -> Result<Self> {
        const W: &str = "DetailedTask";
        let task = Task::decode(v)?;
        let need = |name: &str| {
            opt_value(v, name).ok_or_else(|| Error::decode(W, format!("status needs {name:?}")))
        };
        let payload = match task.status {
            TaskStatus::Working => TaskPayload::Working,
            TaskStatus::Cancelled => TaskPayload::Cancelled,
            TaskStatus::InputRequired => {
                // Reuse the input-required codec for the request map.
                let mut holder = Value::object();
                holder.insert("resultType", ResultType::INPUT_REQUIRED);
                holder.insert("inputRequests", need("inputRequests")?);
                let parsed = InputRequiredResult::from_value(&holder)?;
                TaskPayload::InputRequired {
                    input_requests: parsed.input_requests.unwrap_or_default(),
                }
            }
            TaskStatus::Completed => TaskPayload::Completed {
                result: object(&need("result")?, W)?.clone(),
            },
            TaskStatus::Failed => TaskPayload::Failed {
                error: object(&need("error")?, W)?.clone(),
            },
        };
        Ok(Self { task, payload })
    }
}

/// The answer to a `tools/call` that continues as a task
/// (`resultType: "task"`).
#[derive(Clone, Debug, PartialEq)]
pub struct CreateTaskResult {
    /// The new task.
    pub task: Task,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for CreateTaskResult {
    fn to_value(&self) -> Value {
        self.task
            .encode(Obj::new().set("resultType", ResultType::TASK))
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "CreateTaskResult";
        object(v, W)?;
        if opt_string(v, W, "resultType")?.as_deref() != Some(ResultType::TASK) {
            return Err(Error::decode(W, "resultType is not \"task\""));
        }
        Ok(Self {
            task: Task::decode(v)?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Parameters of `tasks/get` and `tasks/cancel`.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskIdParams {
    /// Which task.
    pub task_id: String,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for TaskIdParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("taskId", self.task_id.as_str())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "TaskIdParams")?;
        Ok(Self {
            task_id: string(v, "TaskIdParams", "taskId")?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Parameters of `tasks/update`.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateTaskParams {
    /// Which task.
    pub task_id: String,
    /// Answers to its input requests, a JSON object keyed like the requests.
    pub input_responses: Value,
    /// Request `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for UpdateTaskParams {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("taskId", self.task_id.as_str())
            .set("inputResponses", self.input_responses.clone())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "UpdateTaskParams";
        object(v, W)?;
        let responses = crate::codec::field(v, W, "inputResponses")?;
        Ok(Self {
            task_id: string(v, W, "taskId")?,
            input_responses: object(responses, "inputResponses")?.clone(),
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Result of `tasks/get`.
#[derive(Clone, Debug, PartialEq)]
pub struct GetTaskResult {
    /// The task and its payload.
    pub task: DetailedTask,
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for GetTaskResult {
    fn to_value(&self) -> Value {
        self.task
            .encode(Obj::new().set("resultType", ResultType::COMPLETE))
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "GetTaskResult")?;
        Ok(Self {
            task: DetailedTask::decode(v)?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// `notifications/tasks`: the task's new state.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskStatusParams {
    /// The task and its payload.
    pub task: DetailedTask,
    /// `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for TaskStatusParams {
    fn to_value(&self) -> Value {
        self.task
            .encode(Obj::new())
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        object(v, "TaskStatusParams")?;
        Ok(Self {
            task: DetailedTask::decode(v)?,
            meta: opt_value(v, "_meta"),
        })
    }
}

/// Result of `tasks/update` and `tasks/cancel`: an acknowledgement.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskAckResult {
    /// Result `_meta`, raw JSON.
    pub meta: Option<Value>,
}

impl Wire for TaskAckResult {
    fn to_value(&self) -> Value {
        Obj::new()
            .set("resultType", ResultType::COMPLETE)
            .opt_value("_meta", &self.meta)
            .done()
    }

    fn from_value(v: &Value) -> Result<Self> {
        const W: &str = "TaskAckResult";
        let map = v
            .as_object()
            .ok_or_else(|| Error::decode(W, "not an object"))?;
        if let Some((k, _)) = map
            .iter()
            .find(|(k, _)| !matches!(k.as_str(), "resultType" | "_meta"))
        {
            return Err(Error::decode(W, format!("unexpected member {k:?}")));
        }
        if opt_string(v, W, "resultType")?.as_deref() != Some(ResultType::COMPLETE) {
            return Err(Error::decode(W, "resultType is not \"complete\""));
        }
        Ok(Self {
            meta: opt_value(v, "_meta"),
        })
    }
}
