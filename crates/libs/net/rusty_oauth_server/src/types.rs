//! Wire types: RFC 7591 client registration and records, and the RFC 6749
//! §5.1 token response.
//!
//! Metadata documents (RFC 8414 / RFC 9728) and error bodies are one-shot
//! `json!` values in [`crate::endpoints`], not a struct each.

use rusty_json::{Map, Value};

/// A field of the wrong JSON type, or a required one that is absent. The
/// text names the field, in the shape serde's errors had, since it goes
/// back to the registering client as `error_description`.
type FieldResult<T> = Result<T, String>;

fn object(value: &Value) -> FieldResult<&Map> {
    value
        .as_object()
        .ok_or_else(|| "invalid type: expected a JSON object".to_string())
}

/// An optional string: absent and `null` are `None`, any other type is an error.
fn opt_str(map: &Map, key: &str) -> FieldResult<Option<String>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("invalid type for `{key}`: expected a string")),
    }
}

fn opt_i64(map: &Map, key: &str) -> FieldResult<Option<i64>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("invalid type for `{key}`: expected an integer")),
    }
}

fn str_list(value: &Value, key: &str) -> FieldResult<Vec<String>> {
    let bad = || format!("invalid type for `{key}`: expected an array of strings");
    value
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(bad))
        .collect()
}

/// A list that must be present.
fn req_list(map: &Map, key: &str) -> FieldResult<Vec<String>> {
    match map.get(key) {
        Some(v) => str_list(v, key),
        None => Err(format!("missing field `{key}`")),
    }
}

/// A list that falls back to `default` when absent.
fn list_or(map: &Map, key: &str, default: fn() -> Vec<String>) -> FieldResult<Vec<String>> {
    match map.get(key) {
        Some(v) => str_list(v, key),
        None => Ok(default()),
    }
}

/// Every member of `map` not named in `known`: the fields kept in `extra`.
fn leftover(map: &Map, known: &[&str]) -> Map {
    map.iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn put(map: &mut Map, key: &str, value: impl Into<Value>) {
    map.insert(key.to_string(), value.into());
}

fn put_opt<T: Into<Value>>(map: &mut Map, key: &str, value: Option<T>) {
    if let Some(v) = value {
        put(map, key, v);
    }
}

fn strings(list: &[String]) -> Value {
    Value::Array(list.iter().map(|s| Value::from(s.as_str())).collect())
}

fn default_grant_types() -> Vec<String> {
    vec![
        "authorization_code".to_string(),
        "refresh_token".to_string(),
    ]
}

fn default_response_types() -> Vec<String> {
    vec!["code".to_string()]
}

/// RFC 7591 §2 dynamic client registration request body.
///
/// Fields this server never inspects (`jwks_uri`, `software_id`,
/// `contacts`, ...) are preserved via `extra` and echoed back verbatim in
/// the registration response — RFC 7591 §3.2.1 says the server "MUST
/// include" every registered metadata field in the response, so round-
/// tripping unknown fields through `extra` gets that for free instead of
/// hand-modeling every optional field.
#[derive(Debug, Clone)]
pub struct ClientMetadata {
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub scope: Option<String>,
    pub client_name: Option<String>,
    pub extra: Map,
}

impl ClientMetadata {
    const KNOWN: [&'static str; 6] = [
        "redirect_uris",
        "token_endpoint_auth_method",
        "grant_types",
        "response_types",
        "scope",
        "client_name",
    ];

    /// Read a registration request body.
    ///
    /// `redirect_uris` is required; `grant_types` and `response_types` default
    /// when absent; every other member lands in `extra`.
    ///
    /// # Errors
    /// Names the first member of the wrong type, or the missing one.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let map = object(value)?;
        Ok(Self {
            redirect_uris: req_list(map, "redirect_uris")?,
            token_endpoint_auth_method: opt_str(map, "token_endpoint_auth_method")?,
            grant_types: list_or(map, "grant_types", default_grant_types)?,
            response_types: list_or(map, "response_types", default_response_types)?,
            scope: opt_str(map, "scope")?,
            client_name: opt_str(map, "client_name")?,
            extra: leftover(map, &Self::KNOWN),
        })
    }

    /// Parse a registration request body.
    ///
    /// # Errors
    /// If the body is not JSON, or [`Self::from_value`] refuses it.
    pub fn from_slice(body: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(body).map_err(|e| e.to_string())?;
        let value = Value::parse(text).map_err(|e| e.to_string())?;
        Self::from_value(&value)
    }
}

/// RFC 7591 full client information record: what's persisted and what
/// `/register` returns. `client_secret`/`client_secret_expires_at` are
/// always `None` — see [`crate::Provider::register_client`]'s doc for why.
#[derive(Debug, Clone)]
pub struct ClientInformation {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub client_id_issued_at: Option<i64>,
    pub client_secret_expires_at: Option<i64>,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub scope: Option<String>,
    pub client_name: Option<String>,
    pub extra: Map,
}

impl ClientInformation {
    const KNOWN: [&'static str; 10] = [
        "client_id",
        "client_secret",
        "client_id_issued_at",
        "client_secret_expires_at",
        "redirect_uris",
        "token_endpoint_auth_method",
        "grant_types",
        "response_types",
        "scope",
        "client_name",
    ];

    /// Read a stored client record. Unlike a registration request, the
    /// redirect, auth-method and grant/response fields are all required.
    ///
    /// # Errors
    /// Names the first member of the wrong type, or the missing one.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let map = object(value)?;
        let token_endpoint_auth_method = opt_str(map, "token_endpoint_auth_method")?
            .ok_or("missing field `token_endpoint_auth_method`")?;
        Ok(Self {
            client_id: opt_str(map, "client_id")?,
            client_secret: opt_str(map, "client_secret")?,
            client_id_issued_at: opt_i64(map, "client_id_issued_at")?,
            client_secret_expires_at: opt_i64(map, "client_secret_expires_at")?,
            redirect_uris: req_list(map, "redirect_uris")?,
            token_endpoint_auth_method,
            grant_types: req_list(map, "grant_types")?,
            response_types: req_list(map, "response_types")?,
            scope: opt_str(map, "scope")?,
            client_name: opt_str(map, "client_name")?,
            extra: leftover(map, &Self::KNOWN),
        })
    }

    /// The record as stored and as `/register` returns it: unset optionals
    /// are omitted, and `extra` is spliced in at the top level. A member of
    /// `extra` never overrides a modelled field.
    pub fn to_value(&self) -> Value {
        let mut map = self.extra.clone();
        put_opt(&mut map, "client_id", self.client_id.as_deref());
        put_opt(&mut map, "client_secret", self.client_secret.as_deref());
        put_opt(&mut map, "client_id_issued_at", self.client_id_issued_at);
        put_opt(
            &mut map,
            "client_secret_expires_at",
            self.client_secret_expires_at,
        );
        put(&mut map, "redirect_uris", strings(&self.redirect_uris));
        put(
            &mut map,
            "token_endpoint_auth_method",
            self.token_endpoint_auth_method.as_str(),
        );
        put(&mut map, "grant_types", strings(&self.grant_types));
        put(&mut map, "response_types", strings(&self.response_types));
        put_opt(&mut map, "scope", self.scope.as_deref());
        put_opt(&mut map, "client_name", self.client_name.as_deref());
        Value::Object(map)
    }
}

/// Why a redirect_uri was rejected — RFC 6749 §3.1.2 redirect_uri pinning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectUriError {
    NotRegistered,
    AmbiguousWithoutOne,
}

impl ClientInformation {
    /// Pin a requested redirect_uri against this client's registered set
    /// (RFC 6749 §3.1.2). Mirrors the reference's
    /// `OAuthClientMetadata.validate_redirect_uri`: an explicit request must
    /// be one of the registered URIs; an omitted one is only unambiguous
    /// when exactly one is registered.
    pub fn validate_redirect_uri(
        &self,
        requested: Option<&str>,
    ) -> Result<String, RedirectUriError> {
        match requested {
            Some(uri) => {
                if self.redirect_uris.iter().any(|u| u == uri) {
                    Ok(uri.to_string())
                } else {
                    Err(RedirectUriError::NotRegistered)
                }
            }
            None => match self.redirect_uris.as_slice() {
                [only] => Ok(only.clone()),
                _ => Err(RedirectUriError::AmbiguousWithoutOne),
            },
        }
    }

    /// Validate a requested scope string against this client's registered
    /// `scope`. Mirrors the reference's `validate_scope`: `None` requested
    /// is always fine; otherwise every requested scope must be one the
    /// client registered with (an unset `self.scope` means the client
    /// registered with no scopes at all, so any non-empty request fails).
    pub fn validate_scope(&self, requested: Option<&str>) -> Result<Option<Vec<String>>, String> {
        let Some(requested) = requested else {
            return Ok(None);
        };
        let requested_scopes: Vec<String> = requested.split(' ').map(str::to_string).collect();
        let allowed: Vec<&str> = self
            .scope
            .as_deref()
            .map(|s| s.split(' ').collect())
            .unwrap_or_default();
        for scope in &requested_scopes {
            if !allowed.contains(&scope.as_str()) {
                return Err(format!("Client was not registered with scope {scope}"));
            }
        }
        Ok(Some(requested_scopes))
    }
}

/// RFC 6749 §5.1 access token response.
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
    pub refresh_token: Option<String>,
}

impl TokenResponse {
    /// The JSON body; unset optionals are omitted.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        put(&mut map, "access_token", self.access_token.as_str());
        put(&mut map, "token_type", self.token_type);
        put_opt(&mut map, "expires_in", self.expires_in);
        put_opt(&mut map, "scope", self.scope.as_deref());
        put_opt(&mut map, "refresh_token", self.refresh_token.as_deref());
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(redirect_uris: &[&str], scope: Option<&str>) -> ClientInformation {
        ClientInformation {
            client_id: Some("c1".to_string()),
            client_secret: None,
            client_id_issued_at: None,
            client_secret_expires_at: None,
            redirect_uris: redirect_uris.iter().map(|s| s.to_string()).collect(),
            token_endpoint_auth_method: "none".to_string(),
            grant_types: default_grant_types(),
            response_types: default_response_types(),
            scope: scope.map(str::to_string),
            client_name: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn redirect_uri_must_match_one_of_the_registered_uris_when_provided() {
        let c = client(&["https://a.example/cb", "https://b.example/cb"], None);
        assert_eq!(
            c.validate_redirect_uri(Some("https://a.example/cb")),
            Ok("https://a.example/cb".to_string())
        );
        assert_eq!(
            c.validate_redirect_uri(Some("https://evil.example/cb")),
            Err(RedirectUriError::NotRegistered)
        );
    }

    #[test]
    fn an_omitted_redirect_uri_only_resolves_when_exactly_one_is_registered() {
        let single = client(&["https://a.example/cb"], None);
        assert_eq!(
            single.validate_redirect_uri(None),
            Ok("https://a.example/cb".to_string())
        );

        let multiple = client(&["https://a.example/cb", "https://b.example/cb"], None);
        assert_eq!(
            multiple.validate_redirect_uri(None),
            Err(RedirectUriError::AmbiguousWithoutOne)
        );
    }

    #[test]
    fn scope_validation_passes_through_none_and_rejects_unregistered_scopes() {
        let c = client(&["https://a.example/cb"], Some("read write"));
        assert_eq!(c.validate_scope(None), Ok(None));
        assert_eq!(
            c.validate_scope(Some("read")),
            Ok(Some(vec!["read".to_string()]))
        );
        assert!(c.validate_scope(Some("admin")).is_err());
    }

    #[test]
    fn a_client_registered_with_no_scope_rejects_any_requested_scope() {
        let c = client(&["https://a.example/cb"], None);
        assert!(c.validate_scope(Some("read")).is_err());
    }

    fn parse(text: &str) -> Result<ClientMetadata, String> {
        ClientMetadata::from_slice(text.as_bytes())
    }

    #[test]
    fn metadata_defaults_and_keeps_unknown_members() {
        let m = parse(r#"{"redirect_uris":["https://a/cb"],"software_id":"x","contacts":["a@b"]}"#)
            .unwrap();
        assert_eq!(m.grant_types, default_grant_types());
        assert_eq!(m.response_types, default_response_types());
        assert_eq!(m.extra.len(), 2);
        assert_eq!(
            m.extra.get("software_id").and_then(Value::as_str),
            Some("x")
        );
    }

    #[test]
    fn metadata_refuses_missing_or_mistyped_members() {
        assert!(parse("{}").unwrap_err().contains("redirect_uris"));
        assert!(parse(r#"{"redirect_uris":"x"}"#).is_err());
        assert!(parse(r#"{"redirect_uris":[1]}"#).is_err());
        assert!(parse(r#"{"redirect_uris":[],"scope":3}"#).is_err());
        assert!(parse("[]").is_err());
        assert!(parse("not json").is_err());
        assert!(ClientMetadata::from_slice(&[0xff]).is_err());
    }

    #[test]
    fn null_optionals_read_as_absent() {
        let m = parse(r#"{"redirect_uris":[],"scope":null,"client_name":null}"#).unwrap();
        assert_eq!((m.scope, m.client_name), (None, None));
    }

    #[test]
    fn information_round_trips_with_extra_and_omits_unset_optionals() {
        let mut c = client(&["https://a/cb"], Some("read"));
        c.client_id_issued_at = Some(7);
        c.extra.insert("software_id".to_string(), Value::from("x"));
        let v = c.to_value();
        assert!(v.get("client_secret").is_none() && v.get("client_name").is_none());
        assert_eq!(v.get("software_id").and_then(Value::as_str), Some("x"));
        let back = ClientInformation::from_value(&v).unwrap();
        assert_eq!(back.to_value(), v);
        assert_eq!(back.client_id_issued_at, Some(7));
    }

    #[test]
    fn a_stored_record_missing_a_required_member_is_refused() {
        let mut v = client(&["https://a/cb"], None).to_value();
        v.as_object_mut().unwrap().remove("grant_types");
        assert!(ClientInformation::from_value(&v).is_err());
        assert!(ClientInformation::from_value(&Value::from("x")).is_err());
    }

    #[test]
    fn token_response_omits_unset_optionals() {
        let t = TokenResponse {
            access_token: "a".into(),
            token_type: "Bearer",
            expires_in: Some(60),
            scope: None,
            refresh_token: None,
        };
        let v = t.to_value();
        assert_eq!(v.get("expires_in").and_then(Value::as_i64), Some(60));
        assert!(v.get("scope").is_none() && v.get("refresh_token").is_none());
    }
}
