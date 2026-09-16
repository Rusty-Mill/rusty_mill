//! A JWKS read from local disk.
//!
//! [`rusty_mcp::auth::JwtValidator`] fetches its key set over HTTP, which is
//! what most deployments want. Our config also accepts `jwks: {file: ...}` —
//! for air-gapped installs, and for tests that should not stand up an HTTP
//! server to prove a signature check works.
//!
//! The verification below deliberately mirrors `JwtValidator::verify`,
//! including the ordering that matters: the algorithm is pinned to the
//! allow-list *before* any key is loaded, which is what defeats the `alg: none`
//! and RS256-verified-as-HS256 family of attacks. If `rusty_mcp` grows a
//! `JwtValidator::from_jwks` constructor this module should collapse into it;
//! duplicated crypto is duplicated risk.
//!
//! Keys are read once at startup. A rotated file needs a restart to take
//! effect — an acceptable trade for a local file, and the reason the remote
//! form exists.

use std::collections::BTreeSet;
use std::path::Path;

use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header, errors::ErrorKind, jwk::JwkSet,
};
use rusty_mcp::auth::{TokenError, TokenValidator, ValidateFuture, VerifiedToken};
use serde_json::Value;

/// Failure to load a JWKS from disk.
#[derive(Debug, thiserror::Error)]
pub enum FileJwksError {
    /// The file could not be read.
    #[error("reading JWKS file `{path}`: {source}")]
    Io {
        /// Path we tried to read.
        path: String,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The file was not a valid JWKS document.
    #[error("parsing JWKS file `{path}`: {source}")]
    Parse {
        /// Path we tried to parse.
        path: String,
        /// Underlying failure.
        #[source]
        source: serde_json::Error,
    },

    /// The document parsed but holds no keys, so nothing could ever verify.
    #[error("JWKS file `{path}` contains no keys")]
    Empty {
        /// Path we read.
        path: String,
    },
}

/// Verifies JWTs against a key set loaded from disk.
#[derive(Debug, Clone)]
pub struct FileJwks {
    issuer: String,
    keys: JwkSet,
    algorithms: Vec<Algorithm>,
    leeway_secs: u64,
    scope_claim: String,
}

impl FileJwks {
    /// Read and parse a JWKS document.
    pub fn load(path: impl AsRef<Path>, issuer: &str) -> Result<Self, FileJwksError> {
        let path = path.as_ref();
        let display = path.display().to_string();

        let raw = std::fs::read_to_string(path).map_err(|source| FileJwksError::Io {
            path: display.clone(),
            source,
        })?;
        let keys: JwkSet = serde_json::from_str(&raw).map_err(|source| FileJwksError::Parse {
            path: display.clone(),
            source,
        })?;

        // An empty key set would fail every request at runtime with a message
        // about an unmatched `kid`, which reads like a client problem. Fail at
        // startup instead, where it reads like the config problem it is.
        if keys.keys.is_empty() {
            return Err(FileJwksError::Empty { path: display });
        }

        Ok(FileJwks {
            issuer: issuer.to_string(),
            keys,
            algorithms: vec![Algorithm::RS256, Algorithm::ES256],
            leeway_secs: 60,
            scope_claim: "scope".into(),
        })
    }

    /// Permit these signing algorithms, replacing the RS256/ES256 default.
    pub fn with_algorithms(mut self, algorithms: impl IntoIterator<Item = Algorithm>) -> Self {
        self.algorithms = algorithms.into_iter().collect();
        self
    }

    /// Clock-skew allowance for `exp` and `nbf`, in seconds. Defaults to 60.
    pub fn with_leeway_secs(mut self, leeway_secs: u64) -> Self {
        self.leeway_secs = leeway_secs;
        self
    }

    async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError> {
        let header = decode_header(token)
            .map_err(|err| TokenError::Invalid(format!("malformed token header: {err}")))?;

        // Pin the algorithm before touching any key: this is what stops an
        // attacker choosing it.
        if !self.algorithms.contains(&header.alg) {
            return Err(TokenError::Invalid(format!(
                "signing algorithm {:?} is not accepted",
                header.alg
            )));
        }

        let kid = header
            .kid
            .ok_or_else(|| TokenError::Invalid("token header has no `kid`".to_string()))?;

        let jwk = self.keys.find(&kid).ok_or_else(|| {
            // No refetch to attempt: the file is all we have.
            TokenError::Invalid(format!("no signing key matches kid `{kid}`"))
        })?;

        let key = DecodingKey::from_jwk(jwk)
            .map_err(|err| TokenError::Invalid(format!("unusable signing key: {err}")))?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.leeway = self.leeway_secs;
        validation.validate_exp = true;
        validation.validate_nbf = true;
        // Audience is enforced by JwtAuthenticator against the policy's
        // `audiences` list. Two places checking it means two sources of truth,
        // and the one that quietly stopped matching is the one nobody notices.
        validation.validate_aud = false;
        validation.required_spec_claims = ["exp", "iss"].iter().map(|s| s.to_string()).collect();

        let data = decode::<Value>(token, &key, &validation).map_err(|err| match err.kind() {
            ErrorKind::ExpiredSignature => TokenError::Expired,
            ErrorKind::InvalidIssuer => {
                TokenError::Invalid("the token was issued by an unexpected issuer".to_string())
            }
            ErrorKind::InvalidSignature => {
                TokenError::Invalid("the token signature is invalid".to_string())
            }
            other => TokenError::Invalid(format!("token validation failed: {other:?}")),
        })?;

        Ok(claims_to_token(data.claims, &self.scope_claim))
    }
}

impl TokenValidator for FileJwks {
    fn validate<'a>(&'a self, token: &'a str) -> ValidateFuture<'a> {
        Box::pin(self.verify(token))
    }
}

fn claims_to_token(claims: Value, scope_claim: &str) -> VerifiedToken {
    let mut token = VerifiedToken::new(audiences(claims.get("aud")));
    token.scopes = scopes(claims.get(scope_claim));
    token.subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string);
    token.client_id = claims
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    token.claims = claims;
    token
}

/// `aud` is a string or an array of strings (RFC 7519 §4.1.3).
fn audiences(aud: Option<&Value>) -> Vec<String> {
    match aud {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Scopes arrive space-delimited (`scope`) or as an array (`scp`).
fn scopes(scope: Option<&Value>) -> BTreeSet<String> {
    match scope {
        Some(Value::String(joined)) => joined.split_whitespace().map(str::to_string).collect(),
        Some(Value::Array(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => BTreeSet::new(),
    }
}
