//! DPoP -- Demonstrating Proof of Possession (RFC 9449).
//!
//! Sender-constrains OAuth access (and refresh) tokens to a public/private
//! key pair the client holds, without needing mutual TLS: every request
//! carries a fresh, signed "proof" JWT showing the client controls the
//! private key the token was issued to, alongside the token itself. Proofs
//! are always `ES256`-signed (RFC 9449 §4.2 permits other JWS algorithms,
//! but `ES256` is what virtually every real implementation uses, and it's
//! what this crate implements), via [`crate::jwt::es256`].

use crate::crypto::sha256::sha256;
use crate::encoding::base64::encode_url_safe_no_pad;
use crate::error::{Error, ErrorCode, Result};
use crate::json::Value;
use crate::jwt::es256::{sign_p256_sha256, EcPrivateKey};
use crate::rand::random_bytes;

/// Builds an RFC 9449 §4.2 DPoP proof JWT for one HTTP request.
///
/// - `http_method`/`http_uri` become the proof's `htm`/`htu` claims;
///   `htu` is normalized by dropping any query string or fragment, per
///   §4.2's requirement that it carry neither.
/// - `access_token`, when set, binds the proof to that specific token via
///   the `ath` claim (§4.3) -- required on resource-server requests, but
///   omitted when proving possession at the token endpoint itself (no
///   access token exists yet).
/// - `nonce`, when set, echoes a server-issued `DPoP-Nonce` value (§8);
///   required on retry after a `use_dpop_nonce` error (see
///   [`is_use_dpop_nonce_error`]).
pub fn build_proof(
    private_key: &EcPrivateKey,
    http_method: &str,
    http_uri: &str,
    access_token: Option<&str>,
    nonce: Option<&str>,
) -> Result<String> {
    let public_key = private_key.public_key();
    let (x, y) = public_key.to_affine_coordinates();
    let jwk = Value::object([
        ("kty".to_string(), Value::from("EC")),
        ("crv".to_string(), Value::from("P-256")),
        ("x".to_string(), Value::from(encode_url_safe_no_pad(&x))),
        ("y".to_string(), Value::from(encode_url_safe_no_pad(&y))),
    ]);
    let header = Value::object([
        ("typ".to_string(), Value::from("dpop+jwt")),
        ("alg".to_string(), Value::from("ES256")),
        ("jwk".to_string(), jwk),
    ]);

    let jti = encode_url_safe_no_pad(&random_bytes(16)?);
    let mut claims_fields = vec![
        ("jti".to_string(), Value::from(jti)),
        ("htm".to_string(), Value::from(http_method)),
        (
            "htu".to_string(),
            Value::from(strip_query_and_fragment(http_uri)),
        ),
        ("iat".to_string(), Value::from(crate::jwt::now_unix())),
    ];
    if let Some(token) = access_token {
        claims_fields.push(("ath".to_string(), Value::from(compute_ath(token))));
    }
    if let Some(nonce) = nonce {
        claims_fields.push(("nonce".to_string(), Value::from(nonce)));
    }
    let claims = Value::Object(claims_fields);

    let header_b64 = encode_url_safe_no_pad(header.to_json().as_bytes());
    let payload_b64 = encode_url_safe_no_pad(claims.to_json().as_bytes());
    let signing_input = format!("{header_b64}.{payload_b64}");

    let signature = sign_p256_sha256(signing_input.as_bytes(), private_key);
    let signature_b64 = encode_url_safe_no_pad(&signature);

    Ok(format!("{signing_input}.{signature_b64}"))
}

/// Computes the `ath` claim value (RFC 9449 §4.3):
/// `base64url(SHA-256(access_token))`.
pub fn compute_ath(access_token: &str) -> String {
    encode_url_safe_no_pad(&sha256(access_token.as_bytes()))
}

fn strip_query_and_fragment(uri: &str) -> String {
    let without_fragment = uri.split('#').next().unwrap_or(uri);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .to_string()
}

/// Builds the `Authorization: DPoP <access_token>` header value (RFC 9449
/// §7.1) -- used instead of `Bearer` for a DPoP-bound token. Send the
/// matching proof (see [`build_proof`]) as a `DPoP` header alongside it.
pub fn authorization_header(access_token: &str) -> String {
    format!("DPoP {access_token}")
}

/// RFC 9449 §8: whether an OAuth error response is asking for the request
/// to be retried with a fresh proof carrying a `nonce` claim (extracted
/// from the response's `DPoP-Nonce` header). On `true`, rebuild the proof
/// via [`build_proof`] with that nonce and retry once.
pub fn is_use_dpop_nonce_error(err: &Error) -> bool {
    matches!(err, Error::OAuth(e) if e.error == ErrorCode::UseDpopNonce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::es256::verify_es256;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // The same real P-256 key pair used throughout crypto::ecc's and
    // jwt::es256's tests (openssl ecparam -genkey).
    fn private_key() -> EcPrivateKey {
        EcPrivateKey::from_bytes(&hex(
            "67718fec6a6b21b412a5c5306286f1ee30e32498fd6c61b66f57d0ad1d7c0738",
        ))
        .unwrap()
    }

    /// Reconstructs the public key embedded in a proof's `jwk` header and
    /// verifies the proof against it -- this both checks the signature is
    /// valid (transitively exercising the already openssl-cross-checked
    /// `verify_es256`/`sign_p256_sha256`) and that the embedded JWK
    /// actually matches the signing key, which is the whole point of
    /// embedding it.
    fn verify_proof(token: &str) -> Value {
        let decoded = crate::jwt::decode_unverified(token).unwrap();
        let jwk = decoded.header.get("jwk").unwrap();
        let x = jwk.get("x").unwrap().as_str().unwrap();
        let y = jwk.get("y").unwrap().as_str().unwrap();
        let key = crate::jwt::es256::EcPublicKey::from_jwk_base64url(x, y).unwrap();
        verify_es256(token, &key).unwrap()
    }

    #[test]
    fn builds_valid_proof_for_token_request() {
        let proof = build_proof(
            &private_key(),
            "POST",
            "https://server.example.com/token",
            None,
            None,
        )
        .unwrap();
        let claims = verify_proof(&proof);
        assert_eq!(claims.get("htm").unwrap().as_str(), Some("POST"));
        assert_eq!(
            claims.get("htu").unwrap().as_str(),
            Some("https://server.example.com/token")
        );
        assert!(claims.get("iat").unwrap().as_i64().is_some());
        assert!(claims.get("ath").is_none());
        assert!(claims.get("nonce").is_none());
    }

    #[test]
    fn strips_query_and_fragment_from_htu() {
        let proof = build_proof(
            &private_key(),
            "GET",
            "https://api.example.com/resource?foo=bar#frag",
            None,
            None,
        )
        .unwrap();
        let claims = verify_proof(&proof);
        assert_eq!(
            claims.get("htu").unwrap().as_str(),
            Some("https://api.example.com/resource")
        );
    }

    #[test]
    fn resource_request_proof_includes_ath() {
        let access_token = "Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU";
        let proof = build_proof(
            &private_key(),
            "GET",
            "https://resource.example.org/protectedresource",
            Some(access_token),
            None,
        )
        .unwrap();
        let claims = verify_proof(&proof);
        assert_eq!(
            claims.get("ath").unwrap().as_str(),
            Some(compute_ath(access_token).as_str())
        );
    }

    #[test]
    fn ath_matches_independently_computed_sha256() {
        // Cross-check against this crate's own (separately, RFC-vector
        // tested) SHA-256 and base64url implementations composed by hand,
        // rather than trusting compute_ath's internals circularly.
        let token = "example-access-token";
        let expected = crate::encoding::base64::encode_url_safe_no_pad(
            &crate::crypto::sha256::sha256(token.as_bytes()),
        );
        assert_eq!(compute_ath(token), expected);
    }

    #[test]
    fn each_proof_gets_a_fresh_jti() {
        let a = build_proof(&private_key(), "GET", "https://example.com", None, None).unwrap();
        let b = build_proof(&private_key(), "GET", "https://example.com", None, None).unwrap();
        let claims_a = verify_proof(&a);
        let claims_b = verify_proof(&b);
        assert_ne!(claims_a.get("jti"), claims_b.get("jti"));
    }

    #[test]
    fn authorization_header_uses_dpop_scheme() {
        assert_eq!(authorization_header("tok"), "DPoP tok");
    }

    #[test]
    fn nonce_retry_flow() {
        // Simulate a resource server's use_dpop_nonce error, per RFC 9449 §8.
        let error_body = r#"{"error":"use_dpop_nonce","error_description":"Authorization server requires nonce in DPoP proof"}"#;
        let err = crate::token::parse_token_response(400, error_body).unwrap_err();
        assert!(is_use_dpop_nonce_error(&err));

        // The retry: rebuild the proof with the nonce from the response's
        // (simulated here) DPoP-Nonce header.
        let server_nonce = "eyJ7S_zG.eyJH0-Z1r";
        let retried_proof = build_proof(
            &private_key(),
            "POST",
            "https://server.example.com/token",
            None,
            Some(server_nonce),
        )
        .unwrap();
        let claims = verify_proof(&retried_proof);
        assert_eq!(claims.get("nonce").unwrap().as_str(), Some(server_nonce));
    }

    #[test]
    fn non_dpop_errors_do_not_trigger_retry() {
        let err =
            crate::token::parse_token_response(400, r#"{"error":"invalid_grant"}"#).unwrap_err();
        assert!(!is_use_dpop_nonce_error(&err));
    }
}
