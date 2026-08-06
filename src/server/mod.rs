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
#[cfg(feature = "grpc")]
pub mod grpc;
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
///
/// This is a *builder*: construct it, call the `with_*` setters, then
/// either use [`AgentServer::into_router`]/[`AgentServer::serve`] directly
/// for the common single-binding case, or call [`AgentServer::build`] to
/// get an [`AgentServices`] handle that can serve this same agent state
/// (task store included) over multiple protocol bindings at once - e.g.
/// JSON-RPC and gRPC on two different ports.
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

    /// Finalizes this builder into a shareable [`AgentServices`] handle:
    /// `.router()` and (with the `grpc` feature) `.grpc_service()` can
    /// each be called on it as many times as needed, all serving the same
    /// underlying agent state.
    pub fn build(self) -> AgentServices {
        AgentServices {
            engine: Arc::new(self.engine),
        }
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
    /// [`AgentServer::serve`] for a zero-setup default. Shorthand for
    /// `self.build().router()`.
    pub fn into_router(self) -> Router {
        self.build().router()
    }

    /// Binds `addr` and serves this agent until the process is
    /// interrupted.
    pub async fn serve(self, addr: impl Into<SocketAddr>) -> std::io::Result<()> {
        self.build().serve_http(addr).await
    }
}

/// A finalized, shareable handle on one agent's state (produced by
/// [`AgentServer::build`]), used to serve it over one or more protocol
/// bindings at once - each sharing the same [`TaskStore`] and
/// [`AgentExecutor`], so e.g. a task created via gRPC is visible to a
/// `GetTask` call made over JSON-RPC.
#[derive(Clone)]
pub struct AgentServices {
    engine: Arc<Engine>,
}

impl AgentServices {
    /// Builds the `axum::Router` for the JSON-RPC and HTTP+JSON/REST
    /// bindings + Agent Card discovery (spec Sections 8.2, 9, 11). Can be
    /// called more than once; each call builds an independent `Router`
    /// sharing this handle's state.
    pub fn router(&self) -> Router {
        router::build_router(self.engine.clone()).merge(rest::build_rest_router(self.engine.clone()))
    }

    /// Binds `addr` and serves the JSON-RPC + REST bindings until the
    /// process is interrupted.
    pub async fn serve_http(&self, addr: impl Into<SocketAddr>) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr.into()).await?;
        axum::serve(listener, self.router()).await
    }

    /// Builds the gRPC service for the `A2AService` binding (spec Section
    /// 10), ready to hand to a [`tonic::transport::Server`] (e.g. via
    /// `.add_service(...)`), or serve directly with
    /// [`AgentServices::serve_grpc`].
    #[cfg(feature = "grpc")]
    pub fn grpc_service(&self) -> grpc::pb::a2a_service_server::A2aServiceServer<grpc::GrpcService> {
        grpc::pb::a2a_service_server::A2aServiceServer::new(grpc::GrpcService::new(self.engine.clone()))
    }

    /// Binds `addr` and serves the gRPC binding until the process is
    /// interrupted.
    #[cfg(feature = "grpc")]
    pub async fn serve_grpc(&self, addr: impl Into<SocketAddr>) -> Result<(), tonic::transport::Error> {
        tonic::transport::Server::builder()
            .add_service(self.grpc_service())
            .serve(addr.into())
            .await
    }
}
