//! The `tower` layer enforcing the resource-server contract.

use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use http::{
    Request, Response, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
};

use super::{
    config::AuthConfig,
    decision::{
        AuthorizationHeader, Refusal, Rejection, bearer_token, check_claims, from_validation,
    },
    token::VerifiedToken,
};

/// Applies bearer-token authorization to the service it wraps.
///
/// Mount it on the MCP endpoint only. The Protected Resource Metadata document
/// must stay reachable without a token, or a client that gets a `401` can never
/// discover where to authenticate — see the wiring example in [`crate::auth`].
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

impl<S, ReqBody, ResBody> tower_service::Service<Request<ReqBody>> for RequireAuth<S>
where
    S: tower_service::Service<Request<ReqBody>, Response = Response<ResBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: From<String> + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<ResBody>, Infallible>> + Send>>;

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
                Err(rejection) => return Ok(into_response(rejection.refusal(&config))),
            };

            // Hand the claims to the handler. Tools reach them through the
            // `http::request::Parts` that the transport puts in the request
            // context, which is how per-tool scope checks are written.
            request.extensions_mut().insert(token);

            inner.call(request).await
        })
    }
}

/// Run the full resource-server check over an `Authorization` header: the
/// pure steps are in [`super::decision`]; only the validator is awaited here.
async fn authorize(
    config: &AuthConfig,
    header: Option<&http::HeaderValue>,
) -> Result<VerifiedToken, Rejection> {
    let token = bearer_token(authorization_header(header))?;
    let verified = from_validation(config.validator.validate(token).await)?;
    check_claims(config, verified)
}

/// Read the header as the core wants it: absent, not text, or text.
fn authorization_header(header: Option<&http::HeaderValue>) -> AuthorizationHeader<'_> {
    match header {
        None => AuthorizationHeader::Absent,
        Some(value) => value
            .to_str()
            .map_or(AuthorizationHeader::NotText, AuthorizationHeader::Text),
    }
}

/// Turn a refusal into a response.
fn into_response<B: From<String>>(refusal: Refusal) -> Response<B> {
    let status =
        StatusCode::from_u16(refusal.status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(B::from(refusal.body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    if let Some(challenge) = refusal.challenge {
        match challenge.parse() {
            Ok(value) => {
                response.headers_mut().insert(WWW_AUTHENTICATE, value);
            }
            Err(err) => {
                // Should be unreachable: `Challenge` strips control characters.
                rusty_log::error!(%err, "built an unusable WWW-Authenticate value");
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token::StaticTokenValidator;

    const RESOURCE: &str = "https://mcp.example.com/mcp";

    fn config_with(validator: StaticTokenValidator) -> AuthConfig {
        AuthConfig::new(RESOURCE, Arc::new(validator)).expect("valid resource")
    }

    fn bearer(value: &str) -> http::HeaderValue {
        http::HeaderValue::from_str(value).expect("valid header")
    }

    #[rusty_tokio::test]
    async fn accepts_a_token_the_validator_vouches_for() {
        let validator = StaticTokenValidator::new().with_token(
            "good",
            VerifiedToken::new([RESOURCE]).with_subject("user-1"),
        );
        let config = config_with(validator);

        let token = authorize(&config, Some(&bearer("Bearer good")))
            .await
            .expect("should be accepted");
        assert_eq!(token.subject.as_deref(), Some("user-1"));
    }

    #[rusty_tokio::test]
    async fn missing_header_is_distinct_from_an_invalid_token() {
        let config = config_with(StaticTokenValidator::new());

        assert!(matches!(
            authorize(&config, None).await,
            Err(Rejection::Missing)
        ));
        assert!(matches!(
            authorize(&config, Some(&bearer("Bearer nope"))).await,
            Err(Rejection::Invalid(_))
        ));
    }

    #[test]
    fn the_header_is_read_as_absent_not_text_or_text() {
        let opaque = http::HeaderValue::from_bytes(b"Bearer \xff").expect("opaque bytes");
        let text = bearer("Bearer abc");

        assert_eq!(authorization_header(None), AuthorizationHeader::Absent);
        assert_eq!(
            authorization_header(Some(&opaque)),
            AuthorizationHeader::NotText
        );
        assert_eq!(
            authorization_header(Some(&text)),
            AuthorizationHeader::Text("Bearer abc")
        );
    }

    #[rusty_tokio::test]
    async fn a_non_text_header_is_a_bad_request_not_a_missing_one() {
        let config = config_with(StaticTokenValidator::new());
        let opaque = http::HeaderValue::from_bytes(b"Bearer \xff").expect("opaque bytes");

        let Err(rejection) = authorize(&config, Some(&opaque)).await else {
            panic!("an opaque header must be refused");
        };
        assert_eq!(rejection.refusal(&config).status.as_u16(), 400);
    }

    #[rusty_tokio::test]
    async fn a_refusal_becomes_a_json_response_with_its_challenge() {
        let config = config_with(StaticTokenValidator::new());
        let response: Response<String> = into_response(Rejection::Missing.refusal(&config));

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert!(
            response.headers()[WWW_AUTHENTICATE]
                .to_str()
                .expect("text")
                .starts_with("Bearer")
        );
        assert_eq!(response.into_body(), r#"{"error":"unauthorized"}"#);
    }

    #[rusty_tokio::test]
    async fn a_challenge_free_refusal_sets_no_www_authenticate() {
        let config = config_with(StaticTokenValidator::new());
        let response: Response<String> = into_response(Rejection::Unavailable.refusal(&config));

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(response.headers().get(WWW_AUTHENTICATE).is_none());
    }
}
