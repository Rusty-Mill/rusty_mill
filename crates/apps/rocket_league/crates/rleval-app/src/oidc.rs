//! Google sign-in (OpenID Connect authorization-code flow with PKCE).
//!
//! The protocol pieces — authorization URL, PKCE, the code exchange, JWKS,
//! RS256 verification and claim validation — come from the Rusty-Mill
//! `rusty_oauth` crate. This module adds what it deliberately leaves out: the
//! server-side login state, the browser session, the email → account allowlist,
//! and the [`HttpTransport`] port that talks to the provider.
//!
//! **Who may sign in.** Signing in with Google proves an email address; it does
//! not, by itself, make anyone a user. Only verified emails listed in the users
//! file (`email:account`) get a session, and each maps to exactly one
//! [`AccountId`]. There is no auto-provisioning.
//!
//! What a login checks, in order: the callback's `state` matches a pending login
//! (single use, 10-minute lifetime); the provider's token response carries an
//! `id_token`; its RS256 signature verifies against the provider's JWKS (the
//! algorithm is fixed by us, never taken from the token); `iss` is one of the
//! configured issuers; `aud` is our client id; it is unexpired; its `nonce` is
//! the one we issued; and `email_verified` is true.
//!
//! Sessions are in memory: restarting the server signs everyone out. That is a
//! deliberate simplification, not a security property to rely on.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusty_oauth::authorization::{parse_callback_query, AuthorizationRequest};
use rusty_oauth::client::{Client, ClientId, ClientSecret};
use rusty_oauth::jwks::JwkSet;
use rusty_oauth::jwt::{self, rsa::verify_rs256, Validation};
use rusty_oauth::pkce::Pkce;
use rusty_oauth::request::{HttpRequest, Method};
use rusty_oauth::token::{authorization_code_request, parse_token_response};

use crate::authn::{LoginError, LoginProvider, SignedIn};
use crate::store::{AccountId, StoreError};

const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING: usize = 1024;
const MAX_SESSIONS: usize = 10_000;
/// How long a fetched JWKS is trusted before it is refreshed.
const JWKS_TTL: Duration = Duration::from_secs(60 * 60);
/// A JWKS is never refetched more often than this, even for an unknown `kid`
/// (a forged token must not be able to make us hammer the provider).
const JWKS_MIN_REFETCH: Duration = Duration::from_secs(60);

/// Google's OpenID Connect endpoints and issuers.
pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
pub const GOOGLE_ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

// ---------------------------------------------------------------------------
// Transport port
// ---------------------------------------------------------------------------

/// A completed HTTP exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

/// Sends one HTTP request to the identity provider.
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> Result<HttpReply, LoginError>;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct OidcConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Where the provider sends the browser back (`https://host/auth/callback`).
    pub redirect_uri: String,
    pub authorization_url: String,
    pub token_url: String,
    pub jwks_url: String,
    /// Accepted values of the ID token's `iss` claim.
    pub issuers: Vec<String>,
    pub session_ttl: Duration,
}

impl fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

impl OidcConfig {
    /// Google's endpoints with the caller's client credentials.
    pub fn google(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_uri,
            authorization_url: GOOGLE_AUTH_URL.into(),
            token_url: GOOGLE_TOKEN_URL.into(),
            jwks_url: GOOGLE_JWKS_URL.into(),
            issuers: GOOGLE_ISSUERS.iter().map(|s| s.to_string()).collect(),
            session_ttl: Duration::from_secs(12 * 60 * 60),
        }
    }

    /// Reject configurations that would leak the client secret or a session:
    /// every URL must be https, except loopback http for local development.
    pub fn validate(&self) -> Result<(), LoginError> {
        if self.client_id.trim().is_empty() || self.client_secret.trim().is_empty() {
            return Err(LoginError("OIDC client id and secret are required".into()));
        }
        if self.issuers.is_empty() {
            return Err(LoginError("at least one OIDC issuer is required".into()));
        }
        for (what, url) in [
            ("authorization URL", &self.authorization_url),
            ("token URL", &self.token_url),
            ("JWKS URL", &self.jwks_url),
            ("redirect URI", &self.redirect_uri),
        ] {
            if !is_https_or_loopback(url) {
                return Err(LoginError(format!(
                    "OIDC {what} must be https (loopback http is allowed for development): {url}"
                )));
            }
        }
        Ok(())
    }

    /// An https deployment gets `Secure` cookies.
    pub fn secure_cookies(&self) -> bool {
        self.redirect_uri.starts_with("https://")
    }
}

fn is_https_or_loopback(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

// ---------------------------------------------------------------------------
// Allowlist
// ---------------------------------------------------------------------------

/// Verified emails allowed to sign in, and the account each one is.
#[derive(Debug, Clone, Default)]
pub struct Users(Vec<(String, AccountId)>);

impl Users {
    /// Parse `email:account` lines (`#` comments and blank lines ignored).
    pub fn parse(text: &str) -> Result<Self, LoginError> {
        let mut out: Vec<(String, AccountId)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |m: String| LoginError(format!("users file line {}: {m}", i + 1));
            let (email, account) = line
                .split_once(':')
                .ok_or_else(|| bad("expected email:account".into()))?;
            let email = email.trim().to_ascii_lowercase();
            if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
                return Err(bad(format!("{email:?} is not an email address")));
            }
            let account =
                AccountId::new(account.trim()).map_err(|e: StoreError| bad(e.to_string()))?;
            if out.iter().any(|(e, _)| *e == email) {
                return Err(bad(format!("{email} listed twice")));
            }
            out.push((email, account));
        }
        if out.is_empty() {
            return Err(LoginError("users file lists no one".into()));
        }
        Ok(Self(out))
    }

    pub fn from_file(path: &std::path::Path) -> Result<Self, LoginError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| LoginError(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    pub fn account_for(&self, email: &str) -> Option<&AccountId> {
        let email = email.to_ascii_lowercase();
        self.0.iter().find(|(e, _)| *e == email).map(|(_, a)| a)
    }

    pub fn knows(&self, account: &AccountId) -> bool {
        self.0.iter().any(|(_, a)| a == account)
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

struct Pending {
    verifier: String,
    nonce: String,
    created: Instant,
}

struct Session {
    account: AccountId,
    expires: Instant,
}

#[derive(Default)]
struct State {
    /// `state` parameter → the login it belongs to.
    pending: HashMap<String, Pending>,
    /// Session token → session.
    sessions: HashMap<String, Session>,
    jwks: Option<(JwkSet, Instant)>,
}

pub struct Oidc {
    config: OidcConfig,
    client: Client,
    users: Users,
    transport: Box<dyn HttpTransport>,
    state: Mutex<State>,
}

impl Oidc {
    pub fn new(
        config: OidcConfig,
        users: Users,
        transport: Box<dyn HttpTransport>,
    ) -> Result<Self, LoginError> {
        config.validate()?;
        let client = Client::confidential(
            ClientId::new(config.client_id.clone()),
            ClientSecret::new(config.client_secret.clone()),
        );
        Ok(Self {
            config,
            client,
            users,
            transport,
            state: Mutex::new(State::default()),
        })
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, LoginError> {
        self.state
            .lock()
            .map_err(|_| LoginError("internal error: login state poisoned".into()))
    }

    /// Fetch the provider's signing keys.
    fn fetch_jwks(&self) -> Result<JwkSet, LoginError> {
        let request = HttpRequest {
            method: Method::Get,
            url: self.config.jwks_url.clone(),
            headers: vec![("Accept".into(), "application/json".into())],
            body: Vec::new(),
        };
        let reply = self.transport.send(&request)?;
        if reply.status != 200 {
            return Err(fail(
                "fetching signing keys",
                format!("HTTP {}", reply.status),
            ));
        }
        JwkSet::parse(&reply.body).map_err(|e| fail("parsing signing keys", e))
    }

    /// The signing keys, refreshed when stale — or when `kid` is unknown and the
    /// cache is old enough that a rotation is plausible.
    fn keys_for(&self, kid: &str) -> Result<JwkSet, LoginError> {
        {
            let state = self.state()?;
            if let Some((keys, fetched)) = &state.jwks {
                let age = fetched.elapsed();
                let has_kid = keys.find(kid).is_some();
                if (has_kid && age < JWKS_TTL) || age < JWKS_MIN_REFETCH {
                    return Ok(keys.clone());
                }
            }
        }
        // Fetched outside the lock: a slow provider must not stall every request.
        let fresh = self.fetch_jwks()?;
        self.state()?.jwks = Some((fresh.clone(), Instant::now()));
        Ok(fresh)
    }

    /// Verify an ID token and return the (lowercased) email it vouches for.
    fn verify_id_token(&self, token: &str, expected_nonce: &str) -> Result<String, LoginError> {
        let decoded = jwt::decode_unverified(token).map_err(|e| fail("reading id_token", e))?;
        let kid = decoded
            .header
            .get("kid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoginError("id_token has no key id".into()))?;
        let key = self
            .keys_for(kid)?
            .rsa_key(kid)
            .map_err(|e| fail("selecting signing key", e))?;
        // The algorithm is ours to pick; verify_rs256 rejects any other `alg`.
        let claims = verify_rs256(token, &key).map_err(|e| fail("verifying id_token", e))?;
        let validation = Validation {
            expected_audience: Some(self.config.client_id.clone()),
            ..Validation::default()
        };
        jwt::validate_claims(&claims, &validation).map_err(|e| fail("validating id_token", e))?;

        let issuer = claims.get("iss").and_then(|v| v.as_str()).unwrap_or("");
        if !self.config.issuers.iter().any(|i| i == issuer) {
            return Err(LoginError(
                "id_token was issued by an unexpected issuer".into(),
            ));
        }
        let nonce = claims.get("nonce").and_then(|v| v.as_str()).unwrap_or("");
        if !constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) {
            return Err(LoginError(
                "id_token nonce does not match this login".into(),
            ));
        }
        let verified = match claims.get("email_verified") {
            Some(v) => v.as_bool() == Some(true) || v.as_str() == Some("true"),
            None => false,
        };
        if !verified {
            return Err(LoginError(
                "the Google account's email is not verified".into(),
            ));
        }
        claims
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| LoginError("id_token has no email".into()))
    }

    fn create_session(&self, account: AccountId) -> Result<String, LoginError> {
        let token = random_hex(32)?;
        let mut state = self.state()?;
        let now = Instant::now();
        state.sessions.retain(|_, s| s.expires > now);
        if state.sessions.len() >= MAX_SESSIONS {
            return Err(LoginError("too many active sessions".into()));
        }
        state.sessions.insert(
            token.clone(),
            Session {
                account,
                expires: now + self.config.session_ttl,
            },
        );
        Ok(token)
    }
}

impl LoginProvider for Oidc {
    fn begin(&self) -> Result<String, LoginError> {
        let pkce = Pkce::generate().map_err(|e| fail("starting login", e))?;
        let nonce = random_hex(16)?;
        let built = AuthorizationRequest::new(
            &self.config.authorization_url,
            &self.client,
            &self.config.redirect_uri,
        )
        .scope("openid email")
        .pkce(&pkce)
        .extra_param("nonce", &nonce)
        .build()
        .map_err(|e| fail("starting login", e))?;

        let mut state = self.state()?;
        let now = Instant::now();
        state
            .pending
            .retain(|_, p| now.duration_since(p.created) < PENDING_TTL);
        if state.pending.len() >= MAX_PENDING {
            return Err(LoginError(
                "too many logins in progress; try again shortly".into(),
            ));
        }
        state.pending.insert(
            built.state,
            Pending {
                verifier: pkce.code_verifier,
                nonce,
                created: now,
            },
        );
        Ok(built.url)
    }

    fn complete(&self, callback_query: &str) -> Result<SignedIn, LoginError> {
        let callback = parse_callback_query(callback_query)
            .map_err(|e| fail("the provider returned an error", e))?;
        let state_param = callback
            .state
            .ok_or_else(|| LoginError("callback has no state".into()))?;
        // Single use: a replayed callback finds nothing.
        let pending = self
            .state()?
            .pending
            .remove(&state_param)
            .filter(|p| p.created.elapsed() < PENDING_TTL)
            .ok_or_else(|| LoginError("unknown or expired login; start again".into()))?;

        let request = authorization_code_request(
            &self.config.token_url,
            &self.client,
            &callback.code,
            &self.config.redirect_uri,
            Some(&pending.verifier),
        )
        .map_err(|e| fail("building the token request", e))?;
        let reply = self.transport.send(&request)?;
        let tokens = parse_token_response(reply.status, &reply.body)
            .map_err(|e| fail("the token endpoint refused the code", e))?;
        let id_token = tokens
            .id_token
            .ok_or_else(|| LoginError("the provider returned no id_token".into()))?;

        let email = self.verify_id_token(&id_token, &pending.nonce)?;
        let account = self
            .users
            .account_for(&email)
            .cloned()
            .ok_or_else(|| LoginError(format!("{email} is not allowed to use this server")))?;
        let token = self.create_session(account.clone())?;
        eprintln!("sign-in: {email} signed in as {account}");
        Ok(SignedIn {
            account,
            token,
            max_age_secs: self.config.session_ttl.as_secs(),
        })
    }

    fn session_account(&self, token: &str) -> Option<AccountId> {
        let state = self.state.lock().ok()?;
        let session = state.sessions.get(token)?;
        (session.expires > Instant::now()).then(|| session.account.clone())
    }

    fn end_session(&self, token: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.sessions.remove(token);
        }
    }

    fn secure_cookies(&self) -> bool {
        self.config.secure_cookies()
    }

    fn knows_account(&self, account: &AccountId) -> bool {
        self.users.knows(account)
    }
}

/// A user-safe message for a failure whose detail is for the operator: the
/// detail goes to stderr, the caller sees only what step failed.
fn fail(step: &str, detail: impl fmt::Display) -> LoginError {
    eprintln!("sign-in: {step} failed: {detail}");
    LoginError(format!("{step} failed"))
}

fn random_hex(bytes: usize) -> Result<String, LoginError> {
    let raw =
        rusty_oauth::rand::random_bytes(bytes).map_err(|e| fail("generating randomness", e))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

/// Comparison whose time does not depend on where the inputs first differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Fixture generated by `tests/fixtures/gen_oidc_fixture.py`: a throwaway RSA
    /// key's JWKS and RS256 tokens for every case below. Nonce is `test-nonce`,
    /// audience `client-123`, issuer Google's; the "expired" token expired in 2001.
    const FIXTURE: &str = include_str!("../tests/fixtures/oidc_fixture.json");

    fn fixture() -> serde_json::Value {
        serde_json::from_str(FIXTURE).unwrap()
    }
    fn token(name: &str) -> String {
        fixture()["tokens"][name].as_str().unwrap().to_string()
    }
    fn jwks() -> String {
        fixture()["jwks"].to_string()
    }

    /// Canned replies by URL; records every request it saw.
    struct Fake {
        replies: Mutex<HashMap<String, HttpReply>>,
        seen: Arc<Mutex<Vec<HttpRequest>>>,
    }

    impl HttpTransport for Fake {
        fn send(&self, request: &HttpRequest) -> Result<HttpReply, LoginError> {
            self.seen.lock().unwrap().push(request.clone());
            self.replies
                .lock()
                .unwrap()
                .get(&request.url)
                .cloned()
                .ok_or_else(|| LoginError(format!("no canned reply for {}", request.url)))
        }
    }

    const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

    fn provider(id_token: &str) -> (Oidc, Arc<Mutex<Vec<HttpRequest>>>) {
        let body = format!(
            r#"{{"access_token":"at","token_type":"Bearer","expires_in":3600,"id_token":"{id_token}"}}"#
        );
        provider_with_token_reply(HttpReply { status: 200, body })
    }

    /// A provider whose token endpoint answers with exactly `token_reply`.
    fn provider_with_token_reply(token_reply: HttpReply) -> (Oidc, Arc<Mutex<Vec<HttpRequest>>>) {
        let mut replies = HashMap::new();
        replies.insert(TOKEN_URL.to_string(), token_reply);
        replies.insert(
            GOOGLE_JWKS_URL.to_string(),
            HttpReply {
                status: 200,
                body: jwks(),
            },
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let fake = Fake {
            replies: Mutex::new(replies),
            seen: Arc::clone(&seen),
        };
        let config = OidcConfig::google(
            "client-123".into(),
            "s3cret".into(),
            "https://rl.example/auth/callback".into(),
        );
        let users = Users::parse("Alice@Example.com:alice\nbob@example.com:bob").unwrap();
        (Oidc::new(config, users, Box::new(fake)).unwrap(), seen)
    }

    /// Register a login as if `begin()` had issued it with the fixture nonce.
    fn pending(oidc: &Oidc, state: &str) {
        oidc.state().unwrap().pending.insert(
            state.to_string(),
            Pending {
                verifier: "verifier-abc".into(),
                nonce: "test-nonce".into(),
                created: Instant::now(),
            },
        );
    }

    fn login_with(
        name: &str,
    ) -> (
        Result<SignedIn, LoginError>,
        Oidc,
        Arc<Mutex<Vec<HttpRequest>>>,
    ) {
        let (oidc, seen) = provider(&token(name));
        pending(&oidc, "st1");
        let result = oidc.complete("code=authcode&state=st1");
        (result, oidc, seen)
    }

    fn rejected(name: &str) -> String {
        let (result, oidc, _) = login_with(name);
        let err = result.expect_err(name).to_string();
        assert!(
            oidc.state().unwrap().sessions.is_empty(),
            "{name}: no session on failure"
        );
        err
    }

    #[test]
    fn a_valid_login_creates_a_session_for_the_allowlisted_account() {
        let (result, oidc, seen) = login_with("valid");
        let done = result.unwrap();
        assert_eq!(
            done.account.as_str(),
            "alice",
            "email matched case-insensitively"
        );
        assert_eq!(done.token.len(), 64);
        assert_eq!(oidc.session_account(&done.token).unwrap().as_str(), "alice");
        assert!(oidc.session_account("forged").is_none());

        // The code exchange carried the PKCE verifier and authenticated the client.
        let requests = seen.lock().unwrap();
        let exchange = requests.iter().find(|r| r.url == TOKEN_URL).unwrap();
        let body = String::from_utf8(exchange.body.clone()).unwrap();
        assert!(body.contains("code_verifier=verifier-abc"), "{body}");
        assert!(body.contains("code=authcode") && body.contains("grant_type=authorization_code"));
        assert!(exchange
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("authorization")));
    }

    #[test]
    fn tokens_that_fail_any_check_are_rejected_and_create_no_session() {
        for (name, needle) in [
            ("wrong_audience", "validating id_token"),
            ("expired", "validating id_token"),
            ("wrong_nonce", "nonce"),
            ("wrong_issuer", "issuer"),
            ("tampered", "verifying id_token"),
            ("alg_none", "verifying id_token"),
            ("unknown_kid", "selecting signing key"),
            ("email_unverified", "not verified"),
            ("unlisted_email", "not allowed"),
        ] {
            let err = rejected(name);
            assert!(err.contains(needle), "{name}: {err}");
        }
    }

    #[test]
    fn a_login_can_only_be_completed_once() {
        let (result, oidc, _) = login_with("valid");
        result.unwrap();
        let replay = oidc
            .complete("code=authcode&state=st1")
            .unwrap_err()
            .to_string();
        assert!(replay.contains("unknown or expired"), "{replay}");
    }

    #[test]
    fn unknown_missing_and_expired_state_is_refused() {
        let (oidc, _) = provider(&token("valid"));
        assert!(oidc.complete("code=c&state=never-issued").is_err());
        assert!(oidc
            .complete("code=c")
            .unwrap_err()
            .to_string()
            .contains("no state"));
        pending(&oidc, "old");
        oidc.state()
            .unwrap()
            .pending
            .get_mut("old")
            .unwrap()
            .created = Instant::now()
            .checked_sub(PENDING_TTL + Duration::from_secs(1))
            .unwrap();
        assert!(oidc
            .complete("code=c&state=old")
            .unwrap_err()
            .to_string()
            .contains("expired"));
    }

    #[test]
    fn a_provider_error_in_the_callback_is_reported() {
        let (oidc, _) = provider(&token("valid"));
        let err = oidc
            .complete("error=access_denied&state=x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("provider returned an error"), "{err}");
    }

    #[test]
    fn a_token_endpoint_failure_is_reported() {
        let (oidc, _) = provider_with_token_reply(HttpReply {
            status: 400,
            body: r#"{"error":"invalid_grant"}"#.into(),
        });
        pending(&oidc, "st1");
        let err = oidc.complete("code=c&state=st1").unwrap_err().to_string();
        assert!(err.contains("token endpoint refused"), "{err}");
    }

    #[test]
    fn begin_issues_a_pkce_authorization_url_and_remembers_the_login() {
        let (oidc, _) = provider(&token("valid"));
        let url = oidc.begin().unwrap();
        for part in [
            "client_id=client-123",
            "response_type=code",
            "scope=openid",
            "code_challenge_method=S256",
            "code_challenge=",
            "nonce=",
            "state=",
            "redirect_uri=https%3A%2F%2Frl.example%2Fauth%2Fcallback",
        ] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert_eq!(oidc.state().unwrap().pending.len(), 1);
    }

    #[test]
    fn signing_keys_are_cached_across_logins() {
        let (oidc, seen) = provider(&token("valid"));
        for state in ["a", "b"] {
            pending(&oidc, state);
            oidc.complete(&format!("code=c&state={state}")).unwrap();
        }
        let fetches = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.url == GOOGLE_JWKS_URL)
            .count();
        assert_eq!(fetches, 1);
    }

    #[test]
    fn sessions_expire_and_can_be_ended() {
        let (result, oidc, _) = login_with("valid");
        let done = result.unwrap();
        oidc.end_session(&done.token);
        assert!(oidc.session_account(&done.token).is_none());

        let again = oidc
            .create_session(AccountId::new("alice").unwrap())
            .unwrap();
        oidc.state()
            .unwrap()
            .sessions
            .get_mut(&again)
            .unwrap()
            .expires = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(
            oidc.session_account(&again).is_none(),
            "expired sessions are dead"
        );
    }

    #[test]
    fn config_rejects_insecure_urls_but_allows_loopback_for_development() {
        let mut c = OidcConfig::google("id".into(), "s".into(), "https://h/cb".into());
        assert!(c.validate().is_ok());
        c.token_url = "http://oauth2.googleapis.com/token".into();
        assert!(c.validate().unwrap_err().to_string().contains("token URL"));
        c.token_url = "http://127.0.0.1:9000/token".into();
        assert!(
            c.validate().is_ok(),
            "loopback http is for local mock providers"
        );
        c.token_url = "http://localhost.evil.example/token".into();
        assert!(c.validate().is_err(), "a lookalike host is not loopback");
        let blank = OidcConfig::google("".into(), "s".into(), "https://h/cb".into());
        assert!(blank.validate().is_err());
        assert!(!format!(
            "{:?}",
            OidcConfig::google("id".into(), "s3cret".into(), "https://h".into())
        )
        .contains("s3cret"));
    }

    #[test]
    fn users_file_parsing() {
        let u = Users::parse("# team\nA@x.io:alice\n\nb@x.io : bob").unwrap();
        assert_eq!(u.account_for("a@X.io").unwrap().as_str(), "alice");
        assert!(u.account_for("c@x.io").is_none());
        for (text, needle) in [
            ("alice", "email:account"),
            ("notanemail:alice", "not an email"),
            ("a@x.io:../x", "invalid account"),
            ("a@x.io:a\nA@x.io:b", "twice"),
            ("", "no one"),
        ] {
            let e = Users::parse(text).unwrap_err().to_string();
            assert!(e.contains(needle), "{text:?} -> {e}");
        }
    }
}
