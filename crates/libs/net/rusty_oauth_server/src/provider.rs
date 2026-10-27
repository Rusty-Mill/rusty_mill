//! The OAuth 2.1 policy layer above [`StateStore`]: consent, code and token
//! issuance, refresh rotation, and revocation.
//!
//! There are no accounts or sessions. `/authorize` parks the request and
//! sends the user-agent to `/consent`, where the user pastes a token the
//! server already accepts. [`Verify`] turns that token into a [`Subject`],
//! and the grant acts as that subject from then on. A wrong token and an
//! explicit deny produce the identical outcome, so the form never says
//! which part failed.
//!
//! Authorization codes and pending consent transactions are process-local
//! (`Mutex`-guarded maps): both legs of each exchange hit the same running
//! process, and losing them on restart only means re-running `/authorize`.
//! Client registrations and issued tokens must survive a restart and be
//! visible to an admin tool in another process, so they live in the
//! [`StateStore`].
//!
//! Every method is synchronous. Expiry checks take `now` explicitly, so
//! tests pass any timestamp instead of mocking a clock.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusty_json::{json, Value};

use crate::generate_token;
use crate::store::{StateStore, TokenCounts, TokenKind};
use crate::types::{ClientInformation, ClientMetadata, TokenResponse};

/// Access tokens are short-lived: refresh is cheap and revocation windows
/// stay small.
pub const ACCESS_TOKEN_TTL: i64 = 3600;
/// Refresh tokens live 30 days and rotate on every refresh grant.
pub const REFRESH_TOKEN_TTL: i64 = 30 * 24 * 3600;
/// Authorization codes expire after 5 minutes and are single-use.
pub const AUTH_CODE_TTL: i64 = 300;
/// A pending consent page is valid for 10 minutes.
pub const CONSENT_TTL: i64 = 600;
/// Where the consent form lives (the `/authorize` redirect target).
pub const CONSENT_PATH: &str = "/consent";

/// Current UNIX time in seconds.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Who a grant acts as: a short, stable name chosen by the server, such as
/// a user key. The server resolves it to a live identity on every request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subject(String);

impl Subject {
    /// A subject named `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The subject's name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Turns a token pasted on the consent page into the subject it belongs to,
/// or `None` for a token the server does not accept. It must compare in
/// constant time.
pub type Verify = Box<dyn Fn(&str) -> Option<Subject> + Send + Sync>;

/// Parameters captured at `/authorize` and carried through consent to the
/// issued authorization code.
#[derive(Debug, Clone)]
pub struct AuthorizationParams {
    pub state: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub redirect_uri_provided_explicitly: bool,
    pub resource: Option<String>,
}

#[derive(Debug, Clone)]
struct IssuedAuthorizationCode {
    subject: Subject,
    scopes: Vec<String>,
    expires_at: i64,
    client_id: String,
    code_challenge: String,
    redirect_uri: String,
    redirect_uri_provided_explicitly: bool,
    resource: Option<String>,
}

/// A loaded authorization code, for validation before it is consumed.
#[derive(Debug, Clone)]
pub struct AuthorizationCodeView {
    pub client_id: String,
    pub scopes: Vec<String>,
    pub expires_at: i64,
    pub code_challenge: String,
    pub redirect_uri: String,
    pub redirect_uri_provided_explicitly: bool,
}

#[derive(Debug, Clone)]
struct PendingConsent {
    client_id: String,
    params: AuthorizationParams,
    expires_at: i64,
}

/// What the consent page shows: who wants access and where it redirects.
#[derive(Debug, Clone)]
pub struct PendingConsentView {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uri: String,
}

/// The result of a consent decision.
pub enum ConsentOutcome {
    /// Unknown or expired transaction.
    Expired,
    /// Approved: an authorization code was issued.
    Approved {
        code: String,
        redirect_uri: String,
        state: Option<String>,
    },
    /// Denied: a wrong token or an explicit deny, indistinguishable on
    /// purpose.
    Denied {
        redirect_uri: String,
        state: Option<String>,
    },
}

/// A live access token. `subject` is `None` only for a grant written before
/// subjects existed (remind-me's older state files).
#[derive(Debug, Clone)]
pub struct AccessTokenView {
    pub client_id: String,
    pub subject: Option<Subject>,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
}

/// A live refresh token; `subject` as for [`AccessTokenView`].
#[derive(Debug, Clone)]
pub struct RefreshTokenView {
    pub client_id: String,
    pub subject: Option<Subject>,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
}

/// Why registering a client was refused (RFC 7591 §3.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationError {
    pub error: &'static str,
    pub error_description: String,
}

/// Why a registration failed. The two cases must not share a status code:
/// bad metadata is the client's fault (400), a failed write is ours (500).
#[derive(Debug)]
pub enum RegisterError {
    /// The submitted metadata was rejected (RFC 7591, HTTP 400).
    Invalid(RegistrationError),
    /// The registration could not be written (HTTP 500). The client is not
    /// registered and must not be told it is.
    Storage(io::Error),
}

impl From<RegistrationError> for RegisterError {
    fn from(err: RegistrationError) -> Self {
        Self::Invalid(err)
    }
}

impl From<io::Error> for RegisterError {
    fn from(err: io::Error) -> Self {
        Self::Storage(err)
    }
}

/// The OAuth 2.1 provider over a [`StateStore`].
pub struct Provider {
    verify: Verify,
    store: StateStore,
    codes: Mutex<HashMap<String, IssuedAuthorizationCode>>,
    pending: Mutex<HashMap<String, PendingConsent>>,
}

impl Provider {
    /// A provider that approves consent with `verify` and keeps its state in
    /// `store`.
    pub fn new(verify: Verify, store: StateStore) -> Self {
        Self {
            verify,
            store,
            codes: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// The state store, for admin listings and revocation.
    pub fn store(&self) -> &StateStore {
        &self.store
    }

    /// The subject a pasted token belongs to, if the server accepts it.
    pub fn verify(&self, presented: &str) -> Option<Subject> {
        (self.verify)(presented)
    }

    // -- client registry (RFC 7591) -----------------------------------------

    /// Load a registered client from the state file.
    pub fn get_client(&self, client_id: &str) -> Option<ClientInformation> {
        let record = self.store.get_client(client_id)?;
        rusty_json::from_value(record).ok()
    }

    /// Persist a dynamically registered client.
    ///
    /// Forces `token_endpoint_auth_method = "none"` (no `client_secret`),
    /// whatever the client asked for. A stored secret would have to be kept
    /// in the clear to compare it, while access and refresh tokens are kept
    /// only as hashes; and PKCE (S256, mandatory) already proves possession
    /// without one. So no endpoint ever needs to check a client secret.
    pub fn register_client(
        &self,
        metadata: ClientMetadata,
    ) -> Result<ClientInformation, RegisterError> {
        validate_metadata(&metadata)?;
        let client_id = generate_token()?;
        let info = ClientInformation {
            client_id: Some(client_id.clone()),
            client_secret: None,
            client_id_issued_at: Some(now_unix()),
            client_secret_expires_at: None,
            redirect_uris: metadata.redirect_uris,
            token_endpoint_auth_method: "none".to_string(),
            grant_types: metadata.grant_types,
            response_types: metadata.response_types,
            scope: metadata.scope,
            client_name: metadata.client_name,
            extra: metadata.extra,
        };
        let record = rusty_json::to_value(&info)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        self.store.put_client(&client_id, record)?;
        Ok(info)
    }

    // -- authorization + consent ---------------------------------------------

    /// Park the request and return the `/consent?txn=...` path to redirect
    /// to.
    ///
    /// # Errors
    ///
    /// If no transaction id could be generated.
    pub fn authorize(
        &self,
        client_id: &str,
        params: AuthorizationParams,
        now: i64,
    ) -> io::Result<String> {
        let txn = generate_token()?;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, p| p.expires_at >= now);
        pending.insert(
            txn.clone(),
            PendingConsent {
                client_id: client_id.to_string(),
                params,
                expires_at: now + CONSENT_TTL,
            },
        );
        Ok(format!("{CONSENT_PATH}?txn={txn}"))
    }

    /// Look up a live consent transaction without consuming it.
    pub fn pending_consent(&self, txn: &str, now: i64) -> Option<PendingConsentView> {
        let (client_id, redirect_uri) = {
            let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let entry = pending.get(txn).filter(|e| e.expires_at >= now)?;
            (entry.client_id.clone(), entry.params.redirect_uri.clone())
        };
        let client_name = self.get_client(&client_id).and_then(|c| c.client_name);
        Some(PendingConsentView {
            client_id,
            client_name,
            redirect_uri,
        })
    }

    /// Consume the transaction (single-use whatever the outcome) and issue a
    /// code for `approved_as`, or report the denial. `None` folds together a
    /// wrong token and an explicit deny.
    ///
    /// # Errors
    ///
    /// If no authorization code could be generated.
    pub fn decide_consent(
        &self,
        txn: &str,
        approved_as: Option<Subject>,
        now: i64,
    ) -> io::Result<ConsentOutcome> {
        let entry = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.remove(txn)
        };
        let Some(entry) = entry.filter(|e| e.expires_at >= now) else {
            return Ok(ConsentOutcome::Expired);
        };
        let Some(subject) = approved_as else {
            return Ok(ConsentOutcome::Denied {
                redirect_uri: entry.params.redirect_uri,
                state: entry.params.state,
            });
        };
        let code = generate_token()?;
        let params = entry.params;
        let issued = IssuedAuthorizationCode {
            subject,
            scopes: params.scopes.unwrap_or_default(),
            expires_at: now + AUTH_CODE_TTL,
            client_id: entry.client_id,
            code_challenge: params.code_challenge,
            redirect_uri: params.redirect_uri.clone(),
            redirect_uri_provided_explicitly: params.redirect_uri_provided_explicitly,
            resource: params.resource,
        };
        let mut codes = self.codes.lock().unwrap_or_else(|e| e.into_inner());
        codes.retain(|_, c| c.expires_at >= now);
        codes.insert(code.clone(), issued);
        Ok(ConsentOutcome::Approved {
            code,
            redirect_uri: params.redirect_uri,
            state: params.state,
        })
    }

    // -- authorization-code exchange -----------------------------------------

    /// Look up an issued code without consuming it, so the caller can check
    /// expiry, redirect_uri and PKCE first. A code of another client reads
    /// as absent.
    pub fn load_authorization_code(
        &self,
        client_id: &str,
        code: &str,
    ) -> Option<AuthorizationCodeView> {
        let codes = self.codes.lock().unwrap_or_else(|e| e.into_inner());
        let entry = codes.get(code).filter(|e| e.client_id == client_id)?;
        Some(AuthorizationCodeView {
            client_id: entry.client_id.clone(),
            scopes: entry.scopes.clone(),
            expires_at: entry.expires_at,
            code_challenge: entry.code_challenge.clone(),
            redirect_uri: entry.redirect_uri.clone(),
            redirect_uri_provided_explicitly: entry.redirect_uri_provided_explicitly,
        })
    }

    /// Consume the single-use code and issue an access and refresh token
    /// pair. Only called once every check on the loaded code passed, so a
    /// failed check never consumes it.
    ///
    /// `Ok(None)`: the code did not exist. `Err`: it did, was consumed, and
    /// the tokens could not be written. The code stays consumed either way;
    /// re-admitting it would open a replay window.
    pub fn exchange_authorization_code(
        &self,
        code: &str,
        now: i64,
    ) -> io::Result<Option<TokenResponse>> {
        let entry = {
            let mut codes = self.codes.lock().unwrap_or_else(|e| e.into_inner());
            codes.remove(code)
        };
        let Some(entry) = entry else {
            return Ok(None);
        };
        let grant = Grant {
            client_id: &entry.client_id,
            subject: Some(&entry.subject),
            scopes: entry.scopes,
            resource: entry.resource,
        };
        self.issue_tokens(grant, now).map(Some)
    }

    // -- refresh grant --------------------------------------------------------

    /// Look up a refresh token. Expired or foreign tokens read as absent, and
    /// an expired one is evicted.
    pub fn load_refresh_token(
        &self,
        client_id: &str,
        token: &str,
        now: i64,
    ) -> Option<RefreshTokenView> {
        let meta = self.live_token(TokenKind::Refresh, token, now)?;
        if meta.get("client_id").and_then(Value::as_str) != Some(client_id) {
            return None;
        }
        Some(RefreshTokenView {
            client_id: client_id.to_string(),
            subject: subject_of(&meta),
            scopes: scopes_of(&meta),
            expires_at: meta.get("expires_at").and_then(Value::as_i64),
        })
    }

    /// Rotate: retire the presented refresh token, issue a fresh pair for the
    /// same subject.
    pub fn exchange_refresh_token(
        &self,
        refresh: &RefreshTokenView,
        token: &str,
        scopes: Vec<String>,
        now: i64,
    ) -> io::Result<TokenResponse> {
        // Retire first, and refuse to issue if that fails: otherwise the
        // presented token would stay live beside its replacement.
        self.store.delete_token(TokenKind::Refresh, token)?;
        let grant = Grant {
            client_id: &refresh.client_id,
            subject: refresh.subject.as_ref(),
            scopes,
            resource: None,
        };
        self.issue_tokens(grant, now)
    }

    // -- bearer verification --------------------------------------------------

    /// A live access token issued by this provider. It does not accept the
    /// server's own tokens; see [`Self::verify`] for those.
    pub fn load_access_token(&self, token: &str, now: i64) -> Option<AccessTokenView> {
        let meta = self.live_token(TokenKind::Access, token, now)?;
        let client_id = meta.get("client_id").and_then(Value::as_str)?.to_string();
        Some(AccessTokenView {
            client_id,
            subject: subject_of(&meta),
            scopes: scopes_of(&meta),
            expires_at: meta.get("expires_at").and_then(Value::as_i64),
        })
    }

    /// The subject behind a live access token: what a server's `/mcp` gate
    /// calls when a bearer is not one of its own tokens.
    pub fn subject_of(&self, token: &str, now: i64) -> Option<Subject> {
        self.load_access_token(token, now)?.subject
    }

    // -- revocation (RFC 7009) -------------------------------------------------

    /// Revoke every token of `client_id`. Presenting either half of a
    /// client's pair kills all its tokens; the registration survives, so the
    /// client can authorize again.
    pub fn revoke_tokens_for_client(&self, client_id: &str) -> io::Result<TokenCounts> {
        self.store.delete_tokens_for_client(client_id)
    }

    // -- issuance ---------------------------------------------------------------

    /// A stored token's metadata, unless it has expired. An expired one is
    /// evicted opportunistically: it reads as absent whether or not the
    /// delete lands, so that one failure is deliberately ignored.
    fn live_token(&self, kind: TokenKind, token: &str, now: i64) -> Option<Value> {
        let meta = self.store.get_token(kind, token)?;
        let expired = meta
            .get("expires_at")
            .and_then(Value::as_i64)
            .is_some_and(|exp| exp < now);
        if expired {
            let _ = self.store.delete_token(kind, token);
            return None;
        }
        Some(meta)
    }

    fn issue_tokens(&self, grant: Grant<'_>, now: i64) -> io::Result<TokenResponse> {
        let access_token = generate_token()?;
        let refresh_token = generate_token()?;
        let subject = grant
            .subject
            .map_or(Value::Null, |s| Value::from(s.as_str()));
        let scopes: Vec<Value> = grant
            .scopes
            .iter()
            .map(|s| Value::from(s.as_str()))
            .collect();
        let resource = grant.resource.as_deref().map_or(Value::Null, Value::from);
        self.store.put_token(
            TokenKind::Access,
            &access_token,
            json!({
                "client_id": grant.client_id,
                "subject": subject.clone(),
                "scopes": scopes.clone(),
                "expires_at": now + ACCESS_TOKEN_TTL,
                "resource": resource,
            }),
        )?;
        self.store.put_token(
            TokenKind::Refresh,
            &refresh_token,
            json!({
                "client_id": grant.client_id,
                "subject": subject,
                "scopes": scopes,
                "expires_at": now + REFRESH_TOKEN_TTL,
            }),
        )?;
        // Both halves are on disk before the pair is handed back, so a client
        // never holds a token the server will not recognise.
        Ok(TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in: Some(ACCESS_TOKEN_TTL),
            scope: (!grant.scopes.is_empty()).then(|| grant.scopes.join(" ")),
            refresh_token: Some(refresh_token),
        })
    }
}

/// What a token pair is issued for.
struct Grant<'a> {
    client_id: &'a str,
    subject: Option<&'a Subject>,
    scopes: Vec<String>,
    resource: Option<String>,
}

fn validate_metadata(metadata: &ClientMetadata) -> Result<(), RegistrationError> {
    let invalid = |description: &str| RegistrationError {
        error: "invalid_client_metadata",
        error_description: description.to_string(),
    };
    if metadata.redirect_uris.is_empty() {
        return Err(invalid("redirect_uris must contain at least one URI"));
    }
    let has_grant = |g: &str| metadata.grant_types.iter().any(|x| x == g);
    if !has_grant("authorization_code") || !has_grant("refresh_token") {
        return Err(invalid(
            "grant_types must be authorization_code and refresh_token",
        ));
    }
    if !metadata.response_types.iter().any(|r| r == "code") {
        return Err(invalid(
            "response_types must include 'code' for authorization_code grant",
        ));
    }
    Ok(())
}

fn subject_of(meta: &Value) -> Option<Subject> {
    meta.get("subject")
        .and_then(Value::as_str)
        .map(Subject::new)
}

fn scopes_of(meta: &Value) -> Vec<String> {
    meta.get("scopes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "owner-token";

    fn provider() -> (Provider, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("oauth.json"));
        let verify: Verify = Box::new(|t| (t == OWNER).then(|| Subject::new("owner")));
        (Provider::new(verify, store), dir)
    }

    fn params(challenge: &str) -> AuthorizationParams {
        AuthorizationParams {
            state: Some("st4te".to_string()),
            scopes: None,
            code_challenge: challenge.to_string(),
            redirect_uri: "https://claude.ai/cb".to_string(),
            redirect_uri_provided_explicitly: true,
            resource: None,
        }
    }

    fn metadata(redirect_uris: &[&str], grants: &[&str], responses: &[&str]) -> ClientMetadata {
        let strings = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect();
        ClientMetadata {
            redirect_uris: strings(redirect_uris),
            token_endpoint_auth_method: Some("client_secret_post".to_string()),
            grant_types: strings(grants),
            response_types: strings(responses),
            scope: None,
            client_name: Some("claude.ai".to_string()),
            extra: Default::default(),
        }
    }

    const GRANTS: &[&str] = &["authorization_code", "refresh_token"];

    fn register(provider: &Provider) -> String {
        let info = provider
            .register_client(metadata(&["https://claude.ai/cb"], GRANTS, &["code"]))
            .unwrap();
        info.client_id.unwrap()
    }

    /// Authorize, approve as `owner`, and return the issued code.
    fn approved_code(provider: &Provider, client_id: &str, now: i64) -> String {
        let path = provider
            .authorize(client_id, params("challenge"), now)
            .unwrap();
        let txn = path.strip_prefix("/consent?txn=").unwrap();
        let subject = provider.verify(OWNER);
        match provider.decide_consent(txn, subject, now).unwrap() {
            ConsentOutcome::Approved { code, .. } => code,
            _ => panic!("expected Approved"),
        }
    }

    #[test]
    fn registering_a_client_always_forces_none_auth_and_no_secret() {
        let (provider, _dir) = provider();
        let info = provider
            .register_client(metadata(&["https://claude.ai/cb"], GRANTS, &["code"]))
            .unwrap();
        assert_eq!(info.token_endpoint_auth_method, "none");
        assert!(info.client_secret.is_none());
        let stored = provider.get_client(info.client_id.as_deref().unwrap());
        assert_eq!(stored.unwrap().client_name.as_deref(), Some("claude.ai"));
    }

    #[test]
    fn registration_rejects_missing_redirect_uris_and_wrong_grant_or_response_types() {
        let (provider, _dir) = provider();
        let cb = &["https://a.example/cb"];
        for bad in [
            metadata(&[], GRANTS, &["code"]),
            metadata(cb, &["authorization_code"], &["code"]),
            metadata(cb, GRANTS, &["token"]),
        ] {
            assert!(matches!(
                provider.register_client(bad),
                Err(RegisterError::Invalid(_))
            ));
        }
    }

    #[test]
    fn the_full_flow_issues_tokens_that_carry_the_approving_subject() {
        let (provider, _dir) = provider();
        let client_id = register(&provider);
        let now = now_unix();

        let path = provider
            .authorize(&client_id, params("challenge-abc"), now)
            .unwrap();
        let txn = path.strip_prefix("/consent?txn=").unwrap();
        let view = provider.pending_consent(txn, now).unwrap();
        assert_eq!(view.client_name.as_deref(), Some("claude.ai"));

        let outcome = provider
            .decide_consent(txn, provider.verify(OWNER), now)
            .unwrap();
        let ConsentOutcome::Approved { code, state, .. } = outcome else {
            panic!("expected Approved");
        };
        assert_eq!(state.as_deref(), Some("st4te"));
        // The transaction is single-use.
        assert!(matches!(
            provider.decide_consent(txn, provider.verify(OWNER), now),
            Ok(ConsentOutcome::Expired)
        ));

        let loaded = provider.load_authorization_code(&client_id, &code).unwrap();
        assert_eq!(loaded.code_challenge, "challenge-abc");
        assert!(provider.load_authorization_code("other", &code).is_none());

        let tokens = provider
            .exchange_authorization_code(&code, now)
            .unwrap()
            .unwrap();
        assert_eq!(tokens.expires_in, Some(ACCESS_TOKEN_TTL));
        assert!(provider
            .load_authorization_code(&client_id, &code)
            .is_none());
        assert_eq!(
            provider.subject_of(&tokens.access_token, now),
            Some(Subject::new("owner"))
        );
    }

    #[test]
    fn a_wrong_token_and_a_deny_are_the_same_denial() {
        let (provider, _dir) = provider();
        let client_id = register(&provider);
        let now = now_unix();
        assert!(provider.verify("not-the-owner-token").is_none());

        let path = provider
            .authorize(&client_id, params("challenge"), now)
            .unwrap();
        let txn = path.strip_prefix("/consent?txn=").unwrap();
        let outcome = provider.decide_consent(txn, None, now).unwrap();
        assert!(matches!(outcome, ConsentOutcome::Denied { .. }));
    }

    #[test]
    fn an_expired_consent_transaction_reports_expired() {
        let (provider, _dir) = provider();
        let client_id = register(&provider);
        let now = now_unix();
        let path = provider
            .authorize(&client_id, params("challenge"), now)
            .unwrap();
        let txn = path.strip_prefix("/consent?txn=").unwrap();

        let later = now + CONSENT_TTL + 1;
        assert!(provider.pending_consent(txn, later).is_none());
        assert!(matches!(
            provider.decide_consent(txn, provider.verify(OWNER), later),
            Ok(ConsentOutcome::Expired)
        ));
    }

    #[test]
    fn refresh_rotates_keeps_the_subject_and_access_tokens_expire() {
        let (provider, _dir) = provider();
        let client_id = register(&provider);
        let now = now_unix();
        let code = approved_code(&provider, &client_id, now);
        let tokens = provider
            .exchange_authorization_code(&code, now)
            .unwrap()
            .unwrap();
        let old_refresh = tokens.refresh_token.unwrap();

        let view = provider
            .load_refresh_token(&client_id, &old_refresh, now)
            .unwrap();
        assert!(provider
            .load_refresh_token("other", &old_refresh, now)
            .is_none());
        let rotated = provider
            .exchange_refresh_token(&view, &old_refresh, vec![], now)
            .unwrap();
        assert_ne!(rotated.access_token, tokens.access_token);
        assert!(provider
            .load_refresh_token(&client_id, &old_refresh, now)
            .is_none());
        assert_eq!(
            provider.subject_of(&rotated.access_token, now),
            Some(Subject::new("owner"))
        );

        assert!(provider
            .load_access_token(&tokens.access_token, now)
            .is_some());
        let later = now + ACCESS_TOKEN_TTL + 1;
        assert!(provider
            .load_access_token(&tokens.access_token, later)
            .is_none());
    }

    #[test]
    fn the_servers_own_token_is_not_an_access_token() {
        let (provider, _dir) = provider();
        assert!(provider.load_access_token(OWNER, now_unix()).is_none());
        assert_eq!(provider.verify(OWNER), Some(Subject::new("owner")));
    }

    #[test]
    fn a_grant_written_before_subjects_existed_still_loads_without_one() {
        let (provider, _dir) = provider();
        provider
            .store()
            .put_token(
                TokenKind::Access,
                "legacy",
                json!({"client_id": "c1", "scopes": [], "expires_at": i64::MAX}),
            )
            .unwrap();
        let view = provider.load_access_token("legacy", now_unix()).unwrap();
        assert_eq!(view.client_id, "c1");
        assert_eq!(view.subject, None);
        assert_eq!(provider.subject_of("legacy", now_unix()), None);
    }

    #[test]
    fn revoking_a_clients_tokens_kills_access_and_refresh() {
        let (provider, _dir) = provider();
        let client_id = register(&provider);
        let now = now_unix();
        let code = approved_code(&provider, &client_id, now);
        let tokens = provider
            .exchange_authorization_code(&code, now)
            .unwrap()
            .unwrap();

        let counts = provider.revoke_tokens_for_client(&client_id).unwrap();
        assert_eq!((counts.access_tokens, counts.refresh_tokens), (1, 1));
        assert!(provider
            .load_access_token(&tokens.access_token, now)
            .is_none());
        let refresh = tokens.refresh_token.unwrap();
        assert!(provider
            .load_refresh_token(&client_id, &refresh, now)
            .is_none());
    }
}
