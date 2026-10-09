//! Request authentication: bearer tokens, browser sessions, or open mode.
//!
//! [`Authn`] is the one place that decides *who is asking*. It combines
//!
//! 1. **Bearer tokens** from the accounts file ([`Accounts`]) — for scripts;
//! 2. **Session cookies** issued after a browser login, through a
//!    [`LoginProvider`] (Google sign-in, in the `oidc` feature) — for people;
//! 3. **Open mode** — no credentials configured at all, one `local` account.
//!
//! Open mode applies only when *neither* tokens nor a login provider exist:
//! turning on Google sign-in must never leave the API open to anonymous callers.
//!
//! Cookie-authenticated writes get a CSRF check: a request that carries an
//! `Origin` header must name this server's own host. (Session cookies are also
//! `SameSite=Lax`, which browsers already refuse to send on cross-site POSTs;
//! this is the second layer.) Bearer-token requests carry no ambient credential,
//! so they need no such check.

use std::fmt;

use crate::auth::{Accounts, AuthError};
use crate::store::AccountId;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "rleval_session";

/// What the router needs to know about a request to authenticate it.
pub struct Credentials<'a> {
    pub method: &'a str,
    pub authorization: Option<&'a str>,
    pub cookie: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub host: Option<&'a str>,
}

/// Why a request was not authenticated.
#[derive(Debug)]
pub enum AuthnError {
    /// 401: nothing usable was presented.
    Unauthorized(AuthError),
    /// 403: a valid session, but a cross-origin write.
    CrossOrigin,
}

impl fmt::Display for AuthnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(e) => e.fmt(f),
            Self::CrossOrigin => f.write_str("cross-origin request refused"),
        }
    }
}

/// A completed browser login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIn {
    pub account: AccountId,
    /// The opaque session token to put in the cookie.
    pub token: String,
    pub max_age_secs: u64,
}

/// Why a login step failed; the message is safe to show the user.
#[derive(Debug)]
pub struct LoginError(pub String);

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoginError {}

/// A browser sign-in mechanism that ends in a server-side session.
pub trait LoginProvider: Send + Sync {
    /// Start a login: where to redirect the browser.
    fn begin(&self) -> Result<String, LoginError>;
    /// Finish a login from the callback's query string; creates the session.
    fn complete(&self, callback_query: &str) -> Result<SignedIn, LoginError>;
    /// The account behind a session token, if it is live.
    fn session_account(&self, token: &str) -> Option<AccountId>;
    /// Forget a session.
    fn end_session(&self, token: &str);
    /// Whether cookies should carry the `Secure` attribute (an https deployment).
    fn secure_cookies(&self) -> bool;
    /// Whether `account` can sign in through this provider.
    fn knows_account(&self, account: &AccountId) -> bool;
}

pub struct Authn {
    accounts: Accounts,
    login: Option<Box<dyn LoginProvider>>,
}

impl Authn {
    pub fn new(accounts: Accounts, login: Option<Box<dyn LoginProvider>>) -> Self {
        Self { accounts, login }
    }

    /// No credentials of any kind are configured: everyone is `local`.
    pub fn is_open(&self) -> bool {
        self.accounts.is_open() && self.login.is_none()
    }

    pub fn login(&self) -> Option<&dyn LoginProvider> {
        self.login.as_deref()
    }

    /// Whether `account` can authenticate at all (has a token or can sign in).
    pub fn has_account(&self, account: &AccountId) -> bool {
        self.accounts.has_account(account)
            || self
                .login
                .as_ref()
                .is_some_and(|l| l.knows_account(account))
    }

    pub fn authenticate(&self, c: &Credentials<'_>) -> Result<AccountId, AuthnError> {
        // A bearer token is an explicit credential: if one is sent it decides,
        // and a bad one is not silently rescued by a cookie. Tokens only exist
        // when an accounts file was given — `Accounts` in open mode would accept
        // any header as the `local` user, which must never happen once sign-in
        // has closed the API to anonymous callers.
        if c.authorization.is_some_and(|h| h.starts_with("Bearer ")) {
            if self.accounts.is_open() {
                return Err(AuthnError::Unauthorized(AuthError::Invalid));
            }
            return self
                .accounts
                .authenticate(c.authorization)
                .map_err(AuthnError::Unauthorized);
        }
        if let Some(account) = self.session_from_cookie(c.cookie) {
            return if is_write(c.method) && !same_origin(c.origin, c.host) {
                Err(AuthnError::CrossOrigin)
            } else {
                Ok(account)
            };
        }
        if self.is_open() {
            return self
                .accounts
                .authenticate(None)
                .map_err(AuthnError::Unauthorized);
        }
        Err(AuthnError::Unauthorized(AuthError::Missing))
    }

    /// The live session (token and account) named by a `Cookie` header.
    pub fn session_token<'a>(&self, cookie: Option<&'a str>) -> Option<&'a str> {
        let token = cookie_value(cookie?, SESSION_COOKIE)?;
        self.login.as_ref()?.session_account(token)?;
        Some(token)
    }

    fn session_from_cookie(&self, cookie: Option<&str>) -> Option<AccountId> {
        let token = cookie_value(cookie?, SESSION_COOKIE)?;
        self.login.as_ref()?.session_account(token)
    }
}

fn is_write(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS")
}

/// An absent `Origin` is allowed (non-browser clients and same-origin GETs omit
/// it); a present one must be exactly this server's host.
fn same_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let Some(host) = host else { return false };
    let authority = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    authority.eq_ignore_ascii_case(host)
}

/// The value of cookie `name` in a `Cookie:` header.
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// A `Set-Cookie` value establishing the session.
pub fn session_cookie(token: &str, max_age_secs: u64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure}"
    )
}

/// A `Set-Cookie` value that clears the session.
pub fn clearing_cookie(secure: bool) -> String {
    session_cookie("", 0, secure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A provider whose sessions are just a token → account table.
    struct Fake(Mutex<HashMap<String, AccountId>>);

    impl Fake {
        fn with(token: &str, account: &str) -> Box<dyn LoginProvider> {
            let mut m = HashMap::new();
            m.insert(token.to_string(), AccountId::new(account).unwrap());
            Box::new(Self(Mutex::new(m)))
        }
    }

    impl LoginProvider for Fake {
        fn begin(&self) -> Result<String, LoginError> {
            Ok("https://idp.example/auth".into())
        }
        fn complete(&self, _: &str) -> Result<SignedIn, LoginError> {
            Err(LoginError("unused".into()))
        }
        fn session_account(&self, token: &str) -> Option<AccountId> {
            self.0.lock().unwrap().get(token).cloned()
        }
        fn end_session(&self, token: &str) {
            self.0.lock().unwrap().remove(token);
        }
        fn secure_cookies(&self) -> bool {
            true
        }
        fn knows_account(&self, a: &AccountId) -> bool {
            a.as_str() == "carol"
        }
    }

    const TOKEN: &str = "0123456789abcdefXYZ";

    fn creds<'a>(
        method: &'a str,
        authorization: Option<&'a str>,
        cookie: Option<&'a str>,
        origin: Option<&'a str>,
    ) -> Credentials<'a> {
        Credentials {
            method,
            authorization,
            cookie,
            origin,
            host: Some("rl.example"),
        }
    }

    fn both() -> Authn {
        let accounts = Accounts::parse(&format!("alice:{TOKEN}")).unwrap();
        Authn::new(accounts, Some(Fake::with("sess1", "carol")))
    }

    #[test]
    fn open_only_when_no_tokens_and_no_login() {
        let open = Authn::new(Accounts::open(), None);
        assert!(open.is_open());
        assert_eq!(
            open.authenticate(&creds("GET", None, None, None))
                .unwrap()
                .as_str(),
            "local"
        );
        // Turning on sign-in must close the API to anonymous callers.
        let login_only = Authn::new(Accounts::open(), Some(Fake::with("s", "carol")));
        assert!(!login_only.is_open());
        assert!(matches!(
            login_only.authenticate(&creds("GET", None, None, None)),
            Err(AuthnError::Unauthorized(_))
        ));
    }

    #[test]
    fn bearer_token_and_session_cookie_both_authenticate() {
        let a = both();
        let bearer = format!("Bearer {TOKEN}");
        assert_eq!(
            a.authenticate(&creds("POST", Some(&bearer), None, None))
                .unwrap()
                .as_str(),
            "alice"
        );
        let cookie = format!("other=1; {SESSION_COOKIE}=sess1");
        assert_eq!(
            a.authenticate(&creds("GET", None, Some(&cookie), None))
                .unwrap()
                .as_str(),
            "carol"
        );
    }

    #[test]
    fn with_sign_in_but_no_token_file_a_bearer_header_is_not_a_login() {
        // Regression: `Accounts` in open mode says yes to everyone; a junk
        // `Authorization: Bearer x` must not become the `local` account.
        let login_only = Authn::new(Accounts::open(), Some(Fake::with("s", "carol")));
        let r = login_only.authenticate(&creds("GET", Some("Bearer anything"), None, None));
        assert!(matches!(r, Err(AuthnError::Unauthorized(_))), "{r:?}");
    }

    #[test]
    fn a_bad_bearer_is_not_rescued_by_a_valid_cookie() {
        let cookie = format!("{SESSION_COOKIE}=sess1");
        let r = both().authenticate(&creds("GET", Some("Bearer nope"), Some(&cookie), None));
        assert!(matches!(r, Err(AuthnError::Unauthorized(_))));
    }

    #[test]
    fn unknown_or_ended_sessions_are_unauthorized() {
        let a = both();
        let cookie = format!("{SESSION_COOKIE}=forged");
        assert!(matches!(
            a.authenticate(&creds("GET", None, Some(&cookie), None)),
            Err(AuthnError::Unauthorized(_))
        ));
        let real = format!("{SESSION_COOKIE}=sess1");
        a.login().unwrap().end_session("sess1");
        assert!(a
            .authenticate(&creds("GET", None, Some(&real), None))
            .is_err());
    }

    #[test]
    fn cookie_writes_must_be_same_origin() {
        let a = both();
        let cookie = format!("{SESSION_COOKIE}=sess1");
        let post = |origin| a.authenticate(&creds("POST", None, Some(&cookie), origin));
        assert!(post(None).is_ok(), "no Origin: non-browser or same-origin");
        assert!(post(Some("https://rl.example")).is_ok());
        assert!(matches!(
            post(Some("https://evil.example")),
            Err(AuthnError::CrossOrigin)
        ));
        // Reads are not subject to the check.
        assert!(a
            .authenticate(&creds(
                "GET",
                None,
                Some(&cookie),
                Some("https://evil.example")
            ))
            .is_ok());
    }

    #[test]
    fn bearer_writes_skip_the_origin_check() {
        let bearer = format!("Bearer {TOKEN}");
        let r = both().authenticate(&creds(
            "POST",
            Some(&bearer),
            None,
            Some("https://elsewhere"),
        ));
        assert!(r.is_ok(), "no ambient credential, so no CSRF");
    }

    #[test]
    fn has_account_covers_tokens_and_login() {
        let a = both();
        let id = |n| AccountId::new(n).unwrap();
        assert!(a.has_account(&id("alice")) && a.has_account(&id("carol")));
        assert!(!a.has_account(&id("mallory")));
    }

    #[test]
    fn cookies_are_parsed_and_built() {
        assert_eq!(
            cookie_value("a=1; rleval_session=xyz; b=2", SESSION_COOKIE),
            Some("xyz")
        );
        assert_eq!(cookie_value("a=1", SESSION_COOKIE), None);
        let c = session_cookie("tok", 60, true);
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax") && c.contains("Secure"));
        assert!(!session_cookie("tok", 60, false).contains("Secure"));
        assert!(clearing_cookie(false).contains("Max-Age=0"));
    }
}
