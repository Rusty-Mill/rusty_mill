//! The `tower` layer enforcing the resource-server contract.

use std::{
    collections::BTreeSet,
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::response::{IntoResponse, Response};
use http::{
    Request, StatusCode,
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
};

use super::{
    challenge::Challenge,
    config::AuthConfig,
    token::{TokenError, VerifiedToken},
};

/// Applies bearer-token authorization to the service it wraps.
///
/// Mount it on the MCP endpoint only. The Protected Resource Metadata document
/// must stay reachable without a token, or a client that gets a `401` can never
/// discover where to authenticate — [`crate::runtime::serve`] wires both up
/// correctly when [`crate::HttpConfig::auth`] is set.
///
/// ```
/// use std::sync::Arc;
/// use rusty_mcp::auth::{AuthConfig, RequireAuthLayer, StaticTokenValidator, VerifiedToken};
///
/// let validator = StaticTokenValidator::new().with_token(
///     "token-abc",
///     VerifiedToken::new(["https://mcp.example.com/mcp"]).with_scopes(["mcp:read"]),
/// );
///
/// let config = AuthConfig::new("https://mcp.example.com/mcp", Arc::new(validator))
///     .expect("valid canonical resource URI")
///     .with_authorization_servers(["https://auth.example.com"])
///     .with_required_scopes(["mcp:read"]);
///
/// let layer = RequireAuthLayer::new(config);
/// # let _ = layer;
/// ```
#[derive(Debug, Clone)]
pub struct RequireAuthLayer {
    config: Arc<AuthConfig>,
}

impl RequireAuthLayer {
    /// Build a layer enforcing `config`.
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Build a layer from an already-shared config.
    pub fn from_shared(config: Arc<AuthConfig>) -> Self {
        Self { config }
    }

    /// The configuration being enforced.
    pub fn config(&self) -> &Arc<AuthConfig> {
        &self.config
    }
}

impl<S> tower_layer::Layer<S> for RequireAuthLayer {
    type Service = RequireAuth<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireAuth {
            inner,
            config: Arc::clone(&self.config),
        }
    }
}

/// Service produced by [`RequireAuthLayer`].
#[derive(Debug, Clone)]
pub struct RequireAuth<S> {
    inner: S,
    config: Arc<AuthConfig>,
}

impl<S, ReqBody> tower_service::Service<Request<ReqBody>> for RequireAuth<S>
where
    S: tower_service::Service<Request<ReqBody>, Error = Infallible> + Clone + Send + 'static,
    S::Response: IntoResponse,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        // Swap in the readied service: `self.inner` is the one that passed
        // `poll_ready`, and the fresh clone may not be ready yet.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let config = Arc::clone(&self.config);

        Box::pin(async move {
            let token = match authorize(&config, request.headers().get(AUTHORIZATION)).await {
                Ok(token) => token,
                Err(rejection) => return Ok(rejection.into_response(&config)),
            };

            // Hand the claims to the handler. Tools reach them through the
            // `http::request::Parts` that the transport puts in the request
            // context, which is how per-tool scope checks are written.
            request.extensions_mut().insert(token);

            inner.call(request).await.map(IntoResponse::into_response)
        })
    }
}

/// Why a request was turned away.
#[derive(Debug)]
enum Rejection {
    /// No credentials at all.
    Missing,
    /// The `Authorization` header could not be parsed.
    Malformed(String),
    /// A token was presented but did not validate.
    Invalid(String),
    /// Valid token, wrong audience. Treated as invalid rather than forbidden:
    /// the token is unusable here and the client needs a new one.
    WrongAudience,
    /// Valid token, missing scopes.
    InsufficientScope(BTreeSet<String>),
    /// The validator could not reach its dependencies.
    Unavailable,
}

impl Rejection {
    fn into_response(self, config: &AuthConfig) -> Response {
        let metadata_url = config.metadata_url();

        let (status, challenge) = match self {
            Rejection::Missing => (
                StatusCode::UNAUTHORIZED,
                Challenge::unauthorized().with_scope(&config.required_scopes),
            ),
            Rejection::Malformed(reason) => (
                StatusCode::BAD_REQUEST,
                Challenge::invalid_request(reason).with_scope(&config.required_scopes),
            ),
            Rejection::Invalid(reason) => (
                StatusCode::UNAUTHORIZED,
                Challenge::invalid_token(reason).with_scope(&config.required_scopes),
            ),
            Rejection::WrongAudience => (
                StatusCode::UNAUTHORIZED,
                Challenge::invalid_token(format!(
                    "the access token was not issued for {}",
                    config.resource()
                ))
                .with_scope(&config.required_scopes),
            ),
            Rejection::InsufficientScope(missing) => (
                StatusCode::FORBIDDEN,
                Challenge::insufficient_scope(&missing),
            ),
            // No challenge: the client's credentials may be fine, so inviting
            // it to re-authorize would send the user through a pointless login.
            Rejection::Unavailable => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "temporarily_unavailable",
                        "error_description": "token validation is temporarily unavailable",
                    })),
                )
                    .into_response();
            }
        };

        let challenge = challenge.with_resource_metadata(metadata_url);
        let body = serde_json::json!({
            "error": challenge.error().unwrap_or("unauthorized"),
        });

        let mut response = (status, axum::Json(body)).into_response();
        match challenge.to_header_value().parse() {
            Ok(value) => {
                response.headers_mut().insert(WWW_AUTHENTICATE, value);
            }
            Err(err) => {
                // Should be unreachable: `Challenge` strips control characters.
                tracing::error!(%err, "built an unusable WWW-Authenticate value");
            }
        }
        response
    }
}

/// Run the full resource-server check over an `Authorization` header.
async fn authorize(
    config: &AuthConfig,
    header: Option<&http::HeaderValue>,
) -> Result<VerifiedToken, Rejection> {
    let Some(header) = header else {
        return Err(Rejection::Missing);
    };

    let header = header
        .to_str()
        .map_err(|_| Rejection::Malformed("the Authorization header is not valid text".into()))?;

    let token = parse_bearer(header)?;

    let verified = config.validator.validate(token).await.map_err(|err| {
        match err {
            TokenError::Invalid(reason) => Rejection::Invalid(reason),
            TokenError::Expired => Rejection::Invalid("the access token has expired".into()),
            TokenError::Unavailable(reason) => {
                // Log the cause; the client is told nothing beyond "try later".
                tracing::error!(%reason, "token validation failed");
                Rejection::Unavailable
            }
        }
    })?;

    // Audience binding. This is the check that stops the server accepting a
    // token minted for a different resource, so it is enforced here rather than
    // left to each validator to remember.
    if !verified.audience_verified
        && !verified
            .audiences
            .iter()
            .any(|aud| config.matches_audience(aud))
    {
        tracing::warn!(
            audiences = ?verified.audiences,
            resource = config.resource(),
            "rejected a token whose audience does not name this resource"
        );
        return Err(Rejection::WrongAudience);
    }

    if !verified.has_scopes(&config.required_scopes) {
        let missing = verified
            .missing_scopes(&config.required_scopes)
            .into_iter()
            .collect();
        return Err(Rejection::InsufficientScope(missing));
    }

    Ok(verified)
}

/// Pull the credentials out of a `Bearer <token>` header.
///
/// The scheme is matched case-insensitively, as RFC 7235 requires.
fn parse_bearer(header: &str) -> Result<&str, Rejection> {
    let (scheme, token) = header
        .split_once(' ')
        .ok_or_else(|| Rejection::Malformed("expected `Bearer <token>`".into()))?;

    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(Rejection::Malformed(format!(
            "unsupported authorization scheme `{scheme}`; expected Bearer"
        )));
    }

    let token = token.trim();
    if token.is_empty() {
        return Err(Rejection::Malformed("the bearer token is empty".into()));
    }

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token::StaticTokenValidator;

    const RESOURCE: &str = "https://mcp.example.com/mcp";

    fn config_with(validator: StaticTokenValidator, required: &[&str]) -> AuthConfig {
        AuthConfig::new(RESOURCE, Arc::new(validator))
            .expect("valid resource")
            .with_required_scopes(required.iter().map(|s| s.to_string()))
    }

    fn bearer(value: &str) -> http::HeaderValue {
        http::HeaderValue::from_str(value).expect("valid header")
    }

    #[test]
    fn parses_the_bearer_scheme_case_insensitively() {
        assert_eq!(parse_bearer("Bearer abc").ok(), Some("abc"));
        assert_eq!(parse_bearer("bearer abc").ok(), Some("abc"));
        assert_eq!(parse_bearer("BEARER abc").ok(), Some("abc"));
    }

    #[test]
    fn rejects_other_schemes_and_empty_tokens() {
        assert!(matches!(
            parse_bearer("Basic dXNlcjpwdw=="),
            Err(Rejection::Malformed(_))
        ));
        assert!(matches!(parse_bearer("abc"), Err(Rejection::Malformed(_))));
        assert!(matches!(
            parse_bearer("Bearer   "),
            Err(Rejection::Malformed(_))
        ));
    }

    #[tokio::test]
    async fn accepts_a_correctly_audienced_token() {
        let validator = StaticTokenValidator::new().with_token(
            "good",
            VerifiedToken::new([RESOURCE])
                .with_scopes(["mcp:read"])
                .with_subject("user-1"),
        );
        let config = config_with(validator, &["mcp:read"]);

        let token = authorize(&config, Some(&bearer("Bearer good")))
            .await
            .expect("should be accepted");
        assert_eq!(token.subject.as_deref(), Some("user-1"));
    }

    #[tokio::test]
    async fn rejects_a_token_minted_for_another_resource() {
        // The confused-deputy case: a real, validly-signed token that simply
        // was not issued for us.
        let validator = StaticTokenValidator::new().with_token(
            "other",
            VerifiedToken::new(["https://other.example.com/mcp"]).with_scopes(["mcp:read"]),
        );
        let config = config_with(validator, &["mcp:read"]);

        assert!(matches!(
            authorize(&config, Some(&bearer("Bearer other"))).await,
            Err(Rejection::WrongAudience)
        ));
    }

    #[tokio::test]
    async fn rejects_a_token_with_no_audience_at_all() {
        let validator = StaticTokenValidator::new()
            .with_token("bare", VerifiedToken::default().with_scopes(["mcp:read"]));
        let config = config_with(validator, &["mcp:read"]);

        assert!(matches!(
            authorize(&config, Some(&bearer("Bearer bare"))).await,
            Err(Rejection::WrongAudience)
        ));
    }

    #[tokio::test]
    async fn honours_a_validator_that_bound_the_audience_itself() {
        let validator = StaticTokenValidator::new().with_token(
            "delegated",
            VerifiedToken::audience_checked_by_validator().with_scopes(["mcp:read"]),
        );
        let config = config_with(validator, &["mcp:read"]);

        assert!(
            authorize(&config, Some(&bearer("Bearer delegated")))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn reports_every_missing_scope_at_once() {
        let validator = StaticTokenValidator::new()
            .with_token("thin", VerifiedToken::new([RESOURCE]).with_scopes(["a"]));
        let config = config_with(validator, &["a", "b", "c"]);

        let Err(Rejection::InsufficientScope(missing)) =
            authorize(&config, Some(&bearer("Bearer thin"))).await
        else {
            panic!("expected an insufficient-scope rejection");
        };
        assert_eq!(
            missing.into_iter().collect::<Vec<_>>(),
            vec!["b".to_string(), "c".to_string()]
        );
    }

    #[tokio::test]
    async fn missing_header_is_distinct_from_an_invalid_token() {
        let config = config_with(StaticTokenValidator::new(), &[]);

        assert!(matches!(
            authorize(&config, None).await,
            Err(Rejection::Missing)
        ));
        assert!(matches!(
            authorize(&config, Some(&bearer("Bearer nope"))).await,
            Err(Rejection::Invalid(_))
        ));
    }
}
