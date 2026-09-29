//! The user registry and the token check (ADR-0002, step 2).
//!
//! A token is `<user key>.<secret>`: the key names the user, so one record is
//! looked up instead of every token being tried, and the secret is 32 random
//! bytes in URL-safe base64. Only a SHA-256 digest of the secret is kept, in
//! `users.json`, which is enough for a secret that cannot be guessed. Nothing
//! here does I/O beyond that one file, and nothing is wired to the API yet.
//!
//! # File
//!
//! ```json
//! {"version":1,"users":[{"key":"alice","disabled":false,"tokens":[
//!   {"id":"ab12cd34","label":"phone","sha256":"<64 hex>","createdMs":1}]}]}
//! ```
//!
//! # Checking a token
//!
//! [`Registry::authenticate`] gives the same answer, `None`, and does the same
//! work for a malformed token, an unknown user, a disabled user, a revoked
//! token and a wrong secret, so a caller learns nothing about which users
//! exist.

use crate::api::constant_time_eq;
use crate::pool::UserKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Random bytes in a token's secret.
pub const SECRET_BYTES: usize = 32;
/// The secret's length once encoded: 32 bytes, unpadded URL-safe base64.
const SECRET_CHARS: usize = 43;
/// The file format this build writes and reads.
pub const REGISTRY_VERSION: u32 = 1;
/// How often a running server looks at the file for changes.
pub const DEFAULT_RELOAD_INTERVAL: Duration = Duration::from_secs(1);
/// The longest token label, in bytes.
pub const MAX_LABEL_BYTES: usize = 64;
/// Hex characters of a digest kept as a token's id.
const ID_CHARS: usize = 8;
/// Attempts at a fresh token id before giving up (a collision needs a
/// 32-bit coincidence, so more than one attempt is already unlikely).
const ID_ATTEMPTS: usize = 8;

/// What a token is checked against when no stored digest applies.
const DUMMY_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const DUMMY_DIGEST: Digest32 = [0; 32];

/// A SHA-256 digest.
pub type Digest32 = [u8; 32];

/// The digest of a token's secret: the one place SHA-256 is named, so the
/// implementation can change without touching anything else (ADR-0002). It
/// is `rusty_rsa`'s, the monorepo's own.
pub fn digest(secret: &str) -> Digest32 {
    rusty_rsa::sha256(secret.as_bytes())
}

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error("users file: {0}")]
    Format(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no random bytes: {0}")]
    Random(String),
    #[error("user {0:?} already exists")]
    UserExists(String),
    #[error("user {0:?} not found")]
    UnknownUser(String),
    #[error("user {user:?} has no token {id:?}")]
    UnknownToken { user: String, id: String },
    #[error("token label must be at most {MAX_LABEL_BYTES} bytes, with no control characters")]
    InvalidLabel,
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> UsersError + '_ {
    move |source| UsersError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// A credential as presented: the part of `Authorization: Bearer ...` after
/// `Bearer `.
#[derive(Debug, PartialEq, Eq)]
pub struct Token<'a> {
    pub user: UserKey,
    pub secret: &'a str,
}

impl<'a> Token<'a> {
    /// `<user key>.<secret>`, or `None` for anything else. The split is at the
    /// first `.`, which a key cannot hold; the secret must be exactly the
    /// length and alphabet a minted one has.
    pub fn parse(credential: &'a str) -> Option<Self> {
        let (key, secret) = credential.split_once('.')?;
        let url_safe = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
        if secret.len() != SECRET_CHARS || !secret.bytes().all(url_safe) {
            return None;
        }
        Some(Self {
            user: UserKey::parse(key).ok()?,
            secret,
        })
    }
}

/// One token of a user: what is kept of it, never the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    /// The first hex characters of the digest: names the token for `revoke`
    /// and in logs without revealing anything usable.
    pub id: String,
    pub label: String,
    pub sha256: Digest32,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub key: UserKey,
    pub disabled: bool,
    pub tokens: Vec<TokenRecord>,
}

/// Every user and the digests of their tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    users: BTreeMap<String, UserRecord>,
}

impl Registry {
    pub fn user(&self, key: &UserKey) -> Option<&UserRecord> {
        self.users.get(key.as_str())
    }

    /// Every user, ordered by key.
    pub fn users(&self) -> impl Iterator<Item = &UserRecord> {
        self.users.values()
    }

    /// # Errors
    ///
    /// [`UsersError::UserExists`] if `key` is already registered.
    pub fn add_user(&mut self, key: &UserKey) -> Result<(), UsersError> {
        if self.users.contains_key(key.as_str()) {
            return Err(UsersError::UserExists(key.as_str().to_string()));
        }
        self.users.insert(
            key.as_str().to_string(),
            UserRecord {
                key: key.clone(),
                disabled: false,
                tokens: Vec::new(),
            },
        );
        Ok(())
    }

    /// Mint a token for `user` and keep its digest. Returns the full token,
    /// which is not stored and cannot be shown again.
    ///
    /// # Errors
    ///
    /// [`UsersError::UnknownUser`], [`UsersError::InvalidLabel`], or
    /// [`UsersError::Random`] if the OS could not supply random bytes.
    pub fn add_token(
        &mut self,
        user: &UserKey,
        label: &str,
        now_ms: i64,
    ) -> Result<String, UsersError> {
        check_label(label)?;
        let record = self
            .users
            .get_mut(user.as_str())
            .ok_or_else(|| UsersError::UnknownUser(user.as_str().to_string()))?;
        for _ in 0..ID_ATTEMPTS {
            let secret = mint_secret()?;
            let sha256 = digest(&secret);
            let id = hex(&sha256)[..ID_CHARS].to_string();
            if record.tokens.iter().any(|t| t.id == id) {
                continue;
            }
            record.tokens.push(TokenRecord {
                id,
                label: label.to_string(),
                sha256,
                created_ms: now_ms,
            });
            return Ok(format!("{}.{secret}", user.as_str()));
        }
        Err(UsersError::Random(
            "could not mint a token with an unused id".to_string(),
        ))
    }

    /// # Errors
    ///
    /// [`UsersError::UnknownUser`] or [`UsersError::UnknownToken`].
    pub fn revoke(&mut self, user: &UserKey, token_id: &str) -> Result<(), UsersError> {
        let record = self
            .users
            .get_mut(user.as_str())
            .ok_or_else(|| UsersError::UnknownUser(user.as_str().to_string()))?;
        let at = record
            .tokens
            .iter()
            .position(|t| t.id == token_id)
            .ok_or_else(|| UsersError::UnknownToken {
                user: user.as_str().to_string(),
                id: token_id.to_string(),
            })?;
        record.tokens.remove(at);
        Ok(())
    }

    /// Block or allow every token of `user` without touching them.
    ///
    /// # Errors
    ///
    /// [`UsersError::UnknownUser`].
    pub fn set_disabled(&mut self, user: &UserKey, disabled: bool) -> Result<(), UsersError> {
        let record = self
            .users
            .get_mut(user.as_str())
            .ok_or_else(|| UsersError::UnknownUser(user.as_str().to_string()))?;
        record.disabled = disabled;
        Ok(())
    }

    /// The user a credential belongs to, if it is a live token of an enabled
    /// user. Every failure looks the same and costs the same: the digest is
    /// always taken, and always compared at least once.
    pub fn authenticate(&self, credential: &str) -> Option<UserKey> {
        let token = Token::parse(credential);
        let presented = digest(token.as_ref().map_or(DUMMY_SECRET, |t| t.secret));
        let record = token
            .as_ref()
            .and_then(|t| self.users.get(t.user.as_str()))
            .filter(|u| !u.disabled);
        let mut matched = false;
        for stored in record.map_or(&[][..], |u| &u.tokens[..]) {
            matched |= constant_time_eq(&presented, &stored.sha256);
        }
        // With nothing to compare against, compare anyway.
        let _ = constant_time_eq(&presented, &DUMMY_DIGEST);
        record.filter(|_| matched).map(|u| u.key.clone())
    }

    /// The registry as `users.json` holds it.
    pub fn to_json(&self) -> String {
        let file = FileDto {
            version: REGISTRY_VERSION,
            users: self
                .users
                .values()
                .map(|u| UserDto {
                    key: u.key.as_str().to_string(),
                    disabled: u.disabled,
                    tokens: u
                        .tokens
                        .iter()
                        .map(|t| TokenDto {
                            id: t.id.clone(),
                            label: t.label.clone(),
                            sha256: hex(&t.sha256),
                            created_ms: t.created_ms,
                        })
                        .collect(),
                })
                .collect(),
        };
        rusty_json::to_string_pretty(&file).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse `users.json`, refusing anything this build does not fully
    /// understand: another version, a bad key, a duplicate user or token id,
    /// a digest that is not 64 hex characters, an unknown field.
    ///
    /// # Errors
    ///
    /// [`UsersError::Format`] naming what is wrong.
    pub fn from_json(text: &str) -> Result<Self, UsersError> {
        let bad = |message: String| UsersError::Format(message);
        let file: FileDto = rusty_json::from_str(text).map_err(|e| bad(e.to_string()))?;
        if file.version != REGISTRY_VERSION {
            return Err(bad(format!(
                "version {} is not {REGISTRY_VERSION}",
                file.version
            )));
        }
        let mut registry = Registry::default();
        for user in file.users {
            let key = UserKey::parse(&user.key).map_err(|e| bad(e.to_string()))?;
            if registry.users.contains_key(key.as_str()) {
                return Err(bad(format!("user {:?} is listed twice", user.key)));
            }
            let mut tokens: Vec<TokenRecord> = Vec::new();
            for token in user.tokens {
                check_label(&token.label).map_err(|e| bad(e.to_string()))?;
                let sha256 = unhex(&token.sha256)
                    .ok_or_else(|| bad(format!("token {:?}: sha256 is not 64 hex", token.id)))?;
                if tokens.iter().any(|t| t.id == token.id) {
                    return Err(bad(format!(
                        "user {:?}: token id {:?} is listed twice",
                        user.key, token.id
                    )));
                }
                tokens.push(TokenRecord {
                    id: token.id,
                    label: token.label,
                    sha256,
                    created_ms: token.created_ms,
                });
            }
            registry.users.insert(
                key.as_str().to_string(),
                UserRecord {
                    key,
                    disabled: user.disabled,
                    tokens,
                },
            );
        }
        Ok(registry)
    }

    /// Read `path`.
    ///
    /// # Errors
    ///
    /// [`UsersError::Io`] if it cannot be read, [`UsersError::Format`] if it is
    /// not a registry.
    pub fn load(path: &Path) -> Result<Self, UsersError> {
        let text = std::fs::read_to_string(path).map_err(io_error(path))?;
        Self::from_json(&text)
    }

    /// Write `path` whole: to a temporary file beside it, synced, then
    /// renamed over it, so a reader (a running server) sees the old file or
    /// the new one and never half of either. The file is `0600` on Unix.
    ///
    /// # Errors
    ///
    /// [`UsersError::Io`].
    pub fn save(&self, path: &Path) -> Result<(), UsersError> {
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let write = || -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&temporary)?;
            file.write_all(self.to_json().as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)
        };
        write().map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            UsersError::Io {
                path: path.to_path_buf(),
                source,
            }
        })
    }
}

fn check_label(label: &str) -> Result<(), UsersError> {
    if label.len() > MAX_LABEL_BYTES || label.chars().any(char::is_control) {
        return Err(UsersError::InvalidLabel);
    }
    Ok(())
}

/// A fresh secret: 32 OS random bytes as unpadded URL-safe base64.
fn mint_secret() -> Result<String, UsersError> {
    let mut bytes = [0u8; SECRET_BYTES];
    rusty_rand::fill(&mut bytes).map_err(|e| UsersError::Random(e.to_string()))?;
    Ok(rusty_base64::encode_url_safe_no_pad(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Digest32> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileDto {
    version: u32,
    users: Vec<UserDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserDto {
    key: String,
    #[serde(default)]
    disabled: bool,
    tokens: Vec<TokenDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TokenDto {
    id: String,
    label: String,
    sha256: String,
    created_ms: i64,
}

/// The registry file, kept current for a running server: it is re-read when
/// its modification time or length changes, looked at no more than once per
/// `interval`, so a revoke takes effect without a restart.
///
/// A file that no longer parses (a hand edit gone wrong, or the file removed)
/// does not replace the last good registry: the server keeps answering from
/// it, and [`RegistryFile::last_error`] says why the file was refused.
pub struct RegistryFile {
    path: PathBuf,
    registry: Registry,
    stamp: Option<Stamp>,
    checked: Instant,
    interval: Duration,
    last_error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok()?,
        len: meta.len(),
    })
}

impl RegistryFile {
    /// Read `path`, which must exist and parse.
    ///
    /// # Errors
    ///
    /// As [`Registry::load`].
    pub fn open(path: &Path, interval: Duration) -> Result<Self, UsersError> {
        let registry = Registry::load(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            registry,
            stamp: stamp(path),
            checked: Instant::now(),
            interval,
            last_error: None,
        })
    }

    /// The registry, re-read first if the file changed and `interval` has
    /// passed since the last look.
    pub fn current(&mut self) -> &Registry {
        if self.checked.elapsed() >= self.interval {
            self.checked = Instant::now();
            let now = stamp(&self.path);
            if now != self.stamp {
                match Registry::load(&self.path) {
                    Ok(registry) => {
                        self.registry = registry;
                        self.last_error = None;
                    }
                    Err(e) => self.last_error = Some(e.to_string()),
                }
                self.stamp = now;
            }
        }
        &self.registry
    }

    /// Why the file was last refused, or `None` if the last read was good.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> UserKey {
        UserKey::parse(name).unwrap()
    }

    /// A registry with `alice` and one token; returns the token.
    fn with_alice() -> (Registry, String) {
        let mut registry = Registry::default();
        registry.add_user(&key("alice")).unwrap();
        let token = registry.add_token(&key("alice"), "phone", 1).unwrap();
        (registry, token)
    }

    #[test]
    fn a_minted_token_names_its_user_and_authenticates() {
        let (registry, token) = with_alice();
        let (user, secret) = token.split_once('.').unwrap();
        assert_eq!(user, "alice");
        assert_eq!(secret.len(), SECRET_CHARS);
        assert_eq!(registry.authenticate(&token), Some(key("alice")));
    }

    #[test]
    fn two_tokens_are_different_and_both_work() {
        let (mut registry, first) = with_alice();
        let second = registry.add_token(&key("alice"), "laptop", 2).unwrap();
        assert_ne!(first, second);
        assert_eq!(registry.authenticate(&first), Some(key("alice")));
        assert_eq!(registry.authenticate(&second), Some(key("alice")));
    }

    #[test]
    fn revoking_one_token_leaves_the_other() {
        let (mut registry, first) = with_alice();
        let second = registry.add_token(&key("alice"), "laptop", 2).unwrap();
        let id = registry.user(&key("alice")).unwrap().tokens[0].id.clone();
        registry.revoke(&key("alice"), &id).unwrap();
        assert_eq!(registry.authenticate(&first), None);
        assert_eq!(registry.authenticate(&second), Some(key("alice")));
        assert!(matches!(
            registry.revoke(&key("alice"), &id),
            Err(UsersError::UnknownToken { .. })
        ));
    }

    #[test]
    fn a_disabled_user_is_refused_until_enabled() {
        let (mut registry, token) = with_alice();
        registry.set_disabled(&key("alice"), true).unwrap();
        assert_eq!(registry.authenticate(&token), None);
        registry.set_disabled(&key("alice"), false).unwrap();
        assert_eq!(registry.authenticate(&token), Some(key("alice")));
    }

    #[test]
    fn every_kind_of_wrong_credential_is_refused() {
        let (mut registry, token) = with_alice();
        registry.add_user(&key("bob")).unwrap();
        let bobs = registry.add_token(&key("bob"), "", 1).unwrap();
        let (_, secret) = token.split_once('.').unwrap();
        let (_, bobs_secret) = bobs.split_once('.').unwrap();
        let wrong_secret = format!("alice.{}", "B".repeat(SECRET_CHARS));
        let cases = [
            String::new(),
            "alice".to_string(),
            format!("alice.{}", &secret[1..]),  // too short
            format!("alice.{secret}A"),         // too long
            format!("alice.{}!", &secret[1..]), // outside the alphabet
            format!("../x.{secret}"),           // not a user key
            format!("nobody.{secret}"),         // unknown user
            format!("bob.{secret}"),            // alice's secret, bob's name
            format!("alice.{bobs_secret}"),     // bob's secret, alice's name
            wrong_secret,                       // right shape, wrong secret
            format!("alice.{secret}.{secret}"), // extra dot
            token.to_uppercase(),               // case matters
        ];
        for case in &cases {
            assert_eq!(registry.authenticate(case), None, "{case:?}");
        }
        assert_eq!(registry.authenticate(&token), Some(key("alice")));
        assert_eq!(registry.authenticate(&bobs), Some(key("bob")));
    }

    #[test]
    fn parse_splits_at_the_first_dot_only_for_well_formed_secrets() {
        let secret = "A".repeat(SECRET_CHARS);
        let credential = format!("a-b_1.{secret}");
        let parsed = Token::parse(&credential).unwrap();
        assert_eq!(parsed.user, key("a-b_1"));
        assert_eq!(parsed.secret, secret);
        assert_eq!(Token::parse(&format!(".{secret}")), None);
        assert_eq!(Token::parse(&format!("a.b.{secret}")), None);
    }

    #[test]
    fn the_file_holds_digests_and_never_the_secret() {
        let (registry, token) = with_alice();
        let (_, secret) = token.split_once('.').unwrap();
        let json = registry.to_json();
        assert!(!json.contains(secret));
        assert!(json.contains(&hex(&digest(secret))));
    }

    #[test]
    fn a_registry_round_trips_through_json() {
        let (mut registry, _) = with_alice();
        registry.add_user(&key("bob")).unwrap();
        registry.set_disabled(&key("bob"), true).unwrap();
        registry.add_token(&key("bob"), "ci", 9).unwrap();
        let back = Registry::from_json(&registry.to_json()).unwrap();
        assert_eq!(back, registry);
    }

    #[test]
    fn a_file_this_build_does_not_fully_understand_is_refused() {
        let good = "a".repeat(64);
        let token = |id: &str, sha: &str| {
            format!(r#"{{"id":"{id}","label":"","sha256":"{sha}","createdMs":1}}"#)
        };
        let file =
            |version: u32, users: &str| format!(r#"{{"version":{version},"users":[{users}]}}"#);
        let user = |key: &str, tokens: &str| format!(r#"{{"key":"{key}","tokens":[{tokens}]}}"#);
        let cases = [
            ("not json", "nope".to_string()),
            ("another version", file(2, "")),
            ("bad key", file(1, &user("../x", ""))),
            (
                "duplicate user",
                file(1, &format!("{},{}", user("a", ""), user("a", ""))),
            ),
            (
                "short digest",
                file(1, &user("a", &token("t1", &"a".repeat(63)))),
            ),
            (
                "non-hex digest",
                file(1, &user("a", &token("t1", &"g".repeat(64)))),
            ),
            (
                "duplicate token id",
                file(
                    1,
                    &user(
                        "a",
                        &format!("{},{}", token("t1", &good), token("t1", &good)),
                    ),
                ),
            ),
            (
                "unknown field",
                r#"{"version":1,"users":[],"extra":true}"#.to_string(),
            ),
        ];
        for (name, text) in cases {
            assert!(
                matches!(Registry::from_json(&text), Err(UsersError::Format(_))),
                "{name}"
            );
        }
        assert!(Registry::from_json(&file(1, &user("a", &token("t1", &good)))).is_ok());
    }

    #[test]
    fn users_and_labels_are_validated() {
        let mut registry = Registry::default();
        registry.add_user(&key("a")).unwrap();
        assert!(matches!(
            registry.add_user(&key("a")),
            Err(UsersError::UserExists(_))
        ));
        assert!(matches!(
            registry.add_token(&key("nobody"), "", 1),
            Err(UsersError::UnknownUser(_))
        ));
        assert!(matches!(
            registry.add_token(&key("a"), &"x".repeat(MAX_LABEL_BYTES + 1), 1),
            Err(UsersError::InvalidLabel)
        ));
        assert!(matches!(
            registry.add_token(&key("a"), "a\nb", 1),
            Err(UsersError::InvalidLabel)
        ));
        assert!(matches!(
            registry.set_disabled(&key("nobody"), true),
            Err(UsersError::UnknownUser(_))
        ));
    }

    #[test]
    fn the_digest_is_sha256() {
        // FIPS 180-2's "abc" vector.
        assert_eq!(
            hex(&digest("abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hex_round_trips_and_refuses_the_wrong_shape() {
        let bytes: Digest32 = std::array::from_fn(|i| i as u8 * 7);
        assert_eq!(unhex(&hex(&bytes)), Some(bytes));
        assert_eq!(unhex(""), None);
        assert_eq!(unhex(&"é".repeat(32)), None);
    }

    #[test]
    fn save_writes_atomically_and_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let (registry, token) = with_alice();
        registry.save(&path).unwrap();
        assert!(!dir.path().join("users.json.tmp").exists());
        let back = Registry::load(&path).unwrap();
        assert_eq!(back.authenticate(&token), Some(key("alice")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn loading_a_missing_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = Registry::load(&dir.path().join("users.json")).unwrap_err();
        assert!(matches!(err, UsersError::Io { .. }), "{err}");
    }

    #[test]
    fn a_running_file_sees_a_revoke_without_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let (mut registry, token) = with_alice();
        registry.save(&path).unwrap();
        let mut file = RegistryFile::open(&path, Duration::ZERO).unwrap();
        assert_eq!(file.current().authenticate(&token), Some(key("alice")));

        let id = registry.user(&key("alice")).unwrap().tokens[0].id.clone();
        registry.revoke(&key("alice"), &id).unwrap();
        registry.add_user(&key("bob")).unwrap(); // a different length, so a new stamp
        registry.save(&path).unwrap();
        assert_eq!(file.current().authenticate(&token), None);
        assert!(file.last_error().is_none());
    }

    #[test]
    fn a_broken_edit_keeps_the_last_good_registry_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let (registry, token) = with_alice();
        registry.save(&path).unwrap();
        let mut file = RegistryFile::open(&path, Duration::ZERO).unwrap();

        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(file.current().authenticate(&token), Some(key("alice")));
        assert!(file.last_error().unwrap().contains("users file"));

        registry.save(&path).unwrap();
        assert_eq!(file.current().authenticate(&token), Some(key("alice")));
        assert!(file.last_error().is_none());
    }

    #[test]
    fn a_file_is_not_looked_at_more_often_than_its_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let (mut registry, token) = with_alice();
        registry.save(&path).unwrap();
        let mut file = RegistryFile::open(&path, Duration::from_secs(3600)).unwrap();
        let id = registry.user(&key("alice")).unwrap().tokens[0].id.clone();
        registry.revoke(&key("alice"), &id).unwrap();
        registry.save(&path).unwrap();
        // The change is on disk, but the interval has not passed.
        assert_eq!(file.current().authenticate(&token), Some(key("alice")));
    }
}
