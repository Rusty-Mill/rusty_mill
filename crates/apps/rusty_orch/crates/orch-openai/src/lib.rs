//! One OpenAI-compatible chat completion over plain HTTP/1.1, for a model
//! server on the local network (llama-swap in v0; Gemini later, over
//! HTTPS, once the workspace has its canonical blocking client).
//!
//! The lens runner (#594) asks it one stateless question at a time: a
//! cacheable system part and a per-call part in, the reply text and token
//! usage out. Interpreting the reply is the caller's job.
//!
//! - [`Config`]: validated settings; `https` is refused.
//! - [`OpenAiHttp`]: resolves the host on every call and refuses unless
//!   every address is loopback or private ([`is_private`]), then connects to
//!   an address it checked, never the name again, so DNS cannot change
//!   between check and connect. One deadline bounds the whole call.
//! - The API key, when one is configured, is read from its environment
//!   variable at call time and never appears in an error.
//! - [`OpenAiHttp`] implements the lens runner's [`orch_lens::Model`]:
//!   [`answer`] checks the reply with [`orch_lens::parse`].

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod address;
mod client;
mod config;
mod model;
mod wire;

pub use address::{all_private, is_private};
pub use client::{EnvLookup, OpenAiHttp, Resolve, SystemResolver, MAX_PENDING_LOOKUPS};
pub use config::{Config, ConfigError};
pub use model::answer;
pub use wire::{parse_response, request_body, Completion, Prompt, Usage};
