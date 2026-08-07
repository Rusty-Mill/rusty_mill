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
pub mod auth;
mod engine;
mod executor;
#[cfg(feature = "grpc")]
pub mod grpc;
mod push;
mod rest;
mod router;
mod store;

pub use auth::{AuthContext, AuthVerifier, Credentials};
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

    /// Registers an [`AuthVerifier`] to enforce this agent's
    /// `securitySchemes`/`securityRequirements` (spec Section 4.5) - see
    /// the [`auth`] module docs. Without one, declared security
    /// requirements are enforced by rejecting every request (fail
    /// closed); with none declared at all, the agent remains public
    /// either way.
    pub fn with_auth_verifier(mut self, verifier: Arc<dyn AuthVerifier>) -> Self {
        self.engine.set_auth_verifier(verifier);
        self
    }

    /// Lets an `mtls` security scheme (spec Section 4.5.5) be satisfied via
    /// a header/gRPC-metadata entry a TLS-terminating reverse proxy sets in
    /// front of this server to report the result of verifying the client's
    /// certificate - e.g. nginx's `ssl-client-verify`/`ssl-client-s-dn`,
    /// Envoy's `x-forwarded-client-cert`, or an AWS ALB's
    /// `x-amzn-mtls-clientcert`. This crate's own servers never terminate
    /// TLS themselves (see the [`auth`] module docs), so without this
    /// configured, an `mtls`-only requirement is never satisfiable - no
    /// credential is ever extracted for it, so a registered
    /// [`AuthVerifier`] is simply never called. `header_name` is looked up
    /// the same case-insensitive way every other scheme's credential is:
    /// as an HTTP header on JSON-RPC/REST, lowercased as gRPC metadata.
    ///
    /// This is a convenience for the common case of one proxy-set header
    /// meaning "this connection's client certificate was verified"; your
    /// [`AuthVerifier`] still decides what the extracted value has to say
    /// to accept the request (e.g. checking it equals `"SUCCESS"`, or
    /// parsing a subject DN out of it) - this crate has no opinion on your
    /// proxy's specific header format.
    pub fn with_mtls_header(mut self, header_name: impl Into<String>) -> Self {
        self.engine.set_mtls_header(header_name.into());
        self
    }

    /// Enables SSRF protection on push notification webhook URLs (spec
    /// Section 13.2, SHOULD): a `url` that's a literal private/loopback/
    /// link-local IP address, or a hostname that resolves to one, is
    /// rejected both when a config naming it is registered
    /// (`CreateTaskPushNotificationConfig`, or the inline
    /// `taskPushNotificationConfig` on `SendMessage`) and again right
    /// before each delivery (to also catch DNS rebinding - a hostname
    /// that resolved to a public address at registration time but a
    /// private one later).
    ///
    /// Off by default: a local development or test setup delivering to
    /// its own `127.0.0.1` webhook receiver is a completely legitimate
    /// use this crate has no way to distinguish from an attacker
    /// registering a webhook to probe this agent's own internal
    /// network - only you know which situation you're in, so this is
    /// opt-in rather than assumed.
    pub fn with_webhook_ssrf_protection(mut self) -> Self {
        self.engine.set_webhook_ssrf_protection(true);
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
