//! Run lifecycle types: identifiers, status, mode, requests and the [`Run`] resource.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    agent::AgentName,
    error::Error,
    message::Message,
    session::{Session, SessionId},
};

macro_rules! uuid_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a fresh random identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// The underlying UUID.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self).map_err(|err| {
                    Error::invalid_input(format!(
                        concat!("invalid ", stringify!($name), " {:?}: {}"),
                        s, err
                    ))
                })
            }
        }
    };
}

uuid_newtype!(
    /// Identifier of a run.
    RunId
);

/// Lifecycle status of a [`Run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    /// Accepted, not yet started.
    Created,
    /// The agent is actively processing.
    InProgress,
    /// Paused, waiting for the client to resume it.
    Awaiting,
    /// A cancellation request is being processed.
    Cancelling,
    /// Terminated by cancellation.
    Cancelled,
    /// Finished successfully.
    Completed,
    /// Halted by an error.
    Failed,
}

impl RunStatus {
    /// Whether the run has reached a state it can never leave.
    pub const fn is_terminal(self) -> bool {
        matches!(self, RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled)
    }

    /// Whether the run is paused awaiting client input.
    pub const fn is_awaiting(self) -> bool {
        matches!(self, RunStatus::Awaiting)
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            RunStatus::Created => "created",
            RunStatus::InProgress => "in-progress",
            RunStatus::Awaiting => "awaiting",
            RunStatus::Cancelling => "cancelling",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
        };
        f.write_str(text)
    }
}

/// How the client wants the run delivered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// Block until the run reaches a terminal or awaiting state.
    #[default]
    Sync,
    /// Return immediately; poll `GET /runs/{run_id}` for progress.
    Async,
    /// Stream events over `text/event-stream` as they are emitted.
    Stream,
}

/// Payload describing what the agent needs from the client to continue.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AwaitRequest(pub serde_json::Value);

/// Payload supplied by the client to resume an awaiting run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AwaitResume(pub serde_json::Value);

macro_rules! await_payload_impls {
    ($name:ident) => {
        impl $name {
            /// Wrap an arbitrary JSON value.
            pub fn new(value: serde_json::Value) -> Self {
                Self(value)
            }

            /// Serialize a typed payload into this wrapper.
            pub fn from_value<T: Serialize>(value: &T) -> Result<Self, Error> {
                serde_json::to_value(value).map(Self).map_err(|err| {
                    Error::invalid_input(format!(
                        concat!("failed to serialize ", stringify!($name), ": {}"),
                        err
                    ))
                })
            }

            /// Deserialize the payload into a typed value.
            pub fn deserialize<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
                serde_json::from_value(self.0.clone()).map_err(|err| {
                    Error::invalid_input(format!(
                        concat!("failed to deserialize ", stringify!($name), ": {}"),
                        err
                    ))
                })
            }

            /// The wrapped JSON value.
            pub fn as_value(&self) -> &serde_json::Value {
                &self.0
            }

            /// Consume the wrapper, yielding the JSON value.
            pub fn into_value(self) -> serde_json::Value {
                self.0
            }
        }

        impl From<serde_json::Value> for $name {
            fn from(value: serde_json::Value) -> Self {
                Self(value)
            }
        }
    };
}

await_payload_impls!(AwaitRequest);
await_payload_impls!(AwaitResume);

/// Request body of `POST /runs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCreateRequest {
    /// The agent to run.
    pub agent_name: AgentName,
    /// Continue an existing session by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Supply a full session, e.g. one hosted by another ACP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
    /// Input messages. Must contain at least one entry.
    pub input: Vec<Message>,
    /// Delivery mode. Defaults to [`RunMode::Sync`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RunMode>,
}

impl RunCreateRequest {
    /// A request for `agent_name` with the given input messages.
    pub fn new(agent_name: AgentName, input: impl IntoIterator<Item = Message>) -> Self {
        Self {
            agent_name,
            session_id: None,
            session: None,
            input: input.into_iter().collect(),
            mode: None,
        }
    }

    /// Continue the given session.
    pub fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Supply a full session object.
    pub fn with_session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    /// Set the delivery mode.
    pub fn with_mode(mut self, mode: RunMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// The requested mode, applying the [`RunMode::Sync`] default.
    pub fn mode(&self) -> RunMode {
        self.mode.unwrap_or_default()
    }

    /// Check the `minItems: 1` constraint on `input` and validate each message.
    pub fn validate(&self) -> Result<(), Error> {
        if self.input.is_empty() {
            return Err(Error::invalid_input("`input` must contain at least one message"));
        }
        for message in &self.input {
            message.validate()?;
        }
        if let (Some(session_id), Some(session)) = (self.session_id, self.session.as_ref()) {
            if session_id != session.id {
                return Err(Error::invalid_input(
                    "`session_id` and `session.id` refer to different sessions",
                ));
            }
        }
        Ok(())
    }
}

/// Request body of `POST /runs/{run_id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResumeRequest {
    /// The run to resume.
    pub run_id: RunId,
    /// The payload the awaiting agent asked for.
    pub await_resume: AwaitResume,
    /// Delivery mode for the resumed run.
    pub mode: RunMode,
}

impl RunResumeRequest {
    /// Build a resume request.
    pub fn new(run_id: RunId, await_resume: AwaitResume, mode: RunMode) -> Self {
        Self { run_id, await_resume, mode }
    }
}

/// A single agent execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// The agent being run.
    pub agent_name: AgentName,
    /// Session the run belongs to, when it is part of one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// Unique identifier of this run.
    pub run_id: RunId,
    /// Current lifecycle status.
    pub status: RunStatus,
    /// What the agent is waiting for, when [`RunStatus::Awaiting`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub await_request: Option<AwaitRequest>,
    /// Messages produced so far.
    pub output: Vec<Message>,
    /// Failure detail, when [`RunStatus::Failed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    /// When the run was created.
    pub created_at: DateTime<Utc>,
    /// When the run reached a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

impl Run {
    /// A freshly created run in [`RunStatus::Created`].
    pub fn new(agent_name: AgentName, session_id: Option<SessionId>) -> Self {
        Self {
            agent_name,
            session_id,
            run_id: RunId::new(),
            status: RunStatus::Created,
            await_request: None,
            output: Vec::new(),
            error: None,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    /// Concatenated plain text of every output message.
    pub fn output_text(&self) -> String {
        self.output.iter().map(Message::text).collect::<Vec<_>>().join("")
    }

    /// The run as a `Result`, mapping [`RunStatus::Failed`] onto its error.
    pub fn into_result(self) -> Result<Self, Error> {
        match (self.status, self.error.clone()) {
            (RunStatus::Failed, Some(error)) => Err(error),
            (RunStatus::Failed, None) => Err(Error::server_error(format!(
                "run {} failed without an error payload",
                self.run_id
            ))),
            _ => Ok(self),
        }
    }
}

/// Response body of `GET /runs/{run_id}/events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEventsListResponse {
    /// Events emitted by the run, in order.
    pub events: Vec<super::event::Event>,
}
