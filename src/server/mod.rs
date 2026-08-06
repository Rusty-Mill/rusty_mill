//! An [`axum`]-based harness for implementing an A2A agent server over the
//! JSON-RPC 2.0 protocol binding (spec Section 9).
//!
//! Implement [`AgentExecutor`] to define what your agent does, then hand
//! it (plus an [`crate::types::AgentCard`] describing it) to
//! [`AgentServer`] to get task lifecycle management, Server-Sent Events
//! streaming, and Agent Card discovery for free.
//!
//! ```no_run
//! use std::sync::Arc;
//! use async_trait::async_trait;
//! use rusty_a2a::error::Result;
//! use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
//! use rusty_a2a::types::{AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Message, TaskState};
//!
//! struct EchoAgent;
//!
//! #[async_trait]
//! impl AgentExecutor for EchoAgent {
//!     async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
//!         events.status(TaskState::Working);
//!         events.status_with_message(
//!             TaskState::Completed,
//!             Some(Message::agent_text(format!("you said: {}", ctx.message.text()))),
//!         );
//!         Ok(())
//!     }
//! }
//!
//! # async fn run() -> std::io::Result<()> {
//! let card = AgentCard::new(
//!     "Echo Agent",
//!     "Echoes back whatever you send it.",
//!     "0.1.0",
//!     AgentInterface::json_rpc("http://localhost:8080"),
//! )
//! .with_streaming(true);
//!
//! let server = AgentServer::new(card, Arc::new(EchoAgent));
//! server.serve(([127, 0, 0, 1], 8080)).await
//! # }
//! ```
mod engine;
mod executor;
mod rest;
mod router;
mod store;

pub use executor::{AgentExecutor, EventSink, RequestContext};
pub use store::{InMemoryTaskStore, TaskStore};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;

use crate::types::AgentCard;
use engine::Engine;

/// Wires an [`AgentExecutor`] and a [`TaskStore`] up to the A2A JSON-RPC
/// protocol binding and produces a ready-to-serve [`axum::Router`].
pub struct AgentServer {
    engine: Engine,
}

impl AgentServer {
    /// Creates a server backed by an [`InMemoryTaskStore`].
    pub fn new(card: AgentCard, executor: Arc<dyn AgentExecutor>) -> Self {
        Self::with_store(card, executor, Arc::new(InMemoryTaskStore::new()))
    }

    /// Creates a server backed by a custom [`TaskStore`] (e.g. one backed
    /// by a database, for durability across restarts).
    pub fn with_store(card: AgentCard, executor: Arc<dyn AgentExecutor>, store: Arc<dyn TaskStore>) -> Self {
        AgentServer {
            engine: Engine::new(card, executor, store),
        }
    }

    /// Sets the card returned by `GetExtendedAgentCard`. Requires
    /// `capabilities.extendedAgentCard` to also be set on the base card,
    /// or the operation will return `ExtendedAgentCardNotConfiguredError`
    /// regardless.
    pub fn with_extended_card(mut self, card: AgentCard) -> Self {
        self.engine.set_extended_card(card);
        self
    }

    pub fn agent_card(&self) -> &AgentCard {
        self.engine.card()
    }

    /// Builds the `axum::Router` serving this agent: `POST /` for
    /// JSON-RPC calls, the HTTP+JSON/REST routes from spec Section 11.3
    /// (`POST /message:send`, `GET /tasks/{id}`, ...), and
    /// `GET /.well-known/agent-card.json` for discovery - all on one
    /// port. Declare both bindings in your `AgentCard` if you want
    /// clients to be able to discover and choose between them (see
    /// [`crate::types::AgentInterface::json_rpc`] and
    /// [`crate::types::AgentInterface::http_json`]). Mount it yourself
    /// (e.g. behind TLS termination, or nested under a path) or call
    /// [`AgentServer::serve`] for a zero-setup default.
    pub fn into_router(self) -> Router {
        let engine = Arc::new(self.engine);
        router::build_router(engine.clone()).merge(rest::build_rest_router(engine))
    }

    /// Binds `addr` and serves this agent until the process is
    /// interrupted.
    pub async fn serve(self, addr: impl Into<SocketAddr>) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr.into()).await?;
        axum::serve(listener, self.into_router()).await
    }
}
