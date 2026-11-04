//! The model port: one stateless call, a fresh context in, a checked reply
//! out: board posts, or raw text for a caller-side parser (#708).

use orch_dispatch::ClassifiedError;

use crate::post::{Allowed, Post};

/// One question for one lens about one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Stable across calls (intent, board rules, lens prompt): cacheable.
    pub prefix: String,
    /// This claim, its deps, settled lines and the lens's track record.
    pub body: String,
    /// What the reply must be; every adapter checks it with
    /// [`crate::check`] before returning.
    pub shape: Shape,
    pub max_output_tokens: u32,
}

/// Token counts for one call, as the provider reported them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

/// What a question's reply must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// Board posts under the reply protocol, within what the job may post.
    Posts(Allowed),
    /// Non-empty text, for a reply that posts nothing (the #708 alien
    /// lens's enumerate and hunt steps). Its caller parses it.
    Raw,
}

/// A checked reply, of the question's [`Shape`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Posts(Vec<Post>),
    Raw(String),
}

/// A validated reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub reply: Reply,
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
