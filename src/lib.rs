//! `rusty_a2a` - a Rust implementation of the [Agent2Agent (A2A)
//! protocol](https://a2a-protocol.org/latest/), an open standard for
//! interoperable communication between AI agents.
//!
//! This crate provides:
//!
//! - [`types`]: the complete A2A protocol data model (`Task`, `Message`,
//!   `AgentCard`, ...), transliterated from the normative
//!   `specification/a2a.proto`.
//! - [`client`] (feature `client`): an async client for calling A2A agents
//!   over the JSON-RPC 2.0 protocol binding, including SSE streaming.
//! - [`server`] (feature `server`): an [`axum`]-based server harness for
//!   implementing an A2A agent: implement [`server::AgentExecutor`] and get
//!   agent-card discovery, task lifecycle management, and streaming for
//!   free.
//! - [`signing`] (feature `signing`): Agent Card JWS signing and
//!   verification (spec Section 8.4).
//!
//! # Scope
//!
//! The A2A specification defines three interoperable protocol bindings:
//! JSON-RPC 2.0, gRPC, and HTTP+JSON/REST (spec Sections 9-11). This crate
//! implements the **JSON-RPC 2.0 binding**, which the spec calls "a simple,
//! HTTP-based interface" - the most common choice for pure-Rust
//! implementations, since it needs no protobuf toolchain. An agent
//! declaring only a `JSONRPC` interface in its `AgentCard` is fully
//! spec-compliant (Section 5.1 only requires functional equivalence
//! *among* the protocols an agent chooses to support). The canonical
//! `a2a.proto` is vendored under `spec/a2a.proto` for anyone who wants to
//! layer a gRPC or REST binding on top using the same [`types`].
#[cfg(feature = "client")]
pub mod client;
mod codec;
pub mod error;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "signing")]
pub mod signing;
mod timestamp;
pub mod types;

pub use error::A2aError;

/// The `Major.Minor` A2A protocol version implemented by this crate (spec
/// Section 3.6), sent as the `A2A-Version` service parameter.
pub const PROTOCOL_VERSION: &str = "1.0";

/// The well-known path at which an agent's `AgentCard` MUST be served
/// (spec Section 8.2 / 14.3).
pub const AGENT_CARD_WELL_KNOWN_PATH: &str = "/.well-known/agent-card.json";
