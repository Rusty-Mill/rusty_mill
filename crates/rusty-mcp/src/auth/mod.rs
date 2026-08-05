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
//! Set [`crate::HttpConfig::auth`] and [`crate::runtime::serve`] mounts both the
//! guarded MCP endpoint and the unauthenticated metadata document:
//!
//! ```no_run
//! use std::sync::Arc;
//! use rusty_mcp::{HttpConfig, ServerConfig, Transport};
//! use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, VerifiedToken};
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
//! let config = ServerConfig {
//!     transport: Transport::Http(HttpConfig {
//!         auth: Some(Arc::new(auth)),
//!         ..Default::default()
//!     }),
//!     ..Default::default()
//! };
//! # let _ = config;
//! # Ok(())
//! # }
//! ```
//!
//! Or apply [`RequireAuthLayer`] yourself if you build your own router.
//!
//! # Per-tool scopes
//!
//! [`AuthConfig::required_scopes`] guards the whole endpoint. For finer grain,
//! leave it empty and read the token inside a tool — the layer puts it in the
//! request extensions, which the transport forwards as `http::request::Parts`:
//!
//! ```no_run
//! use rmcp::model::ErrorData;
//! use rmcp::service::RequestContext;
//! use rmcp::RoleServer;
//! use rusty_mcp::auth::VerifiedToken;
//!
//! fn require_scope(ctx: &RequestContext<RoleServer>, scope: &str) -> Result<(), ErrorData> {
//!     let token = ctx
//!         .extensions
//!         .get::<http::request::Parts>()
//!         .and_then(|parts| parts.extensions.get::<VerifiedToken>());
//!
//!     match token {
//!         Some(token) if token.scopes.contains(scope) => Ok(()),
//!         Some(_) => Err(ErrorData::invalid_request(
//!             format!("this tool requires the `{scope}` scope"),
//!             None,
//!         )),
//!         // No token in extensions means the server is running unprotected
//!         // (stdio, or HTTP without `auth`). Fail closed for a guarded tool.
//!         None => Err(ErrorData::invalid_request(
//!             "this tool requires an authenticated session",
//!             None,
//!         )),
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
