//! The model port: one stateless call, a fresh context in, validated posts out.

use orch_dispatch::ClassifiedError;

use crate::post::{Allowed, Post};

/// One question for one lens about one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Stable across calls (intent, board rules, lens prompt): cacheable.
    pub prefix: String,
    /// This claim, its deps, settled lines and the lens's track record.
    pub body: String,
    /// What the reply may contain; every adapter checks it with
    /// [`crate::parse`] before returning.
    pub allowed: Allowed,
    pub max_output_tokens: u32,
}

/// Token counts for one call, as the provider reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

/// A validated reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub posts: Vec<Post>,
    pub usage: Usage,
}

/// A provider. Adapters live in their own crates; the runner sees only this.
///
/// A malformed or non-conforming reply is `Permanent`, so nothing
/// unvalidated reaches the board. `Unavailable` moves the job to the next
/// provider in its tier.
pub trait Model {
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError>;
}
