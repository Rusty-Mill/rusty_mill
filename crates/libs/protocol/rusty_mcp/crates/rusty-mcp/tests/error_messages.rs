//! The error types' messages and sources are part of what callers log and
//! chain, so they are pinned here.

use std::error::Error;
use std::sync::Arc;

use rusty_mcp::auth::{AuthConfig, StaticTokenValidator, TokenError};

fn config_error(resource: &str) -> rusty_mcp::auth::AuthConfigError {
    match AuthConfig::new(resource, Arc::new(StaticTokenValidator::new())) {
        Err(err) => err,
        Ok(_) => panic!("`{resource}` should be rejected"),
    }
}

#[test]
fn token_errors_read_as_written() {
    assert_eq!(
        TokenError::Invalid("bad signature".into()).to_string(),
        "bad signature"
    );
    assert_eq!(
        TokenError::Expired.to_string(),
        "the access token has expired"
    );
    assert_eq!(
        TokenError::Unavailable("jwks timed out".into()).to_string(),
        "token validation is unavailable: jwks timed out"
    );
    assert!(TokenError::Expired.source().is_none());
}

#[test]
fn config_errors_name_the_uri_and_chain_the_parse_failure() {
    let fragment = config_error("https://mcp.example.com/mcp#top");
    assert_eq!(
        fragment.to_string(),
        "resource URI `https://mcp.example.com/mcp#top` must not contain a fragment"
    );
    assert!(fragment.source().is_none());

    let query = config_error("https://mcp.example.com/mcp?x=1");
    assert_eq!(
        query.to_string(),
        "resource URI `https://mcp.example.com/mcp?x=1` must not contain a query string"
    );

    let malformed = config_error("not a uri");
    let message = malformed.to_string();
    assert!(
        message.starts_with("`not a uri` is not a valid absolute URI: "),
        "{message}"
    );
    assert!(
        malformed.source().is_some(),
        "the parse failure is the source"
    );
}

#[cfg(feature = "jwt")]
#[test]
fn the_jwt_builder_error_reads_as_written() {
    use rusty_mcp::auth::{JwtValidator, JwtValidatorError};

    let Err(err) = JwtValidator::builder("https://auth.example.com", "https://auth/jwks")
        .with_algorithms([])
        .build()
    else {
        panic!("no algorithms should be rejected");
    };
    assert!(matches!(err, JwtValidatorError::NoAlgorithms));
    assert_eq!(
        err.to_string(),
        "at least one signing algorithm must be allowed"
    );
    assert!(err.source().is_none());
}
