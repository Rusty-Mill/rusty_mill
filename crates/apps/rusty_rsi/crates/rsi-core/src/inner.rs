//! The inner loop's ports: the model it calls and the harness that runs
//! it (ADR-0005 §2, §3).
//!
//! The harness never holds a [`ChatModel`] or a [`PublicTask`] itself: it
//! is an untrusted process that reaches both only through the runtime's
//! broker, which meters every call against a [`Budget`].

use crate::budget::{Budget, CostUsage};
use crate::lineage::ModelId;
use crate::rng::Seed;
use crate::task::{PublicTask, Solution};

/// Who wrote a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Instructions.
    System,
    /// The prompt.
    User,
    /// An earlier model reply.
    Assistant,
}

impl Role {
    /// The OpenAI-compatible role name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    /// Parses a role name produced by [`Role::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// What it says.
    pub content: String,
}

/// A model reply and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The reply text.
    pub text: String,
    /// Prompt tokens charged.
    pub prompt_tokens: u64,
    /// Completion tokens charged.
    pub completion_tokens: u64,
}

/// A chat-completion model endpoint.
pub trait ChatModel {
    /// The adapter's error type.
    type Error;

    /// The model's identifier, as recorded in lineage.
    fn id(&self) -> &ModelId;

    /// Completes `messages`, producing at most `max_tokens` tokens.
    ///
    /// # Errors
    /// When the endpoint could not be reached or answered nonsense.
    fn complete(&self, messages: &[Message], max_tokens: u64) -> Result<Completion, Self::Error>;
}

/// What one inner run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct InnerOutcome {
    /// The harness's last valid submission (`x̂_t`), if it made one.
    pub submission: Option<Solution>,
    /// Tokens and wall-clock time the run consumed.
    pub usage: CostUsage,
    /// The encoded broker transcript, for lineage and trajectory replay.
    pub transcript: Vec<u8>,
    /// The start of the agent's own diagnostic output (stderr).
    pub log: String,
}

/// Runs an inner agent on one task under a budget.
pub trait Harness<T: PublicTask> {
    /// The adapter's error type (infrastructure failures only).
    type Error;

    /// Runs the agent on `task` within `budget`, seeded by `seed`.
    ///
    /// # Errors
    /// Only when the run could not happen. An agent that crashes, loops or
    /// never submits is an [`InnerOutcome`] without a submission.
    fn run(&self, task: &T, budget: &Budget, seed: Seed) -> Result<InnerOutcome, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_names_round_trip() {
        for role in [Role::System, Role::User, Role::Assistant] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("tool"), None);
    }
}
