//! `rusty_a2a` - a Rust implementation of the [Agent2Agent (A2A)
//! protocol](https://a2a-protocol.org/latest/), an open standard for
//! interoperable communication between AI agents.
//!
//! This crate provides:
//!
//! - [`types`]: the complete A2A protocol data model (`Task`, `Message`,
//!   `AgentCard`, ...), transliterated from the normative, vendored
//!   `spec/a2a.proto`.
//! - [`client`] (feature `client`): async clients for calling A2A agents.
//!   [`client::A2aClient`] speaks JSON-RPC 2.0 (including SSE streaming);
//!   [`client::RestClient`] speaks HTTP+JSON/REST; [`client::GrpcClient`]
//!   (feature `client` + `grpc`) speaks gRPC - all three share the same
//!   method names and signatures.
//! - [`server`] (feature `server`): an [`axum`]-based server harness for
//!   implementing an A2A agent: implement [`server::AgentExecutor`] and get
//!   agent-card discovery, task lifecycle management, and streaming for
//!   free, over the JSON-RPC and HTTP+JSON/REST protocol bindings at once.
//!   Add the `grpc` feature and use [`server::AgentServices`] to serve the
//!   same agent state over gRPC too.
//! - [`signing`] (feature `signing`): Agent Card JWS signing and
//!   verification (spec Section 8.4).
//!
//! # Scope
//!
//! The A2A specification defines three interoperable protocol bindings:
//! JSON-RPC 2.0, gRPC, and HTTP+JSON/REST (spec Sections 9-11). This
//! crate's server implements all three - JSON-RPC and HTTP+JSON/REST from
//! the same `axum` router/port (`server::AgentServer::into_router`), and
//! gRPC via `server::AgentServices::grpc_service`/`serve_grpc` (feature
//! `grpc`, compiled from the vendored `spec/a2a.proto` by `build.rs`;
//! requires a `protoc` binary on `PATH`). All bindings share one task
//! store and executor. The client implements all three too, as three
//! separate types (see [`client`] above) rather than one that speaks
//! whichever binding a server happens to prefer. Per spec Section 5.1, an
//! agent only needs to support the protocols it declares in its
//! `AgentCard`, so any subset is spec-compliant.
#[cfg(feature = "client")]
pub mod client;
mod codec;
pub mod error;
#[cfg(feature = "grpc")]
mod grpc_convert;
#[cfg(feature = "grpc")]
pub mod pb;
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
