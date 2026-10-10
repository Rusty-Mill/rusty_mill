//! OAuth 2.1 resource-server authorization for the Streamable HTTP transport.
//!
//! Under MCP 2026-07-28 a protected server is an OAuth 2.1 **resource server**
//! with three obligations. This module covers all three:
//!
//! 1. **Publish Protected Resource Metadata** (RFC 9728) so a client can
//!    discover the authorization server — [`ProtectedResourceMetadata`].
//! 2. **Challenge unauthenticated requests** with a `WWW-Authenticate` header
//!    pointing at that document — [`Challenge`].
//! 3. **Reject every token not issued for this server** — enforced by
//!    [`RequireAuthLayer`] against [`AuthConfig::resource`].
//!
//! The third is the one worth dwelling on. The spec's "MCP servers **MUST NOT**
//! accept or transit any other tokens" exists to prevent a confused deputy: a
//! caller replaying a token minted for some other service and borrowing this
//! server's privileges. The layer enforces audience binding itself rather than
//! trusting each [`TokenValidator`] to remember.
//!
//! Authorization is HTTP-only. The spec says stdio servers **SHOULD NOT** use
//! it and should read credentials from the environment instead.
//!
//! # Wiring it up
//!
//! Guard the MCP route with [`RequireAuthLayer`] and mount the metadata
//! document beside it, unguarded, so a client that gets a `401` can still
//! discover where to authenticate:
//!
//! ```no_run
//! use std::sync::Arc;
//! use axum::{Json, Router, routing::{get, post}};
//! use rusty_mcp::auth::{
//!     AuthConfig, ProtectedResourceMetadata, RequireAuthLayer, StaticTokenValidator,
//!     VerifiedToken,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let validator = StaticTokenValidator::new().with_token(
//!     "dev-token",
//!     VerifiedToken::new(["https://mcp.example.com/mcp"]).with_scopes(["mcp:read"]),
//! );
//!
//! let auth = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))?
//!     .with_authorization_servers(["https://auth.example.com"])
//!     .with_scopes_supported(["mcp:read"])
//!     .with_required_scopes(["mcp:read"]);
//!
//! let metadata = ProtectedResourceMetadata::from_config(&auth);
//! let metadata_path = auth.metadata_path();
//!
//! let app: Router = Router::new()
//!     .route("/mcp", post(|| async { "your MCP handler" }).layer(RequireAuthLayer::new(auth)))
//!     .route(&metadata_path, get(move || async move { Json(metadata.clone()) }));
//! # let _ = app;
//! # Ok(())
//! # }
//! ```
//!
//! # Per-tool scopes
//!
//! [`AuthConfig::required_scopes`] guards the whole endpoint. For finer grain,
//! leave it empty and read the token inside a tool — the layer puts it in the
//! request extensions (`CallContext::caller()` hands a handler the principal
//! when the server is mounted through `rusty_mcp_axum`):
//!
//! ```no_run
//! use rusty_mcp::auth::VerifiedToken;
//!
//! fn require_scope<B>(request: &http::Request<B>, scope: &str) -> Result<(), String> {
//!     match request.extensions().get::<VerifiedToken>() {
//!         Some(token) if token.scopes.contains(scope) => Ok(()),
//!         Some(_) => Err(format!("this tool requires the `{scope}` scope")),
//!         // No token in extensions means the route is running unprotected.
//!         // Fail closed for a guarded tool.
//!         None => Err("this tool requires an authenticated session".to_owned()),
//!     }
//! }
//! ```

mod challenge;
mod config;
#[cfg(feature = "jwt")]
mod jwt;
mod layer;
mod metadata;
mod token;

pub use challenge::Challenge;
pub use config::{AuthConfig, AuthConfigError};
#[cfg(feature = "jwt")]
pub use jwt::{JwtValidator, JwtValidatorBuilder, JwtValidatorError};
pub use layer::{RequireAuth, RequireAuthLayer};
pub use metadata::ProtectedResourceMetadata;
pub use token::{StaticTokenValidator, TokenError, TokenValidator, ValidateFuture, VerifiedToken};
