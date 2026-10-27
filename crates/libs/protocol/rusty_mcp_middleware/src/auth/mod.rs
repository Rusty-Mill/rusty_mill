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
//!    `serve::AuthorizedHandler` (feature `serve`) against
//!    [`AuthConfig::resource`].
//!
//! The third is the one worth dwelling on. The spec's "MCP servers **MUST NOT**
//! accept or transit any other tokens" exists to prevent a confused deputy: a
//! caller replaying a token minted for some other service and borrowing this
//! server's privileges. The check enforces audience binding itself rather than
//! trusting each [`TokenValidator`] to remember.
//!
//! Authorization is HTTP-only. The spec says stdio servers **SHOULD NOT** use
//! it and should read credentials from the environment instead.
//!
//! # Wiring it up
//!
//! Guard the MCP route with `serve::AuthorizedHandler` and serve the
//! metadata document ([`ProtectedResourceMetadata`]) beside it, unguarded, so
//! a client that gets a `401` can still discover where to authenticate. The
//! handler receives the [`VerifiedToken`], so a tool can check its own scopes
//! when [`AuthConfig::required_scopes`] is left empty.
//!
//! That adapter polls the validator without a reactor, so it suits a
//! validator that answers from memory. An async server using the JWKS-backed
//! `JwtValidator` (feature `jwt`) awaits
//! [`TokenValidator::validate`] itself and checks the audience against
//! [`AuthConfig::resource`], as the gateway does.

mod challenge;
mod config;
#[cfg(feature = "serve")]
pub(crate) mod decision;
#[cfg(feature = "jwt")]
mod jwt;
mod metadata;
mod token;

pub use challenge::Challenge;
pub use config::{AuthConfig, AuthConfigError};
#[cfg(feature = "jwt")]
pub use jwt::{JwtAlgorithm, JwtValidator, JwtValidatorBuilder, JwtValidatorError};
pub use metadata::ProtectedResourceMetadata;
pub use token::{StaticTokenValidator, TokenError, TokenValidator, ValidateFuture, VerifiedToken};
