//! # rusty-acp
//!
//! A Rust implementation of the [Agent Communication Protocol][acp] (ACP)
//! v0.2.0 — an open, REST-based protocol for making AI agents interoperable
//! across frameworks, languages and organisations.
//!
//! The crate is split into three layers, each usable on its own:
//!
//! | Module | Feature | What it gives you |
//! |---|---|---|
//! | [`types`] | always on | The complete wire format: manifests, messages, runs, events, sessions, errors. |
//! | [`client`] | `client` | An HTTP client for calling any ACP server, including SSE streaming. |
//! | [`server`] | `server` | An [`axum`] router that hosts your own agents behind the standard endpoints. |
//! | [`server::store`] | `redis-store` | A Redis-backed [`Store`](server::store::Store), for several replicas behind a load balancer. |
//! | open discovery | `well-known` | Serves agent metadata as YAML at `/.well-known/agent.yml`. |
//!
//! ## Serving an agent
//!
//! Implement [`server::Agent`], register it, and serve the router:
//!
//! ```no_run
//! use rusty_acp::server::{Agent, AcpServer, RunContext};
//! use rusty_acp::types::{AgentManifest, AgentName, Error};
//!
//! struct Echo;
//!
//! #[async_trait::async_trait]
//! impl Agent for Echo {
//!     fn manifest(&self) -> AgentManifest {
//!         AgentManifest::new(AgentName::new("echo").unwrap(), "Echoes the input back")
//!     }
//!
//!     async fn run(&self, ctx: RunContext) -> Result<(), Error> {
//!         let text = ctx.input_text();
//!         ctx.reply_text(text).await?;
//!         Ok(())
//!     }
//! }
//!
//! # async fn serve() -> Result<(), Box<dyn std::error::Error>> {
//! let router = AcpServer::builder().agent(Echo).build()?.into_router();
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
//! axum::serve(listener, router).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Calling an agent
//!
//! ```no_run
//! use rusty_acp::client::AcpClient;
//! use rusty_acp::types::Message;
//!
//! # async fn call() -> Result<(), rusty_acp::AcpError> {
//! let client = AcpClient::new("http://localhost:8000")?;
//! let run = client.run_sync("echo", [Message::user("hello")]).await?;
//! println!("{}", run.output_text());
//! # Ok(())
//! # }
//! ```
//!
//! ## Running several replicas
//!
//! Runs live in process memory by default. Give every replica the same shared
//! [`Store`](server::store::Store) and they share one view of every run — any
//! replica can serve any request, and no session affinity is needed:
//!
//! ```no_run
//! # #[cfg(feature = "redis-store")]
//! # async fn serve(agent: impl rusty_acp::server::Agent) -> Result<(), Box<dyn std::error::Error>> {
//! use rusty_acp::server::{store::RedisStore, AcpServer};
//!
//! let store = RedisStore::connect("redis://127.0.0.1/").await?;
//! let router = AcpServer::builder()
//!     .agent(agent)
//!     .store(std::sync::Arc::new(store))
//!     .build()?
//!     .into_router();
//! # Ok(())
//! # }
//! ```
//!
//! See [`server::store`] for what a backend must guarantee.
//!
//! [acp]: https://agentcommunicationprotocol.dev

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "trace")]
pub mod trace;

// Compiled either way so the plumbing that carries a context — the field on the
// launch spec, the parameter on the run — needs no `cfg` of its own. With the
// feature off nothing ever constructs one, so every branch that reads it is
// dead and the compiler removes it. The alternative was a `cfg` cascade through
// four signatures in two modules to save a type that costs nothing.
#[cfg(not(feature = "trace"))]
#[allow(dead_code)]
mod trace;

pub mod types;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

/// The `reqwest` this crate was built against.
///
/// [`AcpClient::with_http_client`](client::AcpClient::with_http_client) and
/// [`AcpClientBuilder::http_client`](client::AcpClientBuilder::http_client)
/// take a [`reqwest::Client`], which is the documented way to carry
/// credentials. Building one from your own `reqwest` dependency only works if
/// it resolves to this exact crate; when it does not, the error is a mismatch
/// between two types with the same name, which is among the less pleasant
/// things the compiler has to say.
///
/// Going through this re-export makes that impossible:
///
/// ```
/// use rusty_acp::client::AcpClient;
/// use rusty_acp::reqwest;
///
/// # fn build() -> Result<(), Box<dyn std::error::Error>> {
/// let mut headers = reqwest::header::HeaderMap::new();
/// headers.insert("authorization", "Bearer hunter2".parse()?);
/// let http = reqwest::Client::builder().default_headers(headers).build()?;
///
/// let client = AcpClient::with_http_client("http://localhost:8000", http)?;
/// # let _ = client;
/// # Ok(())
/// # }
/// ```
///
/// This does pin you to this crate's `reqwest` major version, which is the
/// point rather than a side effect: the constraint already exists, and today it
/// is discovered at the type error rather than stated.
#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub use reqwest;

#[cfg(feature = "server")]
#[cfg_attr(docsrs, doc(cfg(feature = "server")))]
pub mod server;

pub use types::Error as ProtocolError;

/// The ACP specification version this crate implements.
pub const ACP_VERSION: &str = "0.2.0";

/// Header naming the first index a run's event list actually starts at.
///
/// An extension this crate defines, like the resumable event stream and
/// `/ready`. A server that bounds how much of one run's log it keeps drops the
/// oldest events, and the list response is a *spec* type — a field added to it
/// would put something on the wire ACP does not define, where a header says the
/// same thing to a client that looks and nothing at all to one that does not.
///
/// Sent on every list response rather than only when events have been dropped.
/// `0` means the log is whole; **no header at all** means the server predates
/// this, which is a different answer and one that absence-means-complete could
/// not give.
///
/// At the crate root because both halves need it: the server sends it and
/// [`AcpClient::list_run_events`](client::AcpClient::list_run_events) reads it,
/// and either feature can be enabled without the other.
pub const EVENTS_FROM_HEADER: &str = "acp-events-from";

/// Errors surfaced by the client and server layers.
///
/// [`AcpError::Protocol`] wraps an [`Error`](types::Error) object returned by a
/// peer; the other variants describe failures that happened before or below the
/// protocol.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AcpError {
    /// The peer returned a well-formed ACP error object.
    #[error("acp error: {0}")]
    Protocol(#[from] types::Error),

    /// The peer returned an HTTP error that was not a valid ACP error object.
    #[error("http {status}: {body}")]
    Http {
        /// The HTTP status code.
        status: u16,
        /// The raw response body.
        body: String,
    },

    /// The request could not be sent, or the connection failed mid-flight.
    #[error("transport error: {0}")]
    Transport(String),

    /// A payload could not be encoded or decoded.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// The event stream was malformed or ended unexpectedly.
    #[error("stream error: {0}")]
    Stream(String),

    /// The supplied base URL or endpoint path was not a valid URL.
    #[error("invalid url: {0}")]
    InvalidUrl(String),

    /// A client-side wait gave up before the run settled.
    ///
    /// The run is not necessarily broken — it may simply be slower than the
    /// deadline allowed. `status` is what it had reached when the wait ended.
    #[error("timed out waiting for run {run_id}; last seen `{status}`")]
    Timeout {
        /// The run that was being waited on.
        run_id: String,
        /// The run's status when the wait gave up.
        status: String,
    },
}

impl AcpError {
    /// The [`ErrorCode`](types::ErrorCode) when this is a protocol error.
    pub fn code(&self) -> Option<types::ErrorCode> {
        match self {
            AcpError::Protocol(error) => Some(error.code),
            _ => None,
        }
    }

    /// Whether this failure is worth trying again.
    ///
    /// True for a transport failure, and for the statuses that mean *not now*:
    /// 429, 502, 503 and 504. **Not** true of 500 — that is what a server
    /// returns when the agent itself failed, and a second attempt reproduces it
    /// rather than resolving it.
    ///
    /// The client applies this itself under its
    /// [`RetryPolicy`](client::RetryPolicy); it is public so that callers
    /// wrapping their own loop around a run can make the same distinction.
    pub fn is_transient(&self) -> bool {
        match self {
            AcpError::Transport(_) => true,
            AcpError::Http { status, .. } => matches!(status, 429 | 502 | 503 | 504),
            _ => false,
        }
    }

    /// Whether this is a [`ErrorCode::NotFound`](types::ErrorCode::NotFound)
    /// protocol error, or an HTTP 404.
    pub fn is_not_found(&self) -> bool {
        match self {
            AcpError::Protocol(error) => error.code == types::ErrorCode::NotFound,
            AcpError::Http { status, .. } => *status == 404,
            _ => false,
        }
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(value: serde_json::Error) -> Self {
        AcpError::Serialization(value.to_string())
    }
}

/// Result alias used throughout the crate.
pub type Result<T, E = AcpError> = std::result::Result<T, E>;
