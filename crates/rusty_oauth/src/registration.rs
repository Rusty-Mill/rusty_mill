//! Dynamic Client Registration (RFC 7591).
//!
//! Lets a client register itself with an authorization server at runtime
//! instead of being provisioned out-of-band. Distinct from RFC 8414
//! discovery: this is a separate endpoint (`registration_endpoint`) that
//! *creates* a client rather than describing the server.
//!
//! The follow-up client configuration management protocol (RFC 7592) --
//! reading, updating, or deleting a registration via
//! `registration_client_uri` and `registration_access_token` -- is not
//! implemented here; this module only covers the initial registration
//! request/response.

use crate::error::{Error, OAuthErrorResponse, Result};
use crate::json::{self, Value};
use crate::request::{HttpRequest, Method};

/// The RFC 7591 §2 client metadata sent in a registration request. Every
/// field is optional at the wire level (the server applies its own
/// defaults for anything omitted), but most real deployments need at
/// least `redirect_uris`.
#[derive(Debug, Clone, Default)]
pub struct ClientMetadata {
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub scope: Option<String>,
    pub contacts: Vec<String>,
    pub tos_uri: Option<String>,
    pub policy_uri: Option<String>,
    pub jwks_uri: Option<String>,
    pub software_id: Option<String>,
    pub software_version: Option<String>,
    /// Any additional/non-standard metadata field the server supports
    /// (e.g. vendor extensions), sent verbatim alongside the standard
    /// fields above.
    pub extra: Vec<(String, Value)>,
}

fn string_array_value(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect())
}

impl ClientMetadata {
    fn to_json(&self) -> Value {
        let mut fields: Vec<(String, Value)> = Vec::new();

        if !self.redirect_uris.is_empty() {
            fields.push((
                "redirect_uris".to_string(),
                string_array_value(&self.redirect_uris),
            ));
        }
        if let Some(v) = &self.token_endpoint_auth_method {
            fields.push((
                "token_endpoint_auth_method".to_string(),
                Value::from(v.as_str()),
            ));
        }
        if !self.grant_types.is_empty() {
            fields.push((
                "grant_types".to_string(),
                string_array_value(&self.grant_types),
            ));
        }
        if !self.response_types.is_empty() {
            fields.push((
                "response_types".to_string(),
                string_array_value(&self.response_types),
            ));
        }
        if let Some(v) = &self.client_name {
            fields.push(("client_name".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.client_uri {
            fields.push(("client_uri".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.logo_uri {
            fields.push(("logo_uri".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.scope {
            fields.push(("scope".to_string(), Value::from(v.as_str())));
        }
        if !self.contacts.is_empty() {
            fields.push(("contacts".to_string(), string_array_value(&self.contacts)));
        }
        if let Some(v) = &self.tos_uri {
            fields.push(("tos_uri".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.policy_uri {
            fields.push(("policy_uri".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.jwks_uri {
            fields.push(("jwks_uri".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.software_id {
            fields.push(("software_id".to_string(), Value::from(v.as_str())));
        }
        if let Some(v) = &self.software_version {
            fields.push(("software_version".to_string(), Value::from(v.as_str())));
        }
        fields.extend(self.extra.iter().cloned());

        Value::Object(fields)
    }
}

/// Builds the RFC 7591 §3.1 registration request: a JSON POST of the
/// client metadata. `initial_access_token`, when the server requires one
/// to gate registration, is sent as `Authorization: Bearer <token>`.
pub fn registration_request(
    registration_endpoint: &str,
    metadata: &ClientMetadata,
    initial_access_token: Option<&str>,
) -> HttpRequest {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(token) = initial_access_token {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    HttpRequest {
        method: Method::Post,
        url: registration_endpoint.to_string(),
        headers,
        body: metadata.to_json().to_json().into_bytes(),
    }
}

/// The RFC 7591 §3.2.1 Client Information Response.
#[derive(Debug, Clone)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_id_issued_at: Option<i64>,
    /// Present whenever `client_secret` is; `Some(0)` means it never
    /// expires (RFC 7591 §3.2.1).
    pub client_secret_expires_at: Option<i64>,
    /// RFC 7592: present if the server supports the client configuration
    /// management protocol for this registration.
    pub registration_access_token: Option<String>,
    pub registration_client_uri: Option<String>,
    /// The complete parsed JSON response body, including the echoed
    /// client metadata fields.
    pub raw: Value,
}

/// Parses a registration response. Per RFC 7591 §3.2.1, success is
/// `201 Created`; this accepts any `2xx` to be lenient with servers that
/// return `200 OK`. Anything else is parsed as a §3.2.2 error body (which
/// has the same `error`/`error_description` shape as RFC 6749 §5.2, just
/// with a different set of `error` codes).
pub fn parse_registration_response(status: u16, body: &str) -> Result<ClientRegistrationResponse> {
    let value = json::parse(body)?;

    if !(200..300).contains(&status) {
        if let Some(err) = OAuthErrorResponse::from_json(&value) {
            return Err(err.into());
        }
        return Err(Error::Protocol(format!(
            "registration endpoint returned HTTP {status} with an unrecognized error body"
        )));
    }

    let client_id = value
        .get("client_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("registration response missing `client_id`".to_string()))?
        .to_string();

    Ok(ClientRegistrationResponse {
        client_id,
        client_secret: value
            .get("client_secret")
            .and_then(Value::as_str)
            .map(str::to_string),
        client_id_issued_at: value.get("client_id_issued_at").and_then(Value::as_i64),
        client_secret_expires_at: value
            .get("client_secret_expires_at")
            .and_then(Value::as_i64),
        registration_access_token: value
            .get("registration_access_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        registration_client_uri: value
            .get("registration_client_uri")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw: value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata() -> ClientMetadata {
        ClientMetadata {
            redirect_uris: vec!["https://client.example.org/callback".to_string()],
            token_endpoint_auth_method: Some("client_secret_basic".to_string()),
            grant_types: vec![
                "authorization_code".to_string(),
                "refresh_token".to_string(),
            ],
            response_types: vec!["code".to_string()],
            client_name: Some("My Example Client".to_string()),
            scope: Some("read write".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn builds_json_request_body() {
        let req = registration_request(
            "https://server.example.com/register",
            &sample_metadata(),
            None,
        );
        assert_eq!(req.method, Method::Post);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        assert!(req.headers.iter().all(|(k, _)| k != "Authorization"));

        let body = json::parse(&String::from_utf8(req.body).unwrap()).unwrap();
        assert_eq!(
            body.get("client_name").unwrap().as_str(),
            Some("My Example Client")
        );
        assert_eq!(
            body.get("redirect_uris").unwrap().as_array().unwrap().len(),
            1
        );
        assert_eq!(
            body.get("grant_types").unwrap().as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn includes_initial_access_token_header() {
        let req = registration_request(
            "https://server.example.com/register",
            &sample_metadata(),
            Some("reg-23410913-abewfq.123483"),
        );
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer reg-23410913-abewfq.123483"));
    }

    #[test]
    fn extra_fields_are_included_verbatim() {
        let mut metadata = ClientMetadata::default();
        metadata
            .extra
            .push(("custom_vendor_field".to_string(), Value::from("hello")));
        let req = registration_request("https://server.example.com/register", &metadata, None);
        let body = json::parse(&String::from_utf8(req.body).unwrap()).unwrap();
        assert_eq!(
            body.get("custom_vendor_field").unwrap().as_str(),
            Some("hello")
        );
    }

    #[test]
    fn parses_successful_registration_response() {
        let json = r#"{
            "client_id": "s6BhdRkqt3",
            "client_secret": "cf136dc3c1fc93f31185e5885805d",
            "client_id_issued_at": 2893256800,
            "client_secret_expires_at": 0,
            "redirect_uris": ["https://client.example.org/callback"],
            "grant_types": ["authorization_code"],
            "token_endpoint_auth_method": "client_secret_basic",
            "client_name": "My Example Client"
        }"#;
        let resp = parse_registration_response(201, json).unwrap();
        assert_eq!(resp.client_id, "s6BhdRkqt3");
        assert_eq!(
            resp.client_secret.as_deref(),
            Some("cf136dc3c1fc93f31185e5885805d")
        );
        assert_eq!(resp.client_secret_expires_at, Some(0));
    }

    #[test]
    fn parses_registration_error_response() {
        let err = parse_registration_response(
            400,
            r#"{"error":"invalid_redirect_uri","error_description":"one or more redirect_uris are invalid"}"#,
        )
        .unwrap_err();
        match err {
            Error::OAuth(e) => assert_eq!(e.error.as_str(), "invalid_redirect_uri"),
            other => panic!("expected OAuth error, got {other:?}"),
        }
    }

    #[test]
    fn parses_registration_management_fields_when_present() {
        let json = r#"{
            "client_id": "s6BhdRkqt3",
            "registration_access_token": "reg-boJ2AQ1uCE9ttq7hUJHHTQ",
            "registration_client_uri": "https://server.example.com/register/s6BhdRkqt3"
        }"#;
        let resp = parse_registration_response(200, json).unwrap();
        assert_eq!(
            resp.registration_access_token.as_deref(),
            Some("reg-boJ2AQ1uCE9ttq7hUJHHTQ")
        );
        assert_eq!(
            resp.registration_client_uri.as_deref(),
            Some("https://server.example.com/register/s6BhdRkqt3")
        );
    }
}
