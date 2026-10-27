//! `rusty_oauth_server`: a small OAuth 2.1 authorization server for MCP
//! servers with a handful of known users, sans IO.
//!
//! It implements the flow claude.ai's custom connectors use ("Sign in
//! now"): RFC 8414 and RFC 9728 metadata, RFC 7591 dynamic registration,
//! the PKCE (S256) authorization-code grant with rotating refresh tokens,
//! and RFC 7009 revocation. There are no accounts or passwords. Approval
//! happens on a consent page where the user pastes a bearer token the
//! server already accepts, and the grant then acts as whoever that token
//! belongs to.
//!
//! # The one seam: who a token is
//!
//! A server hands [`Provider::new`] a [`Verify`] function from a pasted
//! token to a [`Subject`], a short stable name of the server's choosing
//! (remind-me's `owner`, a rusty_tick user key, a Mill Board actor). Every
//! grant records its subject, and [`Provider::subject_of`] gives it back
//! for an access token. The server resolves the subject to a live identity
//! on each request, so removing a user also cuts off their OAuth tokens.
//!
//! # No HTTP framework
//!
//! [`Endpoints::handle`] takes a plain [`Request`] and returns a plain
//! [`Response`], or `None` for a path that is not an OAuth path. Each server
//! adapts it to its own stack (`rusty_axum`, `rusty_serve`) in a few lines.
//! Every call is synchronous: small local file I/O, no network.
//!
//! # Modules
//!
//! - [`issuer`]: issuer validation (an https origin, or http on localhost).
//! - [`store`]: the JSON state file of clients and token hashes.
//! - [`types`]: RFC 7591 client records and the RFC 6749 token response.
//! - [`provider`]: the policy: consent, issuance, refresh, revocation.
//! - [`endpoints`]: the HTTP surface, as data in and data out.
//! - `serve` (feature `serve`): the endpoints on `rusty_serve`.

pub mod endpoints;
pub mod issuer;
pub mod provider;
#[cfg(feature = "serve")]
pub mod serve;
pub mod store;
pub mod types;

pub use endpoints::{Branding, Endpoints, Method, Request, Response, PATHS, RESOURCE_PATH};
pub use issuer::{validate_issuer, Issuer, IssuerError};
pub use provider::{now_unix, Provider, Subject, Verify, CONSENT_PATH};
pub use store::{ClientSummary, RevokeSummary, StateStore, TokenCounts, TokenKind};

/// A fresh random secret: 32 bytes from the OS, as 64 hex characters. Used
/// for client ids, authorization codes, consent transactions and tokens.
///
/// # Errors
///
/// If the operating system's random source fails. There is no safe
/// fallback for a credential, so the caller refuses the request.
pub fn generate_token() -> std::io::Result<String> {
    let bytes = rusty_rand::bytes(32).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(rusty_hex::encode(&bytes))
}
