//! Bearer-token accounts.
//!
//! Two modes, chosen by whether an accounts file is supplied:
//!
//! - **Open** (no file): every request is the single `local` account. This is
//!   the current single-user behaviour and is only sensible on loopback.
//! - **Token** (`--accounts <file>`): each line is `account:token`; requests must
//!   carry `Authorization: Bearer <token>`. Tokens are supplied by the operator
//!   (the app has no cryptographic RNG dependency) and must be at least
//!   [`MIN_TOKEN_LEN`] characters.
//!
//! This is the local seam a hosted identity provider (Google sign-in, as the
//! reference product uses) would replace: only [`Accounts::authenticate`] changes.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::store::{AccountId, StoreError};

pub const MIN_TOKEN_LEN: usize = 16;
const OPEN_ACCOUNT: &str = "local";

#[derive(Debug)]
pub enum AuthError {
    /// No `Authorization: Bearer` header on a token-mode server.
    Missing,
    /// A bearer token that matches no account.
    Invalid,
    /// The accounts file could not be read or is malformed.
    Config(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => {
                f.write_str("authentication required: send Authorization: Bearer <token>")
            }
            Self::Invalid => f.write_str("invalid token"),
            Self::Config(m) => write!(f, "accounts file: {m}"),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Debug, Clone)]
pub struct Accounts {
    /// Empty ⇒ open mode.
    tokens: Vec<(AccountId, String)>,
}

impl Accounts {
    /// Single-user mode: no authentication, one `local` account.
    pub fn open() -> Self {
        Self { tokens: Vec::new() }
    }

    pub fn is_open(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Whether `account` has a token (always `false` in open mode).
    pub fn has_account(&self, account: &AccountId) -> bool {
        self.tokens.iter().any(|(a, _)| a == account)
    }

    pub fn from_file(path: &Path) -> Result<Self, AuthError> {
        let text = fs::read_to_string(path)
            .map_err(|e| AuthError::Config(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// Parse `account:token` lines; blank lines and `#` comments are ignored.
    pub fn parse(text: &str) -> Result<Self, AuthError> {
        let mut tokens: Vec<(AccountId, String)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |m: String| AuthError::Config(format!("line {}: {m}", i + 1));
            let (name, token) = line
                .split_once(':')
                .ok_or_else(|| at("expected account:token".into()))?;
            let account = AccountId::new(name.trim()).map_err(|e: StoreError| at(e.to_string()))?;
            let token = token.trim();
            if token.len() < MIN_TOKEN_LEN {
                return Err(at(format!(
                    "token for {account} shorter than {MIN_TOKEN_LEN} characters"
                )));
            }
            if tokens.iter().any(|(_, t)| t == token) {
                return Err(at("duplicate token".into()));
            }
            tokens.push((account, token.to_string()));
        }
        if tokens.is_empty() {
            return Err(AuthError::Config(
                "no accounts defined (omit --accounts for single-user mode)".into(),
            ));
        }
        Ok(Self { tokens })
    }

    /// Resolve the request's `Authorization` header to an account.
    pub fn authenticate(&self, authorization: Option<&str>) -> Result<AccountId, AuthError> {
        if self.is_open() {
            return Ok(AccountId::new(OPEN_ACCOUNT).expect("constant is a valid account id"));
        }
        let token = authorization
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(AuthError::Missing)?;
        self.tokens
            .iter()
            .find(|(_, t)| constant_time_eq(t.as_bytes(), token.as_bytes()))
            .map(|(a, _)| a.clone())
            .ok_or(AuthError::Invalid)
    }
}

/// Length-independent-of-content comparison so token checks don't leak a
/// matching prefix through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1: &str = "aaaaaaaaaaaaaaaa1";
    const T2: &str = "bbbbbbbbbbbbbbbb2";

    fn accounts() -> Accounts {
        Accounts::parse(&format!("# team\nalice:{T1}\n\nbob : {T2}\n")).unwrap()
    }

    #[test]
    fn open_mode_is_the_local_account() {
        let a = Accounts::open();
        assert!(a.is_open());
        assert_eq!(a.authenticate(None).unwrap().as_str(), "local");
    }

    #[test]
    fn token_maps_to_its_account() {
        let a = accounts();
        assert_eq!(
            a.authenticate(Some(&format!("Bearer {T2}")))
                .unwrap()
                .as_str(),
            "bob"
        );
        assert_eq!(
            a.authenticate(Some(&format!("Bearer {T1}")))
                .unwrap()
                .as_str(),
            "alice"
        );
    }

    #[test]
    fn missing_or_wrong_credentials_are_rejected() {
        let a = accounts();
        assert!(matches!(a.authenticate(None), Err(AuthError::Missing)));
        assert!(matches!(
            a.authenticate(Some("Basic abc")),
            Err(AuthError::Missing)
        ));
        assert!(matches!(
            a.authenticate(Some("Bearer ")),
            Err(AuthError::Missing)
        ));
        assert!(matches!(
            a.authenticate(Some("Bearer nope")),
            Err(AuthError::Invalid)
        ));
    }

    #[test]
    fn config_errors_are_specific() {
        let cases = [
            ("alice", "expected account:token"),
            ("alice:short", "shorter than"),
            ("../x:aaaaaaaaaaaaaaaa1", "invalid account"),
            ("", "no accounts"),
        ];
        for (text, needle) in cases {
            let e = Accounts::parse(text).unwrap_err().to_string();
            assert!(e.contains(needle), "{text:?} -> {e}");
        }
        let dup = format!("a:{T1}\nb:{T1}");
        assert!(Accounts::parse(&dup)
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }
}
