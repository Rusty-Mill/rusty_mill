//! The `jwtAuth` route policy.
//!
//! Validation itself is [`rusty_mcp`]'s work: [`rusty_mcp::auth::JwtValidator`]
//! is a JWKS-backed validator with key caching, an algorithm allow-list pinned
//! before any key is touched, and a floor between refetches provoked by an
//! unknown `kid` — that last one matters, because without it anyone can force
//! unbounded outbound requests to the authorization server by presenting
//! tokens with random `kid` values.
//!
//! This crate supplies the two things that validator deliberately leaves to its
//! caller:
//!
//! - **The audience check.** [`rusty_mcp::auth::JwtValidator`] reads `aud` into
//!   [`VerifiedToken::audiences`] and checks nothing, because upstream's own
//!   layer binds it to a single canonical resource URI. Our `audiences` is a
//!   *list*, so the check lives in [`JwtAuthenticator::authenticate`]. Getting
//!   this wrong is the confused-deputy hole: a caller replays a token minted
//!   for some other service and borrows this gateway's privileges. There is a
//!   test named for exactly that.
//! - **A file-backed JWKS.** `JwtValidator` fetches over HTTP only, and our
//!   config also accepts `jwks: {file: ...}`. See [`FileJwks`].

mod file_jwks;

use std::sync::Arc;

use agentgateway_config::{JwtAuth, JwtSource};
use http::{HeaderMap, StatusCode, header};
use rusty_mcp::auth::{JwtValidator, TokenError, TokenValidator, VerifiedToken};

pub use file_jwks::{FileJwks, FileJwksError};

/// Failure to build an authenticator from configuration.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The JWKS file could not be read or parsed.
    #[error("{at}: {source}")]
    Jwks {
        /// Where in the configuration the policy came from.
        at: String,
        /// Underlying failure.
        #[source]
        source: FileJwksError,
    },

    /// The remote validator could not be constructed.
    #[error("{at}: building the JWT validator: {source}")]
    Validator {
        /// Where in the configuration the policy came from.
        at: String,
        /// Underlying failure.
        #[source]
        source: Box<rusty_mcp::auth::JwtValidatorError>,
    },
}

/// Why a request was refused, in terms a `WWW-Authenticate` header can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRejection {
    /// Status to answer with.
    pub status: StatusCode,
    /// RFC 6750 error code, absent when no token was presented at all.
    pub error: Option<&'static str>,
    /// Human-readable reason.
    pub description: String,
}

impl AuthRejection {
    /// The `WWW-Authenticate` challenge for this rejection.
    ///
    /// Omitted for a 503: the client's token may be perfectly good, and a
    /// challenge would send the user through a login that fixes nothing.
    pub fn challenge(&self) -> Option<String> {
        let error = self.error?;
        // Quoted-string values must not carry a bare `"` or `\`, and a
        // validator message can contain anything.
        let description = self.description.replace(['"', '\\'], "");
        Some(format!(
            "Bearer error=\"{error}\", error_description=\"{description}\""
        ))
    }

    fn unauthorized(error: &'static str, description: impl Into<String>) -> Self {
        AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            error: Some(error),
            description: description.into(),
        }
    }
}

/// Enforces a route's `jwtAuth` policy.
#[derive(Clone)]
pub struct JwtAuthenticator {
    audiences: Vec<String>,
    validator: Arc<dyn TokenValidator>,
}

impl std::fmt::Debug for JwtAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtAuthenticator")
            .field("audiences", &self.audiences)
            .finish_non_exhaustive()
    }
}

impl JwtAuthenticator {
    /// Build an authenticator from a route's policy.
    ///
    /// A `file:` JWKS is read here rather than per request, so a missing or
    /// malformed key set is a startup failure instead of a 503 storm later.
    pub fn new(config: &JwtAuth, at: &str) -> Result<Self, AuthError> {
        let validator: Arc<dyn TokenValidator> = match &config.jwks {
            JwtSource::File(path) => Arc::new(
                FileJwks::load(path, &config.issuer).map_err(|source| AuthError::Jwks {
                    at: at.to_string(),
                    source,
                })?,
            ),
            JwtSource::Url(url) => Arc::new(
                JwtValidator::builder(&config.issuer, url)
                    .build()
                    .map_err(|source| AuthError::Validator {
                        at: at.to_string(),
                        source: Box::new(source),
                    })?,
            ),
        };

        Ok(JwtAuthenticator {
            audiences: config.audiences.clone(),
            validator,
        })
    }

    /// Validate the request's bearer token.
    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<VerifiedToken, AuthRejection> {
        let token = bearer(headers)?;

        let verified = self.validator.validate(token).await.map_err(|err| match err {
            TokenError::Expired => {
                AuthRejection::unauthorized("invalid_token", "the access token has expired")
            }
            TokenError::Invalid(reason) => AuthRejection::unauthorized("invalid_token", reason),
            // Never 401. The token may be perfectly good and the JWKS endpoint
            // merely unreachable; answering 401 would tell the client to
            // re-authorize, sending a user through a login that fixes nothing
            // and hiding an outage as an auth problem.
            TokenError::Unavailable(reason) => AuthRejection {
                status: StatusCode::SERVICE_UNAVAILABLE,
                error: None,
                description: reason,
            },
            // `TokenError` is `#[non_exhaustive]`, so a future rusty_mcp
            // release can add a variant. Deny rather than guess: a rejection
            // this gateway did not anticipate is not one to wave through, and
            // the log line is how we find out the mapping needs updating.
            other => {
                tracing::warn!(error = %other, "unrecognized token rejection; denying");
                AuthRejection::unauthorized("invalid_token", other.to_string())
            }
        })?;

        self.check_audience(&verified)?;
        Ok(verified)
    }

    /// Reject a token that was not minted for us.
    ///
    /// An empty `audiences` accepts any audience, matching the config docs. It
    /// is a deliberate opt-out, not an oversight: some deployments terminate
    /// audience binding upstream.
    fn check_audience(&self, token: &VerifiedToken) -> Result<(), AuthRejection> {
        if self.audiences.is_empty() || token.audience_verified {
            return Ok(());
        }

        let matched = token
            .audiences
            .iter()
            .any(|found| self.audiences.iter().any(|want| want == found));

        if matched {
            return Ok(());
        }

        // The reason is deliberately not echoed back: telling a caller which
        // audiences we accept helps them go and find a token that carries one.
        tracing::debug!(
            presented = ?token.audiences,
            accepted = ?self.audiences,
            "rejecting a token minted for another audience"
        );
        Err(AuthRejection::unauthorized(
            "invalid_token",
            "the access token was not issued for this resource",
        ))
    }
}

/// Pull the bearer token out of an `Authorization` header.
fn bearer(headers: &HeaderMap) -> Result<&str, AuthRejection> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Err(AuthRejection {
            status: StatusCode::UNAUTHORIZED,
            // RFC 6750: no error code when no credentials were presented --
            // the challenge alone tells the client what to do.
            error: None,
            description: "no bearer token was presented".into(),
        });
    };

    let value = value.to_str().map_err(|_| {
        AuthRejection::unauthorized("invalid_request", "the Authorization header is not valid ASCII")
    })?;

    // The scheme is case-insensitive per RFC 7235.
    let (scheme, token) = value.split_once(' ').ok_or_else(|| {
        AuthRejection::unauthorized("invalid_request", "the Authorization header is malformed")
    })?;

    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(AuthRejection::unauthorized(
            "invalid_request",
            "only the Bearer scheme is accepted",
        ));
    }

    let token = token.trim();
    if token.is_empty() {
        return Err(AuthRejection::unauthorized(
            "invalid_request",
            "the bearer token is empty",
        ));
    }
    Ok(token)
}

/// A bare `Bearer` challenge, for a route that requires a token.
pub const BEARER_CHALLENGE: &str = "Bearer";

#[cfg(test)]
mod tests;
