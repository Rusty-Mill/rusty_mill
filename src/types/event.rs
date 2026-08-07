//! Stream events emitted during a run.

use serde::{Deserialize, Serialize};

use crate::types::{
    error::Error,
    message::{Message, MessagePart},
    run::Run,
};

/// An event emitted by a run, as delivered over `text/event-stream` or listed
/// by `GET /runs/{run_id}/events`.
///
/// The wire form is a JSON object discriminated by its `type` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// An agent began composing a message. `parts` may be empty at this point.
    #[serde(rename = "message.created")]
    MessageCreated {
        /// The message being started.
        message: Message,
    },
    /// An incremental part of the message currently being composed.
    #[serde(rename = "message.part")]
    MessagePart {
        /// The newly emitted part.
        part: MessagePart,
    },
    /// A message finished; `message` holds all of its parts.
    #[serde(rename = "message.completed")]
    MessageCompleted {
        /// The completed message.
        message: Message,
    },
    /// An agent-defined event carrying arbitrary JSON.
    #[serde(rename = "generic")]
    Generic {
        /// The agent-defined payload.
        generic: serde_json::Value,
    },
    /// The run was created.
    #[serde(rename = "run.created")]
    RunCreated {
        /// Snapshot of the run.
        run: Box<Run>,
    },
    /// The run started processing.
    #[serde(rename = "run.in-progress")]
    RunInProgress {
        /// Snapshot of the run.
        run: Box<Run>,
    },
    /// The run paused, awaiting client input.
    #[serde(rename = "run.awaiting")]
    RunAwaiting {
        /// Snapshot of the run, including its `await_request`.
        run: Box<Run>,
    },
    /// The run completed successfully.
    #[serde(rename = "run.completed")]
    RunCompleted {
        /// Final snapshot of the run.
        run: Box<Run>,
    },
    /// The run failed.
    #[serde(rename = "run.failed")]
    RunFailed {
        /// Final snapshot of the run, including its `error`.
        run: Box<Run>,
    },
    /// The run was cancelled.
    #[serde(rename = "run.cancelled")]
    RunCancelled {
        /// Final snapshot of the run.
        run: Box<Run>,
    },
    /// A transport- or server-level error occurred on the stream.
    #[serde(rename = "error")]
    Error {
        /// The error.
        error: Error,
    },
}

impl Event {
    /// The value of the event's `type` discriminator, also used as the SSE
    /// `event:` name.
    pub const fn event_type(&self) -> &'static str {
        match self {
            Event::MessageCreated { .. } => "message.created",
            Event::MessagePart { .. } => "message.part",
            Event::MessageCompleted { .. } => "message.completed",
            Event::Generic { .. } => "generic",
            Event::RunCreated { .. } => "run.created",
            Event::RunInProgress { .. } => "run.in-progress",
            Event::RunAwaiting { .. } => "run.awaiting",
            Event::RunCompleted { .. } => "run.completed",
            Event::RunFailed { .. } => "run.failed",
            Event::RunCancelled { .. } => "run.cancelled",
            Event::Error { .. } => "error",
        }
    }

    /// The run snapshot carried by `run.*` events.
    pub fn run(&self) -> Option<&Run> {
        match self {
            Event::RunCreated { run }
            | Event::RunInProgress { run }
            | Event::RunAwaiting { run }
            | Event::RunCompleted { run }
            | Event::RunFailed { run }
            | Event::RunCancelled { run } => Some(run),
            _ => None,
        }
    }

    /// Whether this event ends the stream: a terminal `run.*` event, a
    /// `run.awaiting` pause, or a stream-level `error`.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Event::RunCompleted { .. }
                | Event::RunFailed { .. }
                | Event::RunCancelled { .. }
                | Event::RunAwaiting { .. }
                | Event::Error { .. }
        )
    }

    /// Convenience constructor for [`Event::Generic`].
    pub fn generic(payload: serde_json::Value) -> Self {
        Event::Generic { generic: payload }
    }

    /// Roughly how many bytes this event occupies, for stores that bound a
    /// run's log by size.
    ///
    /// An estimate, not an accounting. Serialising each event to measure it
    /// exactly would put a full JSON encode on every append — the hot path for
    /// a streaming agent — to inform a limit that is a rough ceiling anyway.
    /// What this has to get right is the ratio between a one-word text part and
    /// a base64 artifact, which is the difference the bound exists to notice,
    /// and summing the payloads does that.
    ///
    /// Counted as the payload plus a fixed allowance for the envelope, so a
    /// flood of tiny events is bounded rather than treated as free.
    pub fn approximate_size(&self) -> usize {
        /// Enough to cover the `type` discriminator, the JSON punctuation and
        /// the `Vec`'s own slot, so empty events are not free.
        const ENVELOPE: usize = 128;

        let payload = match self {
            Event::MessagePart { part } => part.approximate_size(),
            Event::MessageCreated { message } | Event::MessageCompleted { message } => {
                message.parts.iter().map(MessagePart::approximate_size).sum()
            }
            // A run snapshot's own size is dominated by its output, which is a
            // message list like any other.
            Event::RunCreated { run }
            | Event::RunInProgress { run }
            | Event::RunAwaiting { run }
            | Event::RunCompleted { run }
            | Event::RunFailed { run }
            | Event::RunCancelled { run } => run
                .output
                .iter()
                .flat_map(|message| message.parts.iter())
                .map(MessagePart::approximate_size)
                .sum(),
            // Serialised, because an agent-defined payload has no structure to
            // walk and is the one case that can be arbitrarily large without
            // going through a message part.
            Event::Generic { generic } => {
                serde_json::to_string(generic).map(|json| json.len()).unwrap_or(0)
            }
            Event::Error { error } => error.message.len(),
        };
        ENVELOPE + payload
    }

    /// Convenience constructor for a `run.*` event matching the run's status.
    ///
    /// Returns `None` for [`crate::types::RunStatus::Cancelling`], which has no
    /// corresponding event in the specification.
    pub fn for_run(run: Run) -> Option<Self> {
        use crate::types::run::RunStatus;
        let run = Box::new(run);
        match run.status {
            RunStatus::Created => Some(Event::RunCreated { run }),
            RunStatus::InProgress => Some(Event::RunInProgress { run }),
            RunStatus::Awaiting => Some(Event::RunAwaiting { run }),
            RunStatus::Completed => Some(Event::RunCompleted { run }),
            RunStatus::Failed => Some(Event::RunFailed { run }),
            RunStatus::Cancelled => Some(Event::RunCancelled { run }),
            RunStatus::Cancelling => None,
        }
    }
}
