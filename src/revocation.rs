//! Token Revocation (RFC 7009).

use crate::client::Client;
use crate::encoding::percent::form_urlencode;
use crate::error::{Error, OAuthErrorResponse, Result};
use crate::json;
use crate::request::HttpRequest;

/// Builds an RFC 7009 §2.1 revocation request.
pub fn revocation_request(
    revocation_endpoint: &str,
    client: &Client,
    token: &str,
    token_type_hint: Option<&str>,
) -> HttpRequest {
    let mut params = vec![("token", token)];
    if let Some(hint) = token_type_hint {
        params.push(("token_type_hint", hint));
    }

    let uses_body_auth = client.auth_method != crate::client::AuthMethod::ClientSecretBasic;
    let secret_str;
    if uses_body_auth {
        params.push(("client_id", client.client_id.as_str()));
        if client.auth_method == crate::client::AuthMethod::ClientSecretPost {
            if let Some(secret) = &client.client_secret {
                secret_str = secret.as_str().to_string();
                params.push(("client_secret", &secret_str));
            }
        }
    }

    let body = form_urlencode(params);
    HttpRequest::form_post(revocation_endpoint, body).with_basic_auth_if_applicable(client)
}

/// Validates an RFC 7009 §2.2 revocation response. A bare `200 OK` (empty
/// body) is success -- including when the token was already invalid or
/// unknown, per §2.2's explicit instruction that the server "responds
/// with HTTP status code 200" regardless. Only a non-2xx response with a
/// standard OAuth error body is treated as an error.
pub fn parse_revocation_response(status: u16, body: &str) -> Result<()> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    if !body.trim().is_empty() {
        if let Ok(value) = json::parse(body) {
            if let Some(err) = OAuthErrorResponse::from_json(&value) {
                return Err(err.into());
            }
        }
    }
    Err(Error::Protocol(format!(
        "revocation endpoint returned HTTP {status}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, ClientId, ClientSecret};

    #[test]
    fn builds_request() {
        let client = Client::confidential(ClientId::new("s6BhdRkqt3"), ClientSecret::new("secret"));
        let req = revocation_request(
            "https://auth.example.com/revoke",
            &client,
            "45ghiukldjahdnhzdauz",
            Some("refresh_token"),
        );
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("token=45ghiukldjahdnhzdauz"));
        assert!(body.contains("token_type_hint=refresh_token"));
    }

    #[test]
    fn success_on_200() {
        assert!(parse_revocation_response(200, "").is_ok());
    }

    #[test]
    fn error_on_400_with_body() {
        let err =
            parse_revocation_response(400, r#"{"error":"unsupported_token_type"}"#).unwrap_err();
        match err {
            Error::OAuth(e) => assert_eq!(e.error.as_str(), "unsupported_token_type"),
            other => panic!("expected OAuth error, got {other:?}"),
        }
    }
}
