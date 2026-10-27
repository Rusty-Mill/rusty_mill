//! The HTTP surface, as plain data: RFC 8414 and RFC 9728 metadata,
//! `/authorize`, `/consent`, `/token`, RFC 7591 `/register` and RFC 7009
//! `/revoke`, plus the `401` challenge a server's `/mcp` gate sends.
//!
//! [`Endpoints::handle`] takes a [`Request`] and returns a [`Response`], or
//! `None` when the path is not one of these, so a server falls through to
//! its own routes. It does no I/O beyond the [`Provider`]'s state file.
//!
//! # No client-secret check
//!
//! [`Provider::register_client`] gives every client
//! `token_endpoint_auth_method = "none"` and no secret, so `/token` and
//! `/revoke` reduce client authentication to "`client_id` names a
//! registered client". PKCE is what proves possession.
//!
//! # `/authorize` error routing
//!
//! The client and its `redirect_uri` are resolved first. Until both are
//! known an error is a JSON `400`; after that every error redirects back to
//! the client (RFC 6749 §4.1.2.1).

use std::collections::HashMap;
use std::sync::Arc;

use rusty_json::{json, Value};
use rusty_oauth::pkce::{CodeChallengeMethod, Pkce};

use crate::issuer::Issuer;
use crate::provider::{
    AuthorizationParams, ConsentOutcome, Provider, RegisterError, Subject, CONSENT_PATH,
};
use crate::types::{ClientMetadata, TokenResponse};

/// The protected resource: every caller serves MCP at `/mcp`.
pub const RESOURCE_PATH: &str = "/mcp";

const AUTHORIZATION_PATH: &str = "/authorize";
const TOKEN_PATH: &str = "/token";
const REGISTRATION_PATH: &str = "/register";
const REVOCATION_PATH: &str = "/revoke";
const AS_METADATA_PATH: &str = "/.well-known/oauth-authorization-server";
const AS_METADATA_ALIAS_PATH: &str = "/.well-known/oauth-authorization-server/mcp";
const PR_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
const PR_METADATA_ALIAS_PATH: &str = "/.well-known/oauth-protected-resource";

/// Every path [`Endpoints::handle`] answers, for a router that registers
/// routes one by one.
pub const PATHS: &[&str] = &[
    AS_METADATA_PATH,
    AS_METADATA_ALIAS_PATH,
    PR_METADATA_PATH,
    PR_METADATA_ALIAS_PATH,
    AUTHORIZATION_PATH,
    CONSENT_PATH,
    TOKEN_PATH,
    REGISTRATION_PATH,
    REVOCATION_PATH,
];

const NO_STORE: (&str, &str) = ("cache-control", "no-store");
const NO_CACHE: (&str, &str) = ("pragma", "no-cache");
const PUBLIC_HOUR: (&str, &str) = ("cache-control", "public, max-age=3600");

/// The request methods the endpoints tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Other,
}

/// One HTTP request, as the endpoints need it.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub method: Method,
    /// The path, without the query string.
    pub path: &'a str,
    /// The raw query string, without the `?`; empty when there is none.
    pub query: &'a str,
    /// The body: form-encoded for every POST but `/register`, which is JSON.
    pub body: &'a [u8],
}

/// One HTTP response. `headers` includes `content-type` when there is a
/// body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            headers: vec![("content-type", "application/json".to_string())],
            body: body.to_json_string().into_bytes(),
        }
    }

    fn html(status: u16, body: String) -> Self {
        Self {
            status,
            headers: vec![("content-type", "text/html; charset=utf-8".to_string())],
            body: body.into_bytes(),
        }
    }

    fn redirect(location: &str) -> Self {
        Self::empty(302)
            .with_header(("location", location))
            .with_header(NO_STORE)
    }

    fn with_header(mut self, (name, value): (&'static str, &str)) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

/// How the consent page names the server and the token it asks for.
#[derive(Debug, Clone, Copy)]
pub struct Branding {
    /// Short name, used as RFC 9728's `resource_name` and in the page title.
    pub resource_name: &'static str,
    /// What the client gets access to, e.g. "your remind_me memory store".
    pub resource_description: &'static str,
    /// The label over the token field, e.g. "Your rusty_tick API token".
    pub token_label: &'static str,
}

/// The OAuth endpoints of one server.
pub struct Endpoints {
    provider: Arc<Provider>,
    issuer: Issuer,
    branding: Branding,
}

impl Endpoints {
    /// The endpoints for `provider`, advertised under `issuer`.
    pub fn new(provider: Arc<Provider>, issuer: Issuer, branding: Branding) -> Self {
        Self {
            provider,
            issuer,
            branding,
        }
    }

    /// The provider, for a server's `/mcp` gate.
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// The RFC 9728 metadata URL for `/mcp`, which a `401` points at.
    pub fn resource_metadata_url(&self) -> String {
        self.issuer.endpoint(PR_METADATA_PATH)
    }

    /// Whether `path` is one of the OAuth endpoints, for a server that
    /// decides authentication before routing.
    pub fn is_oauth_path(path: &str) -> bool {
        PATHS.contains(&path)
    }

    /// The `401` for a `/mcp` request without a valid bearer: RFC 6750's
    /// challenge, pointing at the protected-resource metadata so a client
    /// can discover this server.
    pub fn unauthorized(&self) -> Response {
        let challenge = format!(
            "Bearer error=\"invalid_token\", error_description=\"Authentication required\", resource_metadata=\"{}\"",
            self.resource_metadata_url()
        );
        let body =
            json!({ "error": "invalid_token", "error_description": "Authentication required" });
        Response::json(401, &body).with_header(("www-authenticate", &challenge))
    }

    /// Answer `request` if it is an OAuth request; `None` otherwise.
    pub fn handle(&self, request: &Request<'_>, now: i64) -> Option<Response> {
        use Method::{Get, Post};
        let response = match (request.path, request.method) {
            (AS_METADATA_PATH | AS_METADATA_ALIAS_PATH, Get) => self.as_metadata(),
            (PR_METADATA_PATH | PR_METADATA_ALIAS_PATH, Get) => self.protected_resource_metadata(),
            (AUTHORIZATION_PATH, Get) => {
                self.authorize(&Form::parse(request.query.as_bytes()), now)
            }
            (AUTHORIZATION_PATH, Post) => self.authorize(&Form::parse(request.body), now),
            (CONSENT_PATH, Get) => self.consent_page(&Form::parse(request.query.as_bytes()), now),
            (CONSENT_PATH, Post) => self.consent_decision(&Form::parse(request.body), now),
            (TOKEN_PATH, Post) => self.token(&Form::parse(request.body), now),
            (REGISTRATION_PATH, Post) => self.register(request.body),
            (REVOCATION_PATH, Post) => self.revoke(&Form::parse(request.body), now),
            (path, _) if Self::is_oauth_path(path) => Response::empty(405),
            _ => return None,
        };
        Some(response)
    }

    // -- metadata (RFC 8414 / RFC 9728) ---------------------------------------

    fn as_metadata(&self) -> Response {
        let body = json!({
            "issuer": self.issuer.as_str(),
            "authorization_endpoint": self.issuer.endpoint(AUTHORIZATION_PATH),
            "token_endpoint": self.issuer.endpoint(TOKEN_PATH),
            "registration_endpoint": self.issuer.endpoint(REGISTRATION_PATH),
            "revocation_endpoint": self.issuer.endpoint(REVOCATION_PATH),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
            "revocation_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
            "code_challenge_methods_supported": ["S256"],
        });
        Response::json(200, &body).with_header(PUBLIC_HOUR)
    }

    fn protected_resource_metadata(&self) -> Response {
        let body = json!({
            "resource": self.issuer.endpoint(RESOURCE_PATH),
            "authorization_servers": [self.issuer.as_str()],
            "bearer_methods_supported": ["header"],
            "resource_name": self.branding.resource_name,
        });
        Response::json(200, &body).with_header(PUBLIC_HOUR)
    }

    // -- /authorize -----------------------------------------------------------

    fn authorize(&self, params: &Form, now: i64) -> Response {
        let state = params.get("state");
        let Some(client_id) = params.get("client_id") else {
            return authorize_json_error("client_id is required", state);
        };
        let Some(client) = self.provider.get_client(client_id) else {
            return authorize_json_error(&format!("Client ID '{client_id}' not found"), state);
        };
        let Ok(redirect_uri) = client.validate_redirect_uri(params.get("redirect_uri")) else {
            return authorize_json_error("redirect_uri not registered for client", state);
        };

        // From here on every failure redirects back with an error.
        let fail = |error: &str, description: &str| {
            Response::redirect(&construct_redirect_uri(
                &redirect_uri,
                &[
                    ("error", Some(error)),
                    ("error_description", Some(description)),
                    ("state", state),
                ],
            ))
        };
        match params.get("response_type") {
            Some("code") => {}
            Some(_) => return fail("unsupported_response_type", "response_type must be 'code'"),
            None => return fail("invalid_request", "response_type is required"),
        }
        let Some(code_challenge) = params.get("code_challenge") else {
            return fail("invalid_request", "code_challenge is required");
        };
        if params
            .get("code_challenge_method")
            .is_some_and(|m| m != "S256")
        {
            return fail("invalid_request", "code_challenge_method must be S256");
        }
        let scopes = match client.validate_scope(params.get("scope")) {
            Ok(scopes) => scopes,
            Err(description) => return fail("invalid_scope", &description),
        };

        let auth_params = AuthorizationParams {
            state: state.map(str::to_string),
            scopes,
            code_challenge: code_challenge.to_string(),
            redirect_uri: redirect_uri.clone(),
            redirect_uri_provided_explicitly: params.get("redirect_uri").is_some(),
            resource: params.get("resource").map(str::to_string),
        };
        match self.provider.authorize(client_id, auth_params, now) {
            Ok(location) => Response::redirect(&location),
            Err(e) => server_error("could not start the authorization", &e),
        }
    }

    // -- /consent ---------------------------------------------------------------

    fn consent_page(&self, query: &Form, now: i64) -> Response {
        let txn = query.get("txn").unwrap_or_default();
        let Some(view) = self.provider.pending_consent(txn, now) else {
            return expired();
        };
        let client = view.client_name.unwrap_or(view.client_id);
        let page = consent_html(&self.branding, &client, &view.redirect_uri, txn);
        Response::html(200, page).with_header(NO_STORE)
    }

    fn consent_decision(&self, form: &Form, now: i64) -> Response {
        let txn = form.get("txn").unwrap_or_default();
        let token = form.get(TOKEN_FIELD).unwrap_or_default();
        let approved_as: Option<Subject> = match form.get("action") {
            Some("approve") => self.provider.verify(token),
            _ => None,
        };
        let outcome = match self.provider.decide_consent(txn, approved_as, now) {
            Ok(outcome) => outcome,
            Err(e) => return server_error("could not issue an authorization code", &e),
        };
        match outcome {
            ConsentOutcome::Expired => expired(),
            ConsentOutcome::Denied {
                redirect_uri,
                state,
            } => Response::redirect(&construct_redirect_uri(
                &redirect_uri,
                &[
                    ("error", Some("access_denied")),
                    ("state", state.as_deref()),
                ],
            )),
            ConsentOutcome::Approved {
                code,
                redirect_uri,
                state,
            } => Response::redirect(&construct_redirect_uri(
                &redirect_uri,
                &[("code", Some(code.as_str())), ("state", state.as_deref())],
            )),
        }
    }

    // -- /token -------------------------------------------------------------------

    fn token(&self, form: &Form, now: i64) -> Response {
        let Some(client_id) = form.get("client_id") else {
            return token_error(401, "unauthorized_client", "Missing client_id");
        };
        if self.provider.get_client(client_id).is_none() {
            return token_error(401, "unauthorized_client", "Invalid client_id");
        }
        match form.get("grant_type") {
            Some("authorization_code") => self.code_grant(form, client_id, now),
            Some("refresh_token") => self.refresh_grant(form, client_id, now),
            Some(other) => token_error(
                400,
                "unsupported_grant_type",
                &format!("Unsupported grant type: {other}"),
            ),
            None => token_error(400, "invalid_request", "grant_type is required"),
        }
    }

    fn code_grant(&self, form: &Form, client_id: &str, now: i64) -> Response {
        let Some(code) = form.get("code") else {
            return token_error(400, "invalid_request", "code is required");
        };
        let Some(loaded) = self.provider.load_authorization_code(client_id, code) else {
            return token_error(400, "invalid_grant", "authorization code does not exist");
        };
        // RFC 6749 §10.5: codes expire.
        if loaded.expires_at < now {
            return token_error(400, "invalid_grant", "authorization code has expired");
        }
        // RFC 6749 §10.6: redirect_uri must not change between the legs.
        let expected = loaded
            .redirect_uri_provided_explicitly
            .then_some(loaded.redirect_uri.as_str());
        if form.get("redirect_uri") != expected {
            return token_error(
                400,
                "invalid_request",
                "redirect_uri did not match the one used when creating auth code",
            );
        }
        let Some(verifier) = form.get("code_verifier") else {
            return token_error(400, "invalid_request", "code_verifier is required");
        };
        // RFC 7636 §4.6. A mismatch does not consume the code.
        if !Pkce::verify(verifier, &loaded.code_challenge, CodeChallengeMethod::S256) {
            return token_error(400, "invalid_grant", "incorrect code_verifier");
        }
        match self.provider.exchange_authorization_code(code, now) {
            Ok(Some(tokens)) => token_success(&tokens),
            Ok(None) => token_error(400, "invalid_grant", "authorization code does not exist"),
            // The code is spent but the tokens never reached disk: our fault,
            // and handing back a token the next request rejects is worse.
            Err(e) => {
                eprintln!("oauth: could not persist issued tokens: {e}");
                token_error(500, "server_error", "issued tokens could not be persisted")
            }
        }
    }

    fn refresh_grant(&self, form: &Form, client_id: &str, now: i64) -> Response {
        let Some(refresh_token) = form.get("refresh_token") else {
            return token_error(400, "invalid_request", "refresh_token is required");
        };
        let Some(loaded) = self
            .provider
            .load_refresh_token(client_id, refresh_token, now)
        else {
            return token_error(400, "invalid_grant", "refresh token does not exist");
        };
        let scopes: Vec<String> = match form.get("scope") {
            Some(requested) => requested.split(' ').map(str::to_string).collect(),
            None => loaded.scopes.clone(),
        };
        if let Some(scope) = scopes.iter().find(|s| !loaded.scopes.contains(s)) {
            return token_error(
                400,
                "invalid_scope",
                &format!("cannot request scope `{scope}` not provided by refresh token"),
            );
        }
        match self
            .provider
            .exchange_refresh_token(&loaded, refresh_token, scopes, now)
        {
            Ok(tokens) => token_success(&tokens),
            Err(e) => {
                eprintln!("oauth: could not persist rotated tokens: {e}");
                token_error(500, "server_error", "rotated tokens could not be persisted")
            }
        }
    }

    // -- /register (RFC 7591) -------------------------------------------------------

    fn register(&self, body: &[u8]) -> Response {
        let metadata: ClientMetadata = match rusty_json::from_slice(body) {
            Ok(metadata) => metadata,
            Err(e) => {
                let body = json!({
                    "error": "invalid_client_metadata",
                    "error_description": e.to_string(),
                });
                return Response::json(400, &body);
            }
        };
        match self.provider.register_client(metadata) {
            Ok(info) => match rusty_json::to_value(&info) {
                Ok(body) => Response::json(201, &body),
                Err(e) => server_error("registration could not be encoded", &e),
            },
            Err(RegisterError::Invalid(err)) => {
                let body =
                    json!({ "error": err.error, "error_description": err.error_description });
                Response::json(400, &body)
            }
            // A valid registration that could not be stored is our fault, not
            // bad metadata.
            Err(RegisterError::Storage(e)) => {
                eprintln!("oauth: could not persist client registration: {e}");
                let body = json!({
                    "error": "server_error",
                    "error_description": "registration could not be persisted",
                });
                Response::json(500, &body)
            }
        }
    }

    // -- /revoke (RFC 7009) -----------------------------------------------------------

    fn revoke(&self, form: &Form, now: i64) -> Response {
        let (Some(token), Some(client_id)) = (form.get("token"), form.get("client_id")) else {
            let body = json!({
                "error": "invalid_request",
                "error_description": "token and client_id are required",
            });
            return Response::json(400, &body);
        };
        if self.provider.get_client(client_id).is_none() {
            let body =
                json!({ "error": "unauthorized_client", "error_description": "Invalid client_id" });
            return Response::json(401, &body);
        }
        let access = || {
            self.provider
                .load_access_token(token, now)
                .map(|a| a.client_id)
        };
        let refresh = || {
            self.provider
                .load_refresh_token(client_id, token, now)
                .map(|r| r.client_id)
        };
        let owner = if form.get("token_type_hint") == Some("refresh_token") {
            refresh().or_else(access)
        } else {
            access().or_else(refresh)
        };
        // An unknown or foreign token is still a 200 (RFC 7009 §2.2): a
        // revocation never reveals whether a token existed. A revocation we
        // failed to carry out is not that, and must not read as success.
        if owner.as_deref() == Some(client_id) {
            if let Err(e) = self.provider.revoke_tokens_for_client(client_id) {
                eprintln!("oauth: could not persist token revocation: {e}");
                let body = json!({
                    "error": "server_error",
                    "error_description": "revocation could not be persisted",
                });
                return Response::json(500, &body);
            }
        }
        Response::empty(200)
            .with_header(NO_STORE)
            .with_header(NO_CACHE)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The consent form's token field. Named `owner_token` for remind-me, whose
/// consent page this was first.
const TOKEN_FIELD: &str = "owner_token";

/// A decoded query string or form body. The first value of a repeated key
/// wins.
struct Form(HashMap<String, String>);

impl Form {
    fn parse(input: &[u8]) -> Self {
        let mut fields = HashMap::new();
        for (key, value) in rusty_url::form_urlencoded::parse(input) {
            fields
                .entry(key.into_owned())
                .or_insert_with(|| value.into_owned());
        }
        Self(fields)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

fn authorize_json_error(description: &str, state: Option<&str>) -> Response {
    let mut body = json!({ "error": "invalid_request", "error_description": description });
    if let (Some(state), Some(map)) = (state, body.as_object_mut()) {
        map.insert("state".to_string(), Value::from(state));
    }
    Response::json(400, &body).with_header(NO_STORE)
}

fn token_error(status: u16, error: &str, description: &str) -> Response {
    let body = json!({ "error": error, "error_description": description });
    Response::json(status, &body)
        .with_header(NO_STORE)
        .with_header(NO_CACHE)
}

fn token_success(tokens: &TokenResponse) -> Response {
    match rusty_json::to_value(tokens) {
        Ok(body) => Response::json(200, &body)
            .with_header(NO_STORE)
            .with_header(NO_CACHE),
        Err(e) => server_error("tokens could not be encoded", &e),
    }
}

fn server_error(what: &str, error: &dyn std::fmt::Display) -> Response {
    eprintln!("oauth: {what}: {error}");
    Response::json(500, &json!({ "error": "server_error" }))
}

const EXPIRED_HTML: &str = "<!doctype html><html><body><h2>Authorization request expired</h2>\
<p>This consent link is no longer valid. Retry the connection from your client.</p></body></html>";

fn expired() -> Response {
    Response::html(400, EXPIRED_HTML.to_string())
}

/// Percent-encode one query component, form style (space as `+`).
fn urlencode_component(s: &str) -> String {
    rusty_url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Append `params` to `base`'s query string, keeping any query it already
/// has. `None` values are dropped.
fn construct_redirect_uri(base: &str, params: &[(&str, Option<&str>)]) -> String {
    let (path, existing) = base.split_once('?').unwrap_or((base, ""));
    let mut pieces: Vec<String> = existing
        .split('&')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    for (key, value) in params {
        if let Some(v) = value {
            pieces.push(format!(
                "{}={}",
                urlencode_component(key),
                urlencode_component(v)
            ));
        }
    }
    if pieces.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", pieces.join("&"))
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn consent_html(branding: &Branding, client: &str, redirect_uri: &str, txn: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} — authorize connector</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 26rem; margin: 4rem auto; padding: 0 1rem; }}
  .card {{ border: 1px solid #ddd; border-radius: 8px; padding: 1.5rem; }}
  input[type=password] {{ width: 100%; padding: .5rem; margin: .75rem 0; box-sizing: border-box; }}
  button {{ padding: .5rem 1.25rem; border-radius: 6px; border: 1px solid #888; cursor: pointer; }}
  button.approve {{ background: #1a7f37; color: #fff; border-color: #1a7f37; }}
  code {{ background: #f4f4f4; padding: 0 .25rem; }}
</style></head><body>
<div class="card">
  <h2>Authorize connector</h2>
  <p><strong>{client}</strong> wants to access {what}.</p>
  <p>Redirects to: <code>{redirect}</code></p>
  <form method="post" action="consent">
    <input type="hidden" name="txn" value="{txn}">
    <label for="{field}">{label}</label>
    <input type="password" id="{field}" name="{field}" autocomplete="off" autofocus>
    <button class="approve" name="action" value="approve">Approve</button>
    <button name="action" value="deny">Deny</button>
  </form>
</div>
</body></html>
"#,
        name = html_escape(branding.resource_name),
        what = html_escape(branding.resource_description),
        label = html_escape(branding.token_label),
        field = TOKEN_FIELD,
        client = html_escape(client),
        redirect = html_escape(redirect_uri),
        txn = html_escape(txn),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Verify;
    use crate::store::StateStore;

    const TOKEN: &str = "a-server-token-long-enough";
    const BRANDING: Branding = Branding {
        resource_name: "test",
        resource_description: "the test store",
        token_label: "Your test token",
    };

    fn endpoints() -> (Endpoints, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let verify: Verify = Box::new(|t| (t == TOKEN).then(|| Subject::new("alice")));
        let provider = Provider::new(verify, StateStore::new(dir.path().join("oauth.json")));
        let issuer = crate::validate_issuer("https://example.test").unwrap();
        (Endpoints::new(Arc::new(provider), issuer, BRANDING), dir)
    }

    fn call(e: &Endpoints, method: Method, target: &str, body: &[u8]) -> Response {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let request = Request {
            method,
            path,
            query,
            body,
        };
        e.handle(&request, crate::now_unix())
            .expect("an OAuth path")
    }

    fn header<'a>(r: &'a Response, name: &str) -> &'a str {
        r.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    fn json_of(r: &Response) -> Value {
        std::str::from_utf8(&r.body).unwrap().parse().unwrap()
    }

    fn query_value(location: &str, key: &str) -> Option<String> {
        let query = location.split_once('?')?.1;
        Form::parse(query.as_bytes()).get(key).map(str::to_string)
    }

    /// Register, authorize and approve with `token`; the redirect back.
    fn consent(e: &Endpoints, token: &str, challenge: &str) -> (String, Response) {
        let reg = call(
            e,
            Method::Post,
            "/register",
            br#"{"redirect_uris":["https://claude.ai/cb"],"client_name":"claude.ai","software_id":"x"}"#,
        );
        assert_eq!(reg.status, 201);
        let reg = json_of(&reg);
        assert_eq!(reg["token_endpoint_auth_method"], "none");
        assert_eq!(reg["software_id"], "x", "unknown metadata is echoed back");
        let client_id = reg["client_id"].as_str().unwrap().to_string();

        let target = format!(
            "/authorize?response_type=code&client_id={client_id}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fcb&code_challenge={challenge}&code_challenge_method=S256&state=xyz"
        );
        let authorize = call(e, Method::Get, &target, b"");
        assert_eq!(authorize.status, 302);
        let location = header(&authorize, "location").to_string();
        let txn = query_value(&location, "txn").unwrap();

        let page = call(e, Method::Get, &location, b"");
        assert_eq!(page.status, 200);
        let html = String::from_utf8(page.body).unwrap();
        assert!(html.contains("claude.ai") && html.contains("Your test token"));

        let form = format!("txn={txn}&owner_token={token}&action=approve");
        (
            client_id,
            call(e, Method::Post, "/consent", form.as_bytes()),
        )
    }

    #[test]
    fn metadata_points_every_endpoint_at_the_issuer() {
        let (e, _dir) = endpoints();
        let as_meta = json_of(&call(&e, Method::Get, AS_METADATA_PATH, b""));
        assert_eq!(as_meta["issuer"], "https://example.test");
        assert_eq!(as_meta["token_endpoint"], "https://example.test/token");
        for path in [PR_METADATA_PATH, PR_METADATA_ALIAS_PATH] {
            let pr = json_of(&call(&e, Method::Get, path, b""));
            assert_eq!(pr["resource"], "https://example.test/mcp");
            assert_eq!(pr["resource_name"], "test");
        }
    }

    #[test]
    fn non_oauth_paths_fall_through_and_wrong_methods_are_405() {
        let (e, _dir) = endpoints();
        let request = Request {
            method: Method::Get,
            path: "/mcp",
            query: "",
            body: b"",
        };
        assert!(e.handle(&request, 0).is_none());
        assert_eq!(call(&e, Method::Get, "/token", b"").status, 405);
    }

    #[test]
    fn the_full_flow_issues_a_token_for_the_approving_subject() {
        let (e, _dir) = endpoints();
        let verifier = crate::generate_token().unwrap();
        let challenge =
            rusty_oauth::pkce::Pkce::from_verifier(verifier.clone(), CodeChallengeMethod::S256)
                .unwrap()
                .code_challenge;
        let (client_id, back) = consent(&e, TOKEN, &challenge);
        assert_eq!(back.status, 302);
        let location = header(&back, "location");
        assert_eq!(query_value(location, "state").as_deref(), Some("xyz"));
        let code = query_value(location, "code").unwrap();

        let wrong = format!(
            "grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fcb&code_verifier=wrong-verifier-wrong-verifier-wrong-verifier-x"
        );
        let refused = call(&e, Method::Post, "/token", wrong.as_bytes());
        assert_eq!(json_of(&refused)["error"], "invalid_grant");

        let form = format!(
            "grant_type=authorization_code&code={code}&client_id={client_id}&redirect_uri=https%3A%2F%2Fclaude.ai%2Fcb&code_verifier={verifier}"
        );
        let tokens = call(&e, Method::Post, "/token", form.as_bytes());
        assert_eq!(
            tokens.status, 200,
            "a PKCE mismatch must not consume the code"
        );
        assert_eq!(header(&tokens, "cache-control"), "no-store");
        let tokens = json_of(&tokens);
        let access = tokens["access_token"].as_str().unwrap();
        let now = crate::now_unix();
        assert_eq!(
            e.provider().subject_of(access, now),
            Some(Subject::new("alice"))
        );

        let replay = call(&e, Method::Post, "/token", form.as_bytes());
        assert_eq!(replay.status, 400);

        let refresh = tokens["refresh_token"].as_str().unwrap();
        let form =
            format!("grant_type=refresh_token&refresh_token={refresh}&client_id={client_id}");
        let rotated = json_of(&call(&e, Method::Post, "/token", form.as_bytes()));
        let rotated = rotated["access_token"].as_str().unwrap();
        assert_eq!(
            e.provider().subject_of(rotated, now),
            Some(Subject::new("alice"))
        );

        let form = format!("token={rotated}&client_id={client_id}");
        assert_eq!(
            call(&e, Method::Post, "/revoke", form.as_bytes()).status,
            200
        );
        assert_eq!(e.provider().subject_of(rotated, now), None);
    }

    #[test]
    fn a_wrong_token_denies_like_an_explicit_deny() {
        let (e, _dir) = endpoints();
        let (_, back) = consent(&e, "not-the-token", "challenge");
        assert_eq!(back.status, 302);
        let location = header(&back, "location");
        assert_eq!(
            query_value(location, "error").as_deref(),
            Some("access_denied")
        );
        assert!(query_value(location, "code").is_none());
    }

    #[test]
    fn authorize_errors_are_json_until_the_redirect_uri_is_known() {
        let (e, _dir) = endpoints();
        let unknown = call(&e, Method::Get, "/authorize?client_id=nope&state=s", b"");
        assert_eq!(unknown.status, 400);
        assert_eq!(json_of(&unknown)["state"], "s");

        let reg = call(
            &e,
            Method::Post,
            "/register",
            br#"{"redirect_uris":["https://a/cb"]}"#,
        );
        let client_id = json_of(&reg)["client_id"].as_str().unwrap().to_string();
        let target = format!("/authorize?client_id={client_id}&response_type=token");
        let wrong_type = call(&e, Method::Get, &target, b"");
        assert_eq!(wrong_type.status, 302);
        let location = header(&wrong_type, "location");
        assert!(location.starts_with("https://a/cb?error=unsupported_response_type"));
    }

    #[test]
    fn registration_rejects_bad_json_and_missing_redirect_uris() {
        let (e, _dir) = endpoints();
        assert_eq!(call(&e, Method::Post, "/register", b"not json").status, 400);
        let empty = call(&e, Method::Post, "/register", br#"{"redirect_uris":[]}"#);
        assert_eq!(json_of(&empty)["error"], "invalid_client_metadata");
    }

    #[test]
    fn an_expired_consent_link_is_a_400_page() {
        let (e, _dir) = endpoints();
        assert_eq!(call(&e, Method::Get, "/consent?txn=nope", b"").status, 400);
        let post = call(&e, Method::Post, "/consent", b"txn=nope&action=approve");
        assert_eq!(post.status, 400);
    }

    #[test]
    fn unauthorized_points_at_the_resource_metadata() {
        let (e, _dir) = endpoints();
        let r = e.unauthorized();
        assert_eq!(r.status, 401);
        assert!(header(&r, "www-authenticate").contains(
            "resource_metadata=\"https://example.test/.well-known/oauth-protected-resource/mcp\""
        ));
    }

    #[test]
    fn redirect_uris_keep_their_query_and_drop_none_values() {
        assert_eq!(
            construct_redirect_uri(
                "https://claude.ai/cb?x=1",
                &[("code", Some("a b")), ("state", None)]
            ),
            "https://claude.ai/cb?x=1&code=a+b"
        );
        assert_eq!(urlencode_component("a&b=c"), "a%26b%3Dc");
    }
}
