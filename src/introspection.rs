//! Token Introspection (RFC 7662).

use crate::client::Client;
use crate::encoding::percent::form_urlencode;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::request::HttpRequest;

/// Builds an RFC 7662 §2.1 introspection request. `token_type_hint`
/// (`"access_token"` or `"refresh_token"`) is optional but helps the
/// server look up the token more efficiently.
pub fn introspection_request(
    introspection_endpoint: &str,
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
    HttpRequest::form_post(introspection_endpoint, body).with_basic_auth_if_applicable(client)
}

/// The RFC 7662 §2.2 introspection response. Only `active` is guaranteed
/// to be present; every other field is `Option` and only meaningful when
/// `active` is `true`.
#[derive(Debug, Clone)]
pub struct IntrospectionResponse {
    pub active: bool,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub token_type: Option<String>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub sub: Option<String>,
    pub aud: Option<String>,
    pub iss: Option<String>,
    pub jti: Option<String>,
    pub raw: Value,
}

/// Parses an introspection response body. Per RFC 7662 §2.2, a response
/// with `"active": false` is a normal, successful result (it just means
/// the token is expired/revoked/invalid) -- not an error.
pub fn parse_introspection_response(status: u16, body: &str) -> Result<IntrospectionResponse> {
    if status != 200 {
        return Err(Error::Protocol(format!(
            "introspection endpoint returned HTTP {status}"
        )));
    }
    let value = json::parse(body)?;
    let active = value
        .get("active")
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::Protocol("introspection response missing `active`".to_string()))?;

    // `aud` may be a single string or an array of strings per RFC 7662 /
    // JWT conventions; normalize to a single comma-joined string for the
    // common case.
    let aud = match value.get("aud") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(items)) => {
            let joined: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            if joined.is_empty() {
                None
            } else {
                Some(joined.join(","))
            }
        }
        _ => None,
    };

    Ok(IntrospectionResponse {
        active,
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_string),
        client_id: value
            .get("client_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        username: value
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string),
        token_type: value
            .get("token_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        exp: value.get("exp").and_then(Value::as_i64),
        iat: value.get("iat").and_then(Value::as_i64),
        nbf: value.get("nbf").and_then(Value::as_i64),
        sub: value.get("sub").and_then(Value::as_str).map(str::to_string),
        aud,
        iss: value.get("iss").and_then(Value::as_str).map(str::to_string),
        jti: value.get("jti").and_then(Value::as_str).map(str::to_string),
        raw: value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Client, ClientId, ClientSecret};

    #[test]
    fn builds_request_with_basic_auth() {
        let client = Client::confidential(ClientId::new("res-server"), ClientSecret::new("secret"));
        let req = introspection_request(
            "https://auth.example.com/introspect",
            &client,
            "mF_9.B5f-4.1JqM",
            Some("access_token"),
        );
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("token=mF_9.B5f-4.1JqM"));
        assert!(body.contains("token_type_hint=access_token"));
        assert!(req.headers.iter().any(|(k, _)| k == "Authorization"));
    }

    #[test]
    fn parses_active_response() {
        let json = r#"{
            "active": true,
            "scope": "read write",
            "client_id": "l238j323",
            "username": "jdoe",
            "exp": 1419356238
        }"#;
        let resp = parse_introspection_response(200, json).unwrap();
        assert!(resp.active);
        assert_eq!(resp.scope.as_deref(), Some("read write"));
        assert_eq!(resp.exp, Some(1419356238));
    }

    #[test]
    fn parses_inactive_response_as_ok() {
        let resp = parse_introspection_response(200, r#"{"active": false}"#).unwrap();
        assert!(!resp.active);
    }

    #[test]
    fn normalizes_array_audience() {
        let resp =
            parse_introspection_response(200, r#"{"active": true, "aud": ["a", "b"]}"#).unwrap();
        assert_eq!(resp.aud.as_deref(), Some("a,b"));
    }
}
