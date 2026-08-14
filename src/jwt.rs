//! JSON Web Token (RFC 7519) and JSON Web Signature (RFC 7515) support.
//!
//! Implements the `HS256` algorithm natively (needed for `client_secret_jwt`
//! client authentication, RFC 7523 §2.2, and for verifying tokens signed
//! with a shared secret). `RS256` verification is implemented in
//! [`rsa`] for verifying OpenID Connect `id_token`s signed
//! by an authorization server's RSA key -- the algorithm essentially every
//! public OAuth/OIDC provider uses.
//!
//! This module intentionally exposes only `HS256`/`RS256`. `alg: "none"`
//! (RFC 7519 §6) is never accepted -- a JWT with an unsigned or unrecognized
//! algorithm is always rejected, closing off the classic "alg confusion"
//! forgery class.

pub mod rsa;

use crate::crypto::hmac::{constant_time_eq, hmac_sha256};
use crate::encoding::base64::{decode_url_safe, encode_url_safe_no_pad};
use crate::error::{Error, Result};
use crate::json::{self, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// A decoded, three-part JWT: header, claims (payload), and the raw
/// signing input/signature needed to verify it.
#[derive(Debug, Clone)]
pub struct DecodedJwt {
    pub header: Value,
    pub claims: Value,
    /// `base64url(header) + "." + base64url(payload)`, i.e. the bytes the
    /// signature was computed over.
    pub signing_input: String,
    pub signature: Vec<u8>,
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Splits and base64url/JSON-decodes a compact JWT (`header.payload.signature`)
/// without checking the signature. Use [`verify_hs256`] or
/// [`rsa::verify_rs256`] when the token's source isn't already trusted.
pub fn decode_unverified(token: &str) -> Result<DecodedJwt> {
    let mut parts = token.split('.');
    let header_b64 = parts.next().ok_or_else(|| malformed("missing header"))?;
    let payload_b64 = parts.next().ok_or_else(|| malformed("missing payload"))?;
    let signature_b64 = parts.next().ok_or_else(|| malformed("missing signature"))?;
    if parts.next().is_some() {
        return Err(malformed("too many segments"));
    }

    let header_json = decode_url_safe(header_b64)?;
    let payload_json = decode_url_safe(payload_b64)?;
    let signature = decode_url_safe(signature_b64)?;

    let header = json::parse(
        std::str::from_utf8(&header_json).map_err(|_| malformed("header is not valid UTF-8"))?,
    )?;
    let claims = json::parse(
        std::str::from_utf8(&payload_json).map_err(|_| malformed("payload is not valid UTF-8"))?,
    )?;

    Ok(DecodedJwt {
        header,
        claims,
        signing_input: format!("{header_b64}.{payload_b64}"),
        signature,
    })
}

fn malformed(msg: &str) -> Error {
    Error::Validation(format!("malformed JWT: {msg}"))
}

/// Encodes and signs a JWT using `HS256` (RFC 7518 §3.2). `extra_header`
/// entries (e.g. `kid`) are merged into the `{"alg":"HS256","typ":"JWT"}`
/// header.
pub fn encode_hs256(claims: &Value, secret: &[u8], extra_header: &[(&str, &str)]) -> String {
    let mut header_fields = vec![
        ("alg".to_string(), Value::from("HS256")),
        ("typ".to_string(), Value::from("JWT")),
    ];
    for (k, v) in extra_header {
        header_fields.push((k.to_string(), Value::from(*v)));
    }
    let header = Value::Object(header_fields);

    let header_b64 = encode_url_safe_no_pad(header.to_json().as_bytes());
    let payload_b64 = encode_url_safe_no_pad(claims.to_json().as_bytes());
    let signing_input = format!("{header_b64}.{payload_b64}");

    let signature = hmac_sha256(secret, signing_input.as_bytes());
    let signature_b64 = encode_url_safe_no_pad(&signature);

    format!("{signing_input}.{signature_b64}")
}

/// Verifies an `HS256`-signed JWT and returns its claims.
pub fn verify_hs256(token: &str, secret: &[u8]) -> Result<Value> {
    let decoded = decode_unverified(token)?;

    let alg = decoded
        .header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("header missing `alg`"))?;
    if alg != "HS256" {
        return Err(Error::Validation(format!(
            "expected alg HS256, got {alg} (alg confusion is not permitted: the caller must \
             pin the expected algorithm and this function will not honor whatever the token claims)"
        )));
    }

    let expected = hmac_sha256(secret, decoded.signing_input.as_bytes());
    if !constant_time_eq(&expected, &decoded.signature) {
        return Err(Error::Validation(
            "JWT signature verification failed".to_string(),
        ));
    }

    Ok(decoded.claims)
}

/// Claim validation options for [`validate_claims`] (RFC 7519 §4.1).
#[derive(Debug, Clone)]
pub struct Validation {
    /// Clock-skew tolerance applied to `exp`/`nbf` checks.
    pub leeway_seconds: i64,
    /// Required exact match against the `iss` claim, if set.
    pub expected_issuer: Option<String>,
    /// Required membership in the `aud` claim (which may be a single
    /// string or an array of strings, RFC 7519 §4.1.3), if set.
    pub expected_audience: Option<String>,
    /// Reject tokens with no `exp` claim (recommended: `true`).
    pub require_exp: bool,
}

impl Default for Validation {
    fn default() -> Self {
        Validation {
            leeway_seconds: 60,
            expected_issuer: None,
            expected_audience: None,
            require_exp: true,
        }
    }
}

/// Validates the registered time-based and identity claims of RFC 7519
/// §4.1: `exp`, `nbf`, `iss`, `aud`. Call this after verifying the
/// signature ([`verify_hs256`] / [`rsa::verify_rs256`]) -- it does not
/// check the signature itself.
pub fn validate_claims(claims: &Value, options: &Validation) -> Result<()> {
    let now = now_unix();

    match claims.get("exp").and_then(Value::as_i64) {
        Some(exp) => {
            if now - options.leeway_seconds >= exp {
                return Err(Error::Validation("token has expired (`exp`)".to_string()));
            }
        }
        None if options.require_exp => {
            return Err(Error::Validation(
                "token is missing required `exp` claim".to_string(),
            ));
        }
        None => {}
    }

    if let Some(nbf) = claims.get("nbf").and_then(Value::as_i64) {
        if now + options.leeway_seconds < nbf {
            return Err(Error::Validation(
                "token is not yet valid (`nbf`)".to_string(),
            ));
        }
    }

    if let Some(expected_iss) = &options.expected_issuer {
        let iss = claims.get("iss").and_then(Value::as_str);
        if iss != Some(expected_iss.as_str()) {
            return Err(Error::Validation(format!(
                "unexpected issuer: expected `{expected_iss}`, got `{iss:?}`"
            )));
        }
    }

    if let Some(expected_aud) = &options.expected_audience {
        let matches = match claims.get("aud") {
            Some(Value::String(s)) => s == expected_aud,
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .any(|a| a == expected_aud),
            _ => false,
        };
        if !matches {
            return Err(Error::Validation(format!(
                "token audience does not include expected value `{expected_aud}`"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_sign_and_verify() {
        let claims = Value::object([
            ("sub".to_string(), Value::from("user-123")),
            ("iss".to_string(), Value::from("https://issuer.example.com")),
        ]);
        let token = encode_hs256(&claims, b"my-secret-key", &[]);
        let verified = verify_hs256(&token, b"my-secret-key").unwrap();
        assert_eq!(verified.get("sub").unwrap().as_str(), Some("user-123"));
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let claims = Value::object([("sub".to_string(), Value::from("user"))]);
        let token = encode_hs256(&claims, b"secret-a", &[]);
        assert!(verify_hs256(&token, b"secret-b").is_err());
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let claims = Value::object([
            ("sub".to_string(), Value::from("user")),
            ("admin".to_string(), Value::from(false)),
        ]);
        let token = encode_hs256(&claims, b"secret", &[]);
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged_claims = Value::object([
            ("sub".to_string(), Value::from("user")),
            ("admin".to_string(), Value::from(true)),
        ]);
        let forged_payload = encode_url_safe_no_pad(forged_claims.to_json().as_bytes());
        parts[1] = &forged_payload;
        let forged_token = parts.join(".");
        assert!(verify_hs256(&forged_token, b"secret").is_err());
    }

    #[test]
    fn rejects_alg_none() {
        // Hand-craft a token claiming alg "none" with an empty signature,
        // the classic JWT forgery. It must never verify successfully.
        let header = Value::object([
            ("alg".to_string(), Value::from("none")),
            ("typ".to_string(), Value::from("JWT")),
        ]);
        let claims = Value::object([("sub".to_string(), Value::from("attacker"))]);
        let header_b64 = encode_url_safe_no_pad(header.to_json().as_bytes());
        let payload_b64 = encode_url_safe_no_pad(claims.to_json().as_bytes());
        let token = format!("{header_b64}.{payload_b64}.");
        assert!(verify_hs256(&token, b"any-secret").is_err());
    }

    #[test]
    fn validate_claims_expired() {
        let claims = Value::object([("exp".to_string(), Value::from(now_unix() - 3600))]);
        let opts = Validation {
            leeway_seconds: 0,
            ..Default::default()
        };
        assert!(validate_claims(&claims, &opts).is_err());
    }

    #[test]
    fn validate_claims_within_leeway() {
        let claims = Value::object([("exp".to_string(), Value::from(now_unix() - 5))]);
        let opts = Validation {
            leeway_seconds: 60,
            ..Default::default()
        };
        assert!(validate_claims(&claims, &opts).is_ok());
    }

    #[test]
    fn validate_claims_issuer_and_audience() {
        let claims = Value::object([
            ("exp".to_string(), Value::from(now_unix() + 3600)),
            ("iss".to_string(), Value::from("https://issuer.example.com")),
            (
                "aud".to_string(),
                Value::Array(vec![Value::from("client-a"), Value::from("client-b")]),
            ),
        ]);
        let opts = Validation {
            expected_issuer: Some("https://issuer.example.com".to_string()),
            expected_audience: Some("client-b".to_string()),
            ..Default::default()
        };
        assert!(validate_claims(&claims, &opts).is_ok());

        let bad_opts = Validation {
            expected_audience: Some("client-c".to_string()),
            ..Default::default()
        };
        assert!(validate_claims(&claims, &bad_opts).is_err());
    }

    #[test]
    fn missing_exp_rejected_by_default() {
        let claims = Value::object([("sub".to_string(), Value::from("user"))]);
        assert!(validate_claims(&claims, &Validation::default()).is_err());
    }
}
