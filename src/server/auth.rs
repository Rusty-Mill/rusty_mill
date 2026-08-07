//! Pluggable enforcement of an [`AgentCard`](crate::types::AgentCard)'s
//! declared `securitySchemes`/`securityRequirements` (spec Section 4.5).
//!
//! This crate can declare and transmit security scheme metadata, but it
//! has no way to know what a valid credential *is* for your deployment
//! (a real API key, a JWT signed by your issuer, ...); that decision
//! belongs to the application. Implement [`AuthVerifier`] to make it and
//! hand it to [`AgentServer::with_auth_verifier`](super::AgentServer::with_auth_verifier);
//! every binding then extracts whatever raw credential material each
//! declared scheme calls for and asks your verifier whether it's valid
//! before dispatching the request.
//!
//! # What's extracted, and what isn't
//!
//! - `apiKey` schemes with `location: "header"`, and `http`/`oauth2`/
//!   `openIdConnect` schemes (read from the `Authorization` header,
//!   stripping the scheme's own name as the expected prefix - `Bearer`
//!   for OAuth2/OIDC) are extracted on all three protocol bindings.
//! - `apiKey` schemes with `location: "query"` or `"cookie"` are only
//!   extracted on the REST binding (JSON-RPC has no meaningful query
//!   string, and gRPC has neither).
//! - `mtls` schemes: this crate's own servers never terminate TLS
//!   themselves, so there's no client certificate to inspect directly.
//!   [`AgentServer::with_mtls_header`](super::AgentServer::with_mtls_header)
//!   lets a deployment behind a TLS-terminating reverse proxy (which
//!   verified the client certificate and recorded the result in a
//!   header/metadata entry - e.g. nginx's `ssl-client-verify`, Envoy's
//!   `x-forwarded-client-cert`) point every `mtls` scheme at that entry
//!   instead. Without it configured, `mtls` requirements remain
//!   unsatisfiable exactly as before: no credential is ever extracted, so
//!   an `AuthVerifier` is simply never called for them.
//!
//! # Semantics
//!
//! `AgentCard.securityRequirements` is a list of alternatives (OpenAPI
//! `security` semantics): satisfying *any one* entry is sufficient. Each
//! entry is itself a map of scheme name -> required scopes, all of which
//! must be satisfied together. An empty `securityRequirements` list means
//! the agent is public - no [`AuthVerifier`] is consulted and no
//! credentials are required.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::error::{A2aError, Result};
use crate::types::{SecurityRequirement, SecurityScheme};

/// The outcome of a successful [`AuthVerifier::verify`] call: whatever the
/// application wants to say about who this request is from. Not currently
/// surfaced to [`AgentExecutor`](super::AgentExecutor) - enforcement
/// (accept/reject), not identity propagation into the executor, is this
/// module's scope for now.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    /// An opaque identifier for the authenticated caller (e.g. a subject
    /// claim, an API key's owner), for logging/debugging.
    pub principal: Option<String>,
    /// OAuth2/OIDC scopes or equivalent, if the verifier wants to record
    /// them.
    pub scopes: Vec<String>,
}

impl AuthContext {
    pub fn new(principal: impl Into<String>) -> Self {
        AuthContext {
            principal: Some(principal.into()),
            scopes: Vec::new(),
        }
    }

    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }
}

/// Raw, not-yet-validated credential material extracted from a request,
/// keyed by the `AgentCard.securitySchemes` name it was extracted for.
#[derive(Debug, Clone, Default)]
pub struct Credentials(pub HashMap<String, String>);

/// Decides whether extracted [`Credentials`] satisfy a
/// [`SecurityRequirement`], and if so, who the caller is.
///
/// Implement this against your own auth provider - checking an API key
/// against a database, verifying a JWT's signature and claims against
/// your issuer, and so on - rather than expecting this crate to do it for
/// you generically.
#[async_trait]
pub trait AuthVerifier: Send + Sync {
    /// Called once per alternative in the effective security requirement
    /// list (`AgentCard.securityRequirements`, or its extended-agent-card
    /// equivalent - spec Section 3.1.11) for which every scheme name in
    /// `requirement.schemes` had a credential extracted. Return `Ok` to
    /// accept the request, or `Err` (conventionally
    /// [`A2aError::Unauthenticated`] or [`A2aError::PermissionDenied`]) to
    /// reject this alternative - the caller tries the next one, if any,
    /// before failing the request.
    async fn verify(
        &self,
        requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext>;
}

/// Extracts whatever raw credential material each of `schemes` calls for,
/// via `lookup_header` (case-insensitive header/metadata lookup) and
/// `lookup_query` (`None` on bindings with no query string, e.g. JSON-RPC
/// and gRPC). `mtls_header`, if set (see
/// [`AgentServer::with_mtls_header`](super::AgentServer::with_mtls_header)),
/// is the header/metadata key name an `mtls` scheme's credential is read
/// from via `lookup_header`.
pub(crate) fn extract_credentials(
    schemes: &HashMap<String, SecurityScheme>,
    lookup_header: impl Fn(&str) -> Option<String>,
    lookup_query: Option<&HashMap<String, String>>,
    mtls_header: Option<&str>,
) -> Credentials {
    let mut found = HashMap::new();
    for (name, scheme) in schemes {
        let value = match scheme {
            SecurityScheme::ApiKey {
                api_key_security_scheme: s,
            } => match s.location.as_str() {
                "header" => lookup_header(&s.name),
                "query" => lookup_query.and_then(|q| q.get(&s.name)).cloned(),
                "cookie" => lookup_header("cookie").and_then(|c| cookie_value(&c, &s.name)),
                _ => None,
            },
            SecurityScheme::HttpAuth {
                http_auth_security_scheme: s,
            } => lookup_header("authorization").and_then(|v| strip_scheme_prefix(&v, &s.scheme)),
            SecurityScheme::OAuth2 { .. } | SecurityScheme::OpenIdConnect { .. } => {
                lookup_header("authorization").and_then(|v| strip_scheme_prefix(&v, "Bearer"))
            }
            SecurityScheme::MutualTls { .. } => mtls_header.and_then(&lookup_header),
        };
        if let Some(v) = value {
            found.insert(name.clone(), v);
        }
    }
    Credentials(found)
}

fn strip_scheme_prefix(header_value: &str, scheme: &str) -> Option<String> {
    let (prefix, rest) = header_value.split_once(' ')?;
    prefix
        .eq_ignore_ascii_case(scheme)
        .then(|| rest.trim().to_string())
}

fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    cookie_header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// Tries each alternative in `requirements` (OR semantics) against
/// `verifier`, requiring every scheme in an alternative's `schemes` map to
/// have a matching entry in `credentials` (AND semantics) before even
/// asking the verifier. Returns the first successful [`AuthContext`]. If
/// every alternative was either missing credentials or rejected, returns
/// the most recent rejection, or a generic [`A2aError::Unauthenticated`]
/// if none had enough credentials to even attempt verification.
pub(crate) async fn authenticate_against(
    requirements: &[SecurityRequirement],
    verifier: &dyn AuthVerifier,
    credentials: &Credentials,
) -> Result<AuthContext> {
    let mut last_err = None;
    for requirement in requirements {
        let has_all = requirement
            .schemes
            .keys()
            .all(|name| credentials.0.contains_key(name));
        if !has_all {
            continue;
        }
        match verifier.verify(requirement, credentials).await {
            Ok(ctx) => return Ok(ctx),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        A2aError::Unauthenticated(
            "no credentials found matching any declared security requirement".to_string(),
        )
    }))
}
