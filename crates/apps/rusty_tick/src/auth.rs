//! Who is asking (ADR-0002, step 3): turns an `Authorization` header into a
//! user, in one of two modes.
//!
//! - **Single user**: one shared token from `RUSTY_TICK_TOKEN`, one fixed
//!   user, and the data directory itself is that user's store. This is how
//!   `rusty_tick` has always run.
//! - **Multi user**: tokens named by user (`<user key>.<secret>`) checked
//!   against `users.json` ([`crate::users`]), which is re-read as it changes.
//!
//! A refusal never says why. It carries the user key the caller *claimed*, if
//! the token parsed, so the server can log it; the secret is never kept.

use crate::api::constant_time_eq;
use crate::pool::UserKey;
use crate::users::{RegistryFile, Token};

/// Shortest accepted single-user token: a guessable token defeats the check.
pub const MIN_TOKEN_LEN: usize = 16;

/// The one user of single-user mode.
const SINGLE_USER: &str = "default";

/// A refused credential. `claimed` is the user key it named, when it named
/// one, and is for logging only.
#[derive(Debug, PartialEq, Eq)]
pub struct Denied {
    pub claimed: Option<UserKey>,
}

pub enum Authenticator {
    Single {
        token: String,
        user: UserKey,
    },
    Multi {
        users: RegistryFile,
        /// The registry error last reported, so a file that stays broken is
        /// reported once, not on every request.
        reported: Option<String>,
    },
}

impl Authenticator {
    /// One shared token.
    ///
    /// # Errors
    ///
    /// If `token` is shorter than [`MIN_TOKEN_LEN`].
    pub fn single(token: String) -> Result<Self, String> {
        if token.len() < MIN_TOKEN_LEN {
            return Err(format!(
                "the API token must be at least {MIN_TOKEN_LEN} characters"
            ));
        }
        let user = UserKey::parse(SINGLE_USER).map_err(|e| e.to_string())?;
        Ok(Self::Single { token, user })
    }

    /// Per-user tokens from `users`.
    pub fn multi(users: RegistryFile) -> Self {
        Self::Multi {
            users,
            reported: None,
        }
    }

    /// The user behind an `Authorization` header value.
    ///
    /// # Errors
    ///
    /// [`Denied`], for a missing header, a scheme other than `Bearer`, and a
    /// token that is unknown, revoked, disabled or wrong alike.
    pub fn authenticate(&mut self, header: Option<&str>) -> Result<UserKey, Denied> {
        let presented = header.and_then(|h| h.strip_prefix("Bearer "));
        match self {
            Self::Single { token, user } => {
                let ok =
                    presented.is_some_and(|p| constant_time_eq(p.as_bytes(), token.as_bytes()));
                if ok {
                    Ok(user.clone())
                } else {
                    Err(Denied { claimed: None })
                }
            }
            Self::Multi { users, reported } => {
                let registry = users.current();
                let found = registry.authenticate(presented.unwrap_or(""));
                let error = users.last_error().map(str::to_string);
                if error != *reported {
                    if let Some(message) = &error {
                        eprintln!(
                            "rusty_tick: users file refused, using the last good one: {message}"
                        );
                    }
                    *reported = error;
                }
                found.ok_or_else(|| Denied {
                    claimed: presented.and_then(Token::parse).map(|token| token.user),
                })
            }
        }
    }
}
