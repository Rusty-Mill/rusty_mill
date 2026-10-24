//! # rusty_lsp
//!
//! A small, reusable [Language Server Protocol][lsp] framework for async Rust.
//! The crate owns the protocol plumbing — JSON-RPC framing, message dispatch,
//! the initialize/shutdown lifecycle, request cancellation — and hands you a
//! single trait, [`LanguageServer`], to implement your language's behaviour.
//!
//! It is **not** a server for any particular language; it is the reusable engine
//! you build one on top of. Compared to writing JSON-RPC by hand you get typed
//! handlers and lifecycle correctness; compared to a `tower`-based stack you get
//! a dependency footprint of just `tokio`, `serde`, and `serde_json`.
//!
//! ## Quick start
//!
// The example needs the `runtime` feature; without it, keep it as docs only.
#![cfg_attr(feature = "runtime", doc = "```no_run")]
#![cfg_attr(not(feature = "runtime"), doc = "```ignore")]
//! use rusty_lsp::{Client, LanguageServer, Server};
//! use rusty_lsp::error::Result;
//! use rusty_lsp::lsp::{
//!     InitializeParams, InitializeResult, ServerCapabilities, ServerInfo, TextDocumentSyncKind,
//! };
//!
//! struct Backend {
//!     client: Client,
//! }
//!
//! impl LanguageServer for Backend {
//!     // Trait methods are declared `-> impl Future + Send`, but you may write
//!     // them as ordinary `async fn`.
//!     async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
//!         Ok(InitializeResult {
//!             capabilities: ServerCapabilities {
//!                 text_document_sync: Some(TextDocumentSyncKind::Full.into()),
//!                 hover_provider: Some(true.into()),
//!                 ..Default::default()
//!             },
//!             server_info: Some(ServerInfo { name: "demo".into(), version: None }),
//!         })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     Server::stdio().serve(|client| Backend { client }).await
//! }
//! ```
//!
//! See `examples/text_server.rs` for a fuller backend that tracks open
//! documents and provides hover, completion, and diagnostics.
//!
//! ## Architecture
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`transport`] | Content-Length framing over any async byte stream |
//! | [`jsonrpc`] | JSON-RPC 2.0 request/response/notification model |
//! | [`lsp`] | Typed LSP protocol data structures |
//! | [`text`] | Position ↔ byte conversions (UTF-16, UTF-8, UTF-32) for buffer indexing |
//! | [`service`] | The [`LanguageServer`] trait you implement |
//! | [`client`] | The [`Client`] handle for server→client messages |
//! | [`server`] | The [`Server`] runtime: dispatch, lifecycle, cancellation |
//! | [`documents`] | Optional managed store of open document text |
//! | [`cancel`] | Cooperative cancellation tokens for request handlers |
//! | [`testing`] | In-memory [`testing::TestClient`] harness for backend tests |
//!
//! ## Cargo features
//!
//! - `runtime` (default) — the async server/client modules and their `tokio`
//!   dependency. Disable default features for the protocol data types alone
//!   (`error`, `jsonrpc`, `lsp`, `text`).
//! - `tcp` — [`Server::from_tcp`], serving an accepted `tokio` TCP stream.
//! - `tracing` — wire-level instrumentation via the `tracing` crate.
//!
//! [lsp]: https://microsoft.github.io/language-server-protocol/

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "runtime")]
pub mod cancel;
#[cfg(feature = "runtime")]
pub mod client;
#[cfg(feature = "runtime")]
pub mod documents;
pub mod error;
pub mod jsonrpc;
pub mod lsp;
#[cfg(feature = "runtime")]
pub mod server;
#[cfg(feature = "runtime")]
pub mod service;
#[cfg(feature = "runtime")]
pub mod testing;
pub mod text;
#[cfg(feature = "runtime")]
pub mod transport;

#[cfg(feature = "runtime")]
pub use cancel::CancelToken;
#[cfg(feature = "runtime")]
pub use client::{Client, ProgressGuard};
#[cfg(feature = "runtime")]
pub use documents::{Document, Documents};
pub use error::{Error, Result};
#[cfg(feature = "runtime")]
pub use server::Server;
#[cfg(feature = "runtime")]
pub use service::LanguageServer;
