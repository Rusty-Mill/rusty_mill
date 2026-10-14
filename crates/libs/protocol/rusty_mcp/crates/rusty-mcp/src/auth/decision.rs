//! The decisions of the resource-server contract, with no I/O.
//!
//! Which header is acceptable, whether a validated token is meant for this
//! resource, which scopes are missing, and what each refusal must say are pure
//! functions of the config and the token. Nothing here awaits, touches a
//! response type, or knows about `tower`, `axum` or `tokio`. The adapter
//! ([`super::layer`]) supplies the one effectful step — running the validator —
//! and turns a [`Refusal`] into its own response.

use std::collections::BTreeSet;

use http::{HeaderValue, StatusCode};
use rusty_json::json;

use super::{
    challenge::Challenge,
    config::AuthConfig,
    token::{TokenError, VerifiedToken},
};

/// Why a request was turned away.
#[derive(Debug)]
pub(crate) enum Rejection {
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

/// What to send back for a [`Rejection`], independent of any response type.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) status: StatusCode,
    /// The `WWW-Authenticate` value, if the client should be challenged.
    pub(crate) challenge: Option<String>,
    /// A compact JSON body.
    pub(crate) body: String,
}

impl Rejection {
    pub(crate) fn refusal(self, config: &AuthConfig) -> Refusal {
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
                return Refusal {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    challenge: None,
                    body: json!({
                        "error": "temporarily_unavailable",
                        "error_description": "token validation is temporarily unavailable",
                    })
                    .to_json_string(),
                };
            }
        };

        let challenge = challenge.with_resource_metadata(config.metadata_url());
        Refusal {
            status,
            body: json!({ "error": challenge.error().unwrap_or("unauthorized") }).to_json_string(),
            challenge: Some(challenge.to_header_value()),
        }
    }
}

/// Pull the bearer token out of the `Authorization` header.
///
/// The scheme is matched case-insensitively, as RFC 7235 requires.
pub(crate) fn bearer_token(header: Option<&HeaderValue>) -> Result<&str, Rejection> {
    let header = header.ok_or(Rejection::Missing)?;
    let header = header
        .to_str()
        .map_err(|_| Rejection::Malformed("the Authorization header is not valid text".into()))?;

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

/// Classify the validator's answer.
pub(crate) fn from_validation(
    result: Result<VerifiedToken, TokenError>,
) -> Result<VerifiedToken, Rejection> {
    result.map_err(|err| match err {
        TokenError::Invalid(reason) => Rejection::Invalid(reason),
        TokenError::Expired => Rejection::Invalid("the access token has expired".into()),
        TokenError::Unavailable(reason) => {
            // Log the cause; the client is told nothing beyond "try later".
            tracing::error!(%reason, "token validation failed");
            Rejection::Unavailable
        }
    })
}

/// Bind a validated token to this resource: audience, then scopes.
///
/// The audience check is what stops the server accepting a token minted for a
/// different resource, so it is enforced here rather than left to each
/// validator to remember.
pub(crate) fn check_claims(
    config: &AuthConfig,
    verified: VerifiedToken,
) -> Result<VerifiedToken, Rejection> {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auth::token::StaticTokenValidator;

    const RESOURCE: &str = "https://mcp.example.com/mcp";

    fn config(required: &[&str]) -> AuthConfig {
        AuthConfig::new(RESOURCE, Arc::new(StaticTokenValidator::new()))
            .expect("valid resource")
            .with_required_scopes(required.iter().map(|s| s.to_string()))
    }

    fn header(value: &str) -> HeaderValue {
        HeaderValue::from_str(value).expect("valid header")
    }

    #[test]
    fn parses_the_bearer_scheme_case_insensitively() {
        for value in ["Bearer abc", "bearer abc", "BEARER abc"] {
            assert_eq!(bearer_token(Some(&header(value))).ok(), Some("abc"));
        }
    }

    #[test]
    fn rejects_other_schemes_and_empty_tokens() {
        for value in ["Basic dXNlcjpwdw==", "abc", "Bearer   "] {
            assert!(
                matches!(
                    bearer_token(Some(&header(value))),
                    Err(Rejection::Malformed(_))
                ),
                "{value:?}"
            );
        }
    }

    #[test]
    fn a_missing_header_is_distinct_from_a_malformed_one() {
        assert!(matches!(bearer_token(None), Err(Rejection::Missing)));
    }

    #[test]
    fn a_header_that_is_not_text_is_malformed() {
        let value = HeaderValue::from_bytes(b"Bearer \xff").expect("opaque bytes");
        assert!(matches!(
            bearer_token(Some(&value)),
            Err(Rejection::Malformed(_))
        ));
    }

    #[test]
    fn classifies_each_validator_error() {
        let invalid = from_validation(Err(TokenError::Invalid("bad".into())));
        assert!(matches!(invalid, Err(Rejection::Invalid(r)) if r == "bad"));
        assert!(matches!(
            from_validation(Err(TokenError::Expired)),
            Err(Rejection::Invalid(_))
        ));
        assert!(matches!(
            from_validation(Err(TokenError::Unavailable("down".into()))),
            Err(Rejection::Unavailable)
        ));
        assert!(from_validation(Ok(VerifiedToken::default())).is_ok());
    }

    #[test]
    fn accepts_a_correctly_audienced_token() {
        let token = VerifiedToken::new([RESOURCE])
            .with_scopes(["mcp:read"])
            .with_subject("user-1");
        let token = check_claims(&config(&["mcp:read"]), token).expect("accepted");
        assert_eq!(token.subject.as_deref(), Some("user-1"));
    }

    #[test]
    fn rejects_a_token_minted_for_another_resource() {
        // The confused-deputy case: a real, validly-signed token that simply
        // was not issued for us.
        let token = VerifiedToken::new(["https://other.example.com/mcp"]).with_scopes(["mcp:read"]);
        assert!(matches!(
            check_claims(&config(&["mcp:read"]), token),
            Err(Rejection::WrongAudience)
        ));
    }

    #[test]
    fn rejects_a_token_with_no_audience_at_all() {
        let token = VerifiedToken::default().with_scopes(["mcp:read"]);
        assert!(matches!(
            check_claims(&config(&["mcp:read"]), token),
            Err(Rejection::WrongAudience)
        ));
    }

    #[test]
    fn honours_a_validator_that_bound_the_audience_itself() {
        let token = VerifiedToken::audience_checked_by_validator().with_scopes(["mcp:read"]);
        assert!(check_claims(&config(&["mcp:read"]), token).is_ok());
    }

    #[test]
    fn reports_every_missing_scope_at_once() {
        let token = VerifiedToken::new([RESOURCE]).with_scopes(["a"]);
        let Err(Rejection::InsufficientScope(missing)) =
            check_claims(&config(&["a", "b", "c"]), token)
        else {
            panic!("expected an insufficient-scope rejection");
        };
        assert_eq!(
            missing.into_iter().collect::<Vec<_>>(),
            vec!["b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn the_audience_is_checked_before_the_scopes() {
        // A token for another resource must not learn which scopes we want.
        let token = VerifiedToken::new(["https://other.example.com/mcp"]);
        assert!(matches!(
            check_claims(&config(&["mcp:read"]), token),
            Err(Rejection::WrongAudience)
        ));
    }

    #[test]
    fn each_refusal_carries_its_status_and_challenge() {
        let config = config(&["mcp:read"]);
        let cases = [
            (Rejection::Missing, StatusCode::UNAUTHORIZED, Some("Bearer")),
            (
                Rejection::Malformed("x".into()),
                StatusCode::BAD_REQUEST,
                Some("invalid_request"),
            ),
            (
                Rejection::Invalid("x".into()),
                StatusCode::UNAUTHORIZED,
                Some("invalid_token"),
            ),
            (
                Rejection::WrongAudience,
                StatusCode::UNAUTHORIZED,
                Some("invalid_token"),
            ),
            (
                Rejection::InsufficientScope(["b".to_string()].into()),
                StatusCode::FORBIDDEN,
                Some("insufficient_scope"),
            ),
            (
                Rejection::Unavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                None,
            ),
        ];
        for (rejection, status, needle) in cases {
            let label = format!("{rejection:?}");
            let refusal = rejection.refusal(&config);
            assert_eq!(refusal.status, status, "{label}");
            match needle {
                Some(needle) => {
                    let challenge = refusal.challenge.expect(&label);
                    assert!(challenge.contains(needle), "{label}: {challenge}");
                    assert!(challenge.contains("resource_metadata="), "{label}");
                }
                None => assert!(refusal.challenge.is_none(), "{label}"),
            }
        }
    }

    #[test]
    fn an_unavailable_validator_gets_no_challenge_and_says_so_in_json() {
        let refusal = Rejection::Unavailable.refusal(&config(&[]));
        assert_eq!(
            refusal.body,
            r#"{"error":"temporarily_unavailable","error_description":"token validation is temporarily unavailable"}"#
        );
    }
}
