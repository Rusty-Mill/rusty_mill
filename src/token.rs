//! The token endpoint: grant requests (RFC 6749 §4) and the token
//! response (RFC 6749 §5).
//!
//! Only the grants retained by OAuth 2.1 are implemented: authorization
//! code (with mandatory PKCE support), client credentials, refresh token,
//! and the device authorization grant (RFC 8628). The Resource Owner
//! Password Credentials grant is intentionally omitted -- OAuth 2.1 §2.4
//! removes it because it requires the client to handle the resource
//! owner's raw credentials, defeating the purpose of delegated auth.

use crate::client::Client;
use crate::encoding::percent::form_urlencode;
use crate::error::{Error, OAuthErrorResponse, Result};
use crate::json::{self, Value};
use crate::request::HttpRequest;

/// Builds the RFC 6749 §4.1.3 token request for the authorization code
/// grant. `code_verifier` should be `Some(&pkce.code_verifier)` whenever
/// the authorization request included a PKCE challenge (RFC 7636 §4.5) --
/// which, under this crate's defaults, is always.
pub fn authorization_code_request(
    token_endpoint: &str,
    client: &Client,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
) -> Result<HttpRequest> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];

    let uses_body_auth = client.auth_method != crate::client::AuthMethod::ClientSecretBasic;
    if uses_body_auth {
        params.push(("client_id", client.client_id.as_str()));
    }
    let secret_str;
    if uses_body_auth && client.auth_method == crate::client::AuthMethod::ClientSecretPost {
        if let Some(secret) = &client.client_secret {
            secret_str = secret.as_str().to_string();
            params.push(("client_secret", &secret_str));
        }
    }
    let assertion;
    if let Some((assertion_type, assertion_value)) =
        client.build_client_assertion(token_endpoint)?
    {
        assertion = assertion_value;
        params.push(("client_assertion_type", assertion_type));
        params.push(("client_assertion", &assertion));
    }
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier));
    }

    let body = form_urlencode(params);
    Ok(HttpRequest::form_post(token_endpoint, body).with_basic_auth_if_applicable(client))
}

/// Builds the RFC 6749 §6 refresh token request.
pub fn refresh_token_request(
    token_endpoint: &str,
    client: &Client,
    refresh_token: &str,
    scope: Option<&str>,
) -> Result<HttpRequest> {
    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];

    let uses_body_auth = client.auth_method != crate::client::AuthMethod::ClientSecretBasic;
    if uses_body_auth {
        params.push(("client_id", client.client_id.as_str()));
    }
    let secret_str;
    if uses_body_auth && client.auth_method == crate::client::AuthMethod::ClientSecretPost {
        if let Some(secret) = &client.client_secret {
            secret_str = secret.as_str().to_string();
            params.push(("client_secret", &secret_str));
        }
    }
    let assertion;
    if let Some((assertion_type, assertion_value)) =
        client.build_client_assertion(token_endpoint)?
    {
        assertion = assertion_value;
        params.push(("client_assertion_type", assertion_type));
        params.push(("client_assertion", &assertion));
    }
    if let Some(scope) = scope {
        params.push(("scope", scope));
    }

    let body = form_urlencode(params);
    Ok(HttpRequest::form_post(token_endpoint, body).with_basic_auth_if_applicable(client))
}

/// Builds the RFC 6749 §4.4.2 client credentials request. Requires a
/// confidential client (RFC 6749 §4.4.1).
pub fn client_credentials_request(
    token_endpoint: &str,
    client: &Client,
    scope: Option<&str>,
) -> Result<HttpRequest> {
    let mut params = vec![("grant_type", "client_credentials")];

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
    let assertion;
    if let Some((assertion_type, assertion_value)) =
        client.build_client_assertion(token_endpoint)?
    {
        assertion = assertion_value;
        params.push(("client_assertion_type", assertion_type));
        params.push(("client_assertion", &assertion));
    }
    if let Some(scope) = scope {
        params.push(("scope", scope));
    }

    let body = form_urlencode(params);
    Ok(HttpRequest::form_post(token_endpoint, body).with_basic_auth_if_applicable(client))
}

/// Builds the RFC 8628 §3.4 device access token polling request.
pub fn device_code_request(
    token_endpoint: &str,
    client: &Client,
    device_code: &str,
) -> Result<HttpRequest> {
    let mut params = vec![
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", device_code),
    ];

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
    let assertion;
    if let Some((assertion_type, assertion_value)) =
        client.build_client_assertion(token_endpoint)?
    {
        assertion = assertion_value;
        params.push(("client_assertion_type", assertion_type));
        params.push(("client_assertion", &assertion));
    }

    let body = form_urlencode(params);
    Ok(HttpRequest::form_post(token_endpoint, body).with_basic_auth_if_applicable(client))
}

/// Builds an RFC 7523 §2.1 JWT bearer assertion grant request, e.g. for
/// service-account-style authentication where the "assertion" is a JWT
/// signed by the client (see [`crate::jwt`]).
pub fn jwt_bearer_request(
    token_endpoint: &str,
    assertion: &str,
    scope: Option<&str>,
) -> HttpRequest {
    let mut params = vec![
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", assertion),
    ];
    if let Some(scope) = scope {
        params.push(("scope", scope));
    }
    let body = form_urlencode(params);
    HttpRequest::form_post(token_endpoint, body)
}

/// A successful token response (RFC 6749 §5.1). Fields beyond the
/// standard set (e.g. OIDC's `id_token`) are preserved in `raw` for
/// callers that need them.
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    /// Present when the authorization server also implements OpenID
    /// Connect and the request included `scope=openid`.
    pub id_token: Option<String>,
    /// The complete parsed JSON response body, for accessing any
    /// non-standard fields.
    pub raw: Value,
}

/// Parses a token endpoint HTTP response. `status` is the HTTP status
/// code and `body` its (JSON) content. Per RFC 6749 §5.1/§5.2, a 200
/// response is a [`TokenResponse`]; anything else is treated as an error
/// body and returned as `Err(Error::OAuth(..))` when it parses as one.
pub fn parse_token_response(status: u16, body: &str) -> Result<TokenResponse> {
    let value = json::parse(body)?;

    if status != 200 {
        if let Some(err) = OAuthErrorResponse::from_json(&value) {
            return Err(err.into());
        }
        return Err(Error::Protocol(format!(
            "token endpoint returned HTTP {status} with an unrecognized error body"
        )));
    }

    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("token response missing `access_token`".to_string()))?
        .to_string();
    let token_type = value
        .get("token_type")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("token response missing `token_type`".to_string()))?
        .to_string();

    Ok(TokenResponse {
        access_token,
        token_type,
        expires_in: value.get("expires_in").and_then(Value::as_u64),
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_string),
        id_token: value
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw: value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{AuthMethod, Client, ClientId, ClientSecret};

    #[test]
    fn authorization_code_request_basic_auth() {
        let client = Client::confidential(ClientId::new("id"), ClientSecret::new("secret"));
        let req = authorization_code_request(
            "https://auth.example.com/token",
            &client,
            "auth-code",
            "https://app.example.com/cb",
            Some("verifier123"),
        )
        .unwrap();
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("code=auth-code"));
        assert!(body.contains("code_verifier=verifier123"));
        assert!(!body.contains("client_secret")); // sent via Basic header, not body
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v.starts_with("Basic ")));
    }

    #[test]
    fn authorization_code_request_public_client_no_secret() {
        let client = Client::public(ClientId::new("public-id"));
        let req = authorization_code_request(
            "https://auth.example.com/token",
            &client,
            "code",
            "https://app/cb",
            Some("v"),
        )
        .unwrap();
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("client_id=public-id"));
        assert!(req.headers.iter().all(|(k, _)| k != "Authorization"));
    }

    #[test]
    fn client_secret_post_puts_secret_in_body() {
        let client = Client::confidential(ClientId::new("id"), ClientSecret::new("s3cr3t"))
            .with_auth_method(AuthMethod::ClientSecretPost);
        let req =
            client_credentials_request("https://auth.example.com/token", &client, Some("read"))
                .unwrap();
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("client_id=id"));
        assert!(body.contains("client_secret=s3cr3t"));
        assert!(req.headers.iter().all(|(k, _)| k != "Authorization"));
    }

    #[test]
    fn parses_successful_token_response() {
        let json = r#"{
            "access_token": "2YotnFZFEjr1zCsicMWpAA",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "tGzv3JOkF0XG5Qx2TlKWIA"
        }"#;
        let resp = parse_token_response(200, json).unwrap();
        assert_eq!(resp.access_token, "2YotnFZFEjr1zCsicMWpAA");
        assert_eq!(resp.token_type, "Bearer");
        assert_eq!(resp.expires_in, Some(3600));
        assert_eq!(
            resp.refresh_token.as_deref(),
            Some("tGzv3JOkF0XG5Qx2TlKWIA")
        );
    }

    #[test]
    fn parses_error_token_response() {
        let json = r#"{"error":"invalid_grant","error_description":"code expired"}"#;
        let err = parse_token_response(400, json).unwrap_err();
        match err {
            Error::OAuth(e) => assert_eq!(e.error, crate::error::ErrorCode::InvalidGrant),
            other => panic!("expected OAuth error, got {other:?}"),
        }
    }

    #[test]
    fn device_code_request_has_correct_grant_type() {
        let client = Client::public(ClientId::new("device-app"));
        let req =
            device_code_request("https://auth.example.com/token", &client, "dev-code-abc").unwrap();
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
        assert!(body.contains("device_code=dev-code-abc"));
    }

    #[test]
    fn client_secret_jwt_attaches_generated_assertion() {
        let client = Client::confidential(ClientId::new("id"), ClientSecret::new("s3cr3t"))
            .with_auth_method(AuthMethod::ClientSecretJwt);
        let req =
            client_credentials_request("https://auth.example.com/token", &client, None).unwrap();
        let body = String::from_utf8(req.body).unwrap();

        assert!(body.contains(
            "client_assertion_type=urn%3Aietf%3Aparams%3Aoauth%3Aclient-assertion-type%3Ajwt-bearer"
        ));
        assert!(!body.contains("client_secret=")); // never sent in the clear alongside an assertion
        assert!(req.headers.iter().all(|(k, _)| k != "Authorization"));

        let pairs = crate::encoding::percent::form_urldecode(&body).unwrap();
        let assertion = pairs
            .iter()
            .find(|(k, _)| k == "client_assertion")
            .map(|(_, v)| v.clone())
            .expect("client_assertion present");
        let claims = crate::jwt::verify_hs256(&assertion, b"s3cr3t").unwrap();
        assert_eq!(claims.get("iss").unwrap().as_str(), Some("id"));
        assert_eq!(claims.get("sub").unwrap().as_str(), Some("id"));
        assert_eq!(
            claims.get("aud").unwrap().as_str(),
            Some("https://auth.example.com/token")
        );
    }

    #[test]
    fn client_secret_jwt_without_secret_errors() {
        let client =
            Client::public(ClientId::new("id")).with_auth_method(AuthMethod::ClientSecretJwt);
        let err = client_credentials_request("https://auth.example.com/token", &client, None)
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn private_key_jwt_uses_caller_supplied_assertion() {
        let client = Client::public(ClientId::new("id"))
            .with_auth_method(AuthMethod::PrivateKeyJwt)
            .with_client_assertion("pre-signed.jwt.value");
        let req =
            client_credentials_request("https://auth.example.com/token", &client, None).unwrap();
        let body = String::from_utf8(req.body).unwrap();
        assert!(body.contains("client_assertion=pre-signed.jwt.value"));
    }

    #[test]
    fn private_key_jwt_without_assertion_errors() {
        let client =
            Client::public(ClientId::new("id")).with_auth_method(AuthMethod::PrivateKeyJwt);
        let err = client_credentials_request("https://auth.example.com/token", &client, None)
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }
}
