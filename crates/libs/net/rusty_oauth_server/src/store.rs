//! JSON-file persistence for OAuth state: registered clients and the
//! hashes of issued access and refresh tokens.
//!
//! The file is re-read on every operation, so a change made by another
//! process (an admin tool revoking a client, say) is visible at once, with
//! no cache to invalidate.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusty_json::{Map, Value};
use serde::{Deserialize, Serialize};

/// Which token bucket a stored or looked-up token hash belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Access,
    Refresh,
}

/// Counts of tokens a bulk operation touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCounts {
    pub access_tokens: usize,
    pub refresh_tokens: usize,
}

/// One registered client with its live token counts, for an admin listing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientSummary {
    pub client_id: String,
    pub client_name: Option<String>,
    pub client_id_issued_at: Option<i64>,
    pub redirect_uris: Option<Vec<String>>,
    pub access_tokens: usize,
    pub refresh_tokens: usize,
}

/// What revoking a client reports: who it was, and the tokens dropped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeSummary {
    pub client_id: String,
    pub client_name: Option<String>,
    pub access_tokens: usize,
    pub refresh_tokens: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    /// The issuer this file's grants were made under; see
    /// [`StateStore::bound`]. Absent in files written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(default)]
    clients: Map,
    #[serde(default)]
    access_tokens: Map,
    #[serde(default)]
    refresh_tokens: Map,
}

impl State {
    fn is_empty(&self) -> bool {
        self.clients.is_empty() && self.access_tokens.is_empty() && self.refresh_tokens.is_empty()
    }

    fn tokens(&self, kind: TokenKind) -> &Map {
        match kind {
            TokenKind::Access => &self.access_tokens,
            TokenKind::Refresh => &self.refresh_tokens,
        }
    }

    fn tokens_mut(&mut self, kind: TokenKind) -> &mut Map {
        match kind {
            TokenKind::Access => &mut self.access_tokens,
            TokenKind::Refresh => &mut self.refresh_tokens,
        }
    }

    /// Remove every token (both buckets) whose metadata `doomed` matches,
    /// mutating `self`.
    fn drop_tokens(&mut self, doomed: impl Fn(&Value) -> bool) -> TokenCounts {
        let drop_from = |tokens: &mut Map| {
            let hashes: Vec<String> = tokens
                .iter()
                .filter(|(_, meta)| doomed(meta))
                .map(|(hash, _)| hash.clone())
                .collect();
            for hash in &hashes {
                tokens.remove(hash);
            }
            hashes.len()
        };
        TokenCounts {
            access_tokens: drop_from(&mut self.access_tokens),
            refresh_tokens: drop_from(&mut self.refresh_tokens),
        }
    }
}

fn client_of(meta: &Value) -> Option<&str> {
    meta.get("client_id").and_then(Value::as_str)
}

fn subject_of(meta: &Value) -> Option<&str> {
    meta.get("subject").and_then(Value::as_str)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn encode(state: &State) -> io::Result<String> {
    let body = rusty_json::to_string_pretty(state).map_err(|e| invalid(e.to_string()))?;
    Ok(format!("{body}\n"))
}

/// SHA-256 hex digest of a token — what the state file stores instead of
/// the raw secret.
fn hash_token(token: &str) -> String {
    rusty_hex::encode(&rusty_oauth::crypto::sha256::sha256(token.as_bytes()))
}

/// JSON-file persistence for OAuth state.
///
/// Layout: `{"clients": {client_id: <RFC 7591 record>}, "access_tokens":
/// {sha256_hex: {client_id, subject, scopes, expires_at, resource}},
/// "refresh_tokens": {sha256_hex: {client_id, subject, scopes,
/// expires_at}}}`. The layout is remind-me's original one, so its existing
/// state files load unchanged (grants written before `subject` existed have
/// none). The file is `0600` and re-read on every operation. The in-process
/// [`Mutex`] only serializes this process's own read-modify-write cycles;
/// cross-process locking is out of scope for a single-owner store.
///
/// A store made with [`Self::bound`] belongs to one issuer, and one made
/// with [`Self::new`] to none (remind-me's and rusty_tick's files). Every
/// access checks the file's issuer against the store's: a file that names
/// another (replaced, or claimed by a second server racing this one) reads
/// as empty and refuses writes, so no store ever uses another's grants.
///
/// Records are raw [`Value`]s, not structs, so an admin tool can list and
/// revoke clients through this type alone, without a [`crate::Provider`].
pub struct StateStore {
    path: PathBuf,
    issuer: Option<String>,
    lock: Mutex<()>,
}

impl StateStore {
    /// A store for a file that names no issuer.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            issuer: None,
            lock: Mutex::new(()),
        }
    }

    /// A store for `issuer`'s grants, so grants made by one server are never
    /// honoured by another that was pointed at the same file. A missing file
    /// is created for `issuer` exclusively (of two servers creating it at
    /// once, one fails). A file that names no issuer is claimed only while it
    /// holds nothing: grants in an unbound file are no evidence of which
    /// server made them.
    ///
    /// # Errors
    /// If the file names a different issuer, names none but holds clients or
    /// tokens, exists but does not parse (it is never silently replaced), was
    /// claimed by another server meanwhile, or cannot be written.
    pub fn bound(path: impl Into<PathBuf>, issuer: &str) -> io::Result<Self> {
        let store = Self {
            path: path.into(),
            issuer: Some(issuer.to_string()),
            lock: Mutex::new(()),
        };
        store.claim()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_once(&self) -> Option<State> {
        let raw = fs::read_to_string(&self.path).ok()?;
        rusty_json::from_str(&raw).ok()
    }

    /// Load state from disk, tolerating a missing or corrupt file (reads
    /// as empty rather than propagating an error, like a fresh store).
    ///
    /// This used to retry a failed read up to eight times with a backoff,
    /// because a file this very process had just written and verified could
    /// transiently read back as missing under parallel test load. That was
    /// never a filesystem quirk to be waited out: [`Self::write`] truncated
    /// the real path in place, so a concurrent reader genuinely could catch
    /// it empty or half-written. Writing to a temp file and renaming makes
    /// that window not exist — a reader sees the old complete file or the
    /// new one — so the retries have nothing left to paper over and are
    /// gone with the cause (issue #160).
    fn read(&self) -> State {
        self.load().unwrap_or_else(|e| {
            eprintln!("oauth: {e}");
            State::default()
        })
    }

    /// [`Self::read`] for a read-modify-write: a file that belongs to
    /// another issuer is an error, so it is never overwritten.
    fn load(&self) -> io::Result<State> {
        let state = self.read_once().unwrap_or_default();
        if state.issuer == self.issuer {
            return Ok(state);
        }
        Err(invalid(match (&state.issuer, &self.issuer) {
            (Some(found), Some(ours)) => format!(
                "{} holds the grants of issuer {found}, not {ours}: give each server its own state file",
                self.path.display()
            ),
            (Some(found), None) => format!(
                "{} is bound to issuer {found}, and this store to none",
                self.path.display()
            ),
            (None, _) => format!(
                "{} no longer names this store's issuer: it was replaced or removed while in use",
                self.path.display()
            ),
        }))
    }

    /// Persist `state` atomically, creating the parent directory if needed
    /// and setting `0600` permissions on unix.
    ///
    /// Writes a sibling temp file, fsyncs it, then renames it over the real
    /// path. `rename` is atomic, so a concurrent reader observes either the
    /// complete old file or the complete new one and never a truncated one
    /// (issue #160). The previous implementation truncated the real path in
    /// place, which is what made a reader's "the file is missing" possible
    /// at all, and what the retry-and-verify loop here was really working
    /// around.
    ///
    /// The temp file is created `0600` (`rusty_atomic_file::write_private`),
    /// so the state is never readable by others, not even briefly. The old
    /// order — write, then chmod — left exactly that window on every
    /// single write.
    ///
    /// **Failures propagate.** They used to be swallowed after a few
    /// retries, on the reasoning that a caller holding the token in memory
    /// keeps working. That is the wrong trade for the one caller that
    /// matters: `issue_tokens` would hand a client a bearer token that was
    /// never persisted, so the very next authenticated request fails with
    /// no indication why, on either side. A refused issuance is a worse
    /// user experience and a far better failure.
    fn write(&self, state: &State) -> io::Result<()> {
        let body = encode(state)?;
        self.create_parent()?;
        rusty_atomic_file::write_private(&self.path, body.as_bytes())
    }

    /// Write `state` to a file that must not exist yet (`0600` on unix):
    /// `AlreadyExists` if another store created it first.
    fn create_new(&self, state: &State) -> io::Result<()> {
        let body = encode(state)?;
        self.create_parent()?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()
    }

    fn create_parent(&self) -> io::Result<()> {
        match self.path.parent() {
            Some(parent) => fs::create_dir_all(parent),
            None => Ok(()),
        }
    }

    /// Make the file this store's, as [`Self::bound`] describes.
    fn claim(&self) -> io::Result<()> {
        let bound = State {
            issuer: self.issuer.clone(),
            ..State::default()
        };
        match self.create_new(&bound) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            created => return created,
        }
        let found = self.read_once().ok_or_else(|| {
            invalid(format!(
                "{} is not a readable OAuth state file",
                self.path.display()
            ))
        })?;
        match (&found.issuer, found.is_empty()) {
            // Ours, or another's (refused).
            (Some(_), _) => self.load().map(drop),
            (None, false) => Err(invalid(format!(
                "{} holds grants of no known issuer: give this server a new state file, or delete this one with its server stopped",
                self.path.display()
            ))),
            (None, true) => {
                self.write(&bound)?;
                // A server claiming the same empty file at once may have won.
                self.load().map(drop)
            }
        }
    }

    // -- clients --------------------------------------------------------

    /// Return the stored registration record for `client_id`, or `None`.
    pub fn get_client(&self, client_id: &str) -> Option<Value> {
        self.read().clients.get(client_id).cloned()
    }

    /// Insert or replace a client registration record.
    ///
    /// Returns the write error rather than swallowing it: a registration
    /// the caller believes succeeded but that never reached disk means the
    /// client is authenticated for this process's lifetime and a stranger
    /// after the next restart (issue #160).
    pub fn put_client(&self, client_id: &str, record: Value) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        state.clients.insert(client_id.to_string(), record);
        self.write(&state)
    }

    /// Summarise registered clients with live token counts (for
    /// an admin listing).
    pub fn list_clients(&self) -> Vec<ClientSummary> {
        let state = self.read();
        state
            .clients
            .iter()
            .map(|(client_id, record)| {
                let count = |kind: TokenKind| {
                    state
                        .tokens(kind)
                        .values()
                        .filter(|meta| {
                            meta.get("client_id").and_then(Value::as_str) == Some(client_id)
                        })
                        .count()
                };
                ClientSummary {
                    client_id: client_id.clone(),
                    client_name: record
                        .get("client_name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    client_id_issued_at: record.get("client_id_issued_at").and_then(Value::as_i64),
                    redirect_uris: record.get("redirect_uris").and_then(Value::as_array).map(
                        |arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        },
                    ),
                    access_tokens: count(TokenKind::Access),
                    refresh_tokens: count(TokenKind::Refresh),
                }
            })
            .collect()
    }

    /// Delete a client registration and every token it holds.
    ///
    /// Returns `None` when `client_id` is unknown — this is the
    /// "revoke one, by id" operation; there is no "revoke all".
    /// `Ok(None)` when the client was not registered; `Err` when the
    /// revocation could not be persisted — which must not read as success,
    /// since the client would still be authenticated after a restart
    /// (issue #160).
    pub fn revoke_client(&self, client_id: &str) -> io::Result<Option<RevokeSummary>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        let Some(record) = state.clients.remove(client_id) else {
            return Ok(None);
        };
        let counts = state.drop_tokens(|meta| client_of(meta) == Some(client_id));
        self.write(&state)?;
        Ok(Some(RevokeSummary {
            client_id: client_id.to_string(),
            client_name: record
                .get("client_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            access_tokens: counts.access_tokens,
            refresh_tokens: counts.refresh_tokens,
        }))
    }

    // -- tokens -----------------------------------------------------------

    /// Store `token`'s hash (never the raw secret) under `kind`.
    ///
    /// The failure that matters most (issue #160): a token handed to a
    /// client but never persisted is rejected on its first use, with
    /// nothing on either side explaining why.
    pub fn put_token(&self, kind: TokenKind, token: &str, meta: Value) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        state.tokens_mut(kind).insert(hash_token(token), meta);
        self.write(&state)
    }

    /// Look up a raw token by hash; `None` when unknown.
    pub fn get_token(&self, kind: TokenKind, token: &str) -> Option<Value> {
        self.read().tokens(kind).get(&hash_token(token)).cloned()
    }

    /// Forget a raw token (no-op when unknown).
    ///
    /// A silently-failed delete is a revocation that did not happen, so
    /// this reports rather than swallows (issue #160).
    pub fn delete_token(&self, kind: TokenKind, token: &str) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        let hash = hash_token(token);
        if state.tokens_mut(kind).remove(&hash).is_some() {
            return self.write(&state);
        }
        Ok(())
    }

    /// Replace the `kind` token `old` with the tokens `with` makes from its
    /// metadata, in one locked read-modify-write: `true` if it was replaced.
    /// `false`, with nothing changed, when `old` is absent (already used or
    /// revoked) or `with` refuses it. A refresh rotation is this, so two
    /// concurrent rotations of one token cannot both issue, and a revocation
    /// that ran first cannot be undone by one that read the token earlier.
    pub fn replace_token(
        &self,
        kind: TokenKind,
        old: &str,
        with: impl FnOnce(&Value) -> Option<Vec<(TokenKind, String, Value)>>,
    ) -> io::Result<bool> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        let hash = hash_token(old);
        let Some(meta) = state.tokens(kind).get(&hash) else {
            return Ok(false);
        };
        let Some(replacements) = with(meta) else {
            return Ok(false);
        };
        state.tokens_mut(kind).remove(&hash);
        for (kind, token, meta) in replacements {
            state.tokens_mut(kind).insert(hash_token(&token), meta);
        }
        self.write(&state)?;
        Ok(true)
    }

    /// Drop every access/refresh token of one grant family: `client_id`'s
    /// tokens for `subject`, keeping the registration and every other
    /// subject's tokens. `None` is the family of grants written before
    /// subjects existed. One public client registration can be authorized by
    /// several users, so a client id alone does not name one user's grants.
    pub fn delete_grant(&self, client_id: &str, subject: Option<&str>) -> io::Result<TokenCounts> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.load()?;
        let counts = state
            .drop_tokens(|meta| client_of(meta) == Some(client_id) && subject_of(meta) == subject);
        if counts.access_tokens > 0 || counts.refresh_tokens > 0 {
            self.write(&state)?;
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rrm_oauth_store_{label}_{}_{}",
            std::process::id(),
            crate::generate_token().unwrap()
        ));
        dir.join("oauth.json")
    }

    #[test]
    fn oauth_store_tolerates_a_missing_or_corrupt_state_file() {
        let path = temp_state_path("missing");
        let store = StateStore::new(&path);
        assert_eq!(store.list_clients(), Vec::new());
        assert!(store.get_client("x").is_none());
        assert!(store.revoke_client("x").unwrap().is_none());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert_eq!(store.list_clients(), Vec::new());

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_store_round_trips_a_client_registration() {
        let path = temp_state_path("client");
        let store = StateStore::new(&path);
        let record = rusty_json::json!({
            "client_name": "claude.ai",
            "client_id_issued_at": 1_700_000_000i64,
            "redirect_uris": ["https://claude.ai/api/mcp/auth_callback"],
        });
        store.put_client("client-1", record.clone()).expect("write");

        assert_eq!(store.get_client("client-1"), Some(record));
        let clients = store.list_clients();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].client_id, "client-1");
        assert_eq!(clients[0].client_name.as_deref(), Some("claude.ai"));
        assert_eq!(clients[0].access_tokens, 0);
        assert_eq!(clients[0].refresh_tokens, 0);

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_store_state_file_has_0600_permissions() {
        let path = temp_state_path("perms");
        let store = StateStore::new(&path);
        store
            .put_client("c1", rusty_json::json!({}))
            .expect("write");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_store_never_persists_raw_token_values() {
        let path = temp_state_path("hash");
        let store = StateStore::new(&path);
        store
            .put_token(
                TokenKind::Access,
                "super-secret-raw-access-token",
                rusty_json::json!({ "client_id": "c1" }),
            )
            .expect("write");

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("super-secret-raw-access-token"));
        assert!(store
            .get_token(TokenKind::Access, "super-secret-raw-access-token")
            .is_some());
        assert!(store.get_token(TokenKind::Access, "wrong-token").is_none());

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_store_delete_grant_drops_only_that_client_and_subjects_tokens() {
        let path = temp_state_path("delete");
        let store = StateStore::new(&path);
        store
            .put_token(
                TokenKind::Access,
                "tok-a",
                rusty_json::json!({ "client_id": "c1" }),
            )
            .expect("write");
        store
            .put_token(
                TokenKind::Refresh,
                "tok-r",
                rusty_json::json!({ "client_id": "c1" }),
            )
            .expect("write");
        store
            .put_token(
                TokenKind::Access,
                "tok-b",
                rusty_json::json!({ "client_id": "c2" }),
            )
            .expect("write");

        let alice = rusty_json::json!({ "client_id": "c1", "subject": "alice" });
        store
            .put_token(TokenKind::Refresh, "tok-alice", alice.clone())
            .expect("write");

        // The legacy family (no subject): c1's subjectless tokens only.
        let counts = store.delete_grant("c1", None).expect("write");
        assert_eq!(counts.access_tokens, 1);
        assert_eq!(counts.refresh_tokens, 1);
        assert!(store.get_token(TokenKind::Access, "tok-a").is_none());
        assert_eq!(
            store.get_token(TokenKind::Access, "tok-b"),
            Some(rusty_json::json!({ "client_id": "c2" }))
        );
        assert_eq!(
            store.get_token(TokenKind::Refresh, "tok-alice"),
            Some(alice)
        );

        // Another subject's family on the same client leaves Alice's alone.
        let none = store.delete_grant("c1", Some("bob")).expect("write");
        assert_eq!((none.access_tokens, none.refresh_tokens), (0, 0));
        assert!(store.get_token(TokenKind::Refresh, "tok-alice").is_some());
        let hers = store.delete_grant("c1", Some("alice")).expect("write");
        assert_eq!((hers.access_tokens, hers.refresh_tokens), (0, 1));

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn oauth_store_revoke_client_deletes_the_registration_and_its_tokens_but_leaves_other_clients_alone(
    ) {
        let path = temp_state_path("revoke");
        let store = StateStore::new(&path);
        store
            .put_client("c1", rusty_json::json!({ "client_name": "claude.ai" }))
            .expect("write");
        store
            .put_client("c2", rusty_json::json!({ "client_name": "other" }))
            .expect("write");
        store
            .put_token(
                TokenKind::Access,
                "tok-a",
                rusty_json::json!({ "client_id": "c1" }),
            )
            .expect("write");
        store
            .put_token(
                TokenKind::Refresh,
                "tok-r",
                rusty_json::json!({ "client_id": "c1" }),
            )
            .expect("write");

        let summary = store
            .revoke_client("c1")
            .expect("write")
            .expect("c1 is registered");
        assert_eq!(summary.client_id, "c1");
        assert_eq!(summary.client_name.as_deref(), Some("claude.ai"));
        assert_eq!(summary.access_tokens, 1);
        assert_eq!(summary.refresh_tokens, 1);

        assert!(store.get_client("c1").is_none());
        assert!(store.get_token(TokenKind::Access, "tok-a").is_none());
        let remaining: Vec<String> = store
            .list_clients()
            .into_iter()
            .map(|c| c.client_id)
            .collect();
        assert_eq!(remaining, vec!["c2".to_string()]);

        // Revoking an already-unknown client_id is a no-op that reports None
        // -- not an error, not a silent "revoked everything".
        assert!(store.revoke_client("c1").unwrap().is_none());
        assert!(store.revoke_client("no-such-client").unwrap().is_none());

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    // -- OAuth state durability (issue #160) --------------------------------

    use rusty_json::json;
    use std::path::PathBuf;

    /// A path whose parent is a regular file, so every write fails.
    ///
    /// Chosen over a read-only directory because these tests also run as
    /// root in CI containers, where mode bits are simply bypassed and a
    /// permission-based injection quietly succeeds instead of failing.
    fn unwritable_store() -> (StateStore, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "rrm_oauth_unwritable_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&base, b"not a directory").expect("seed blocker file");
        let store = StateStore::new(base.join("nested").join("oauth.json"));
        (store, base)
    }

    #[test]
    fn a_write_that_cannot_land_is_reported_not_swallowed() {
        let (store, base) = unwritable_store();

        // Every mutator must surface it. Previously all of these returned
        // (), so a token could be handed to a client that was never stored.
        assert!(store
            .put_token(TokenKind::Access, "tok", json!({}))
            .is_err());
        assert!(store.put_client("cid", json!({})).is_err());

        let _ = fs::remove_file(&base);
    }

    #[test]
    fn a_successful_write_leaves_no_temp_file_behind() {
        let dir = std::env::temp_dir().join(format!("rrm_oauth_tmp_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = StateStore::new(dir.join("oauth.json"));
        store
            .put_token(TokenKind::Access, "tok", json!({"a": 1}))
            .unwrap();

        let strays: Vec<_> = fs::read_dir(&dir)
            .expect("state dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "oauth.json")
            .collect();
        assert!(strays.is_empty(), "temp files left behind: {strays:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file_behind() {
        // The rename is what can fail after the temp file exists, so the
        // cleanup path needs its own check -- otherwise a store on a
        // failing volume slowly fills its directory with debris.
        let dir = std::env::temp_dir().join(format!("rrm_oauth_tmpfail_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        // Make the destination a directory: the write succeeds, the rename
        // onto a non-empty directory does not.
        let target = dir.join("oauth.json");
        fs::create_dir_all(target.join("occupied")).expect("blocker dir");

        let store = StateStore::new(&target);
        assert!(store
            .put_token(TokenKind::Access, "tok", json!({}))
            .is_err());

        let strays: Vec<_> = fs::read_dir(&dir)
            .expect("state dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "oauth.json")
            .collect();
        assert!(
            strays.is_empty(),
            "temp files left behind after failure: {strays:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_state_file_is_never_world_readable_even_briefly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rrm_oauth_perm_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = StateStore::new(dir.join("oauth.json"));
        store
            .put_token(TokenKind::Access, "tok", json!({}))
            .unwrap();

        let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "state file mode is {mode:o}, expected 600");

        let _ = fs::remove_dir_all(&dir);
    }

    /// The failure this whole issue is about: a reader must never observe a
    /// half-written state file.
    ///
    /// Under the old truncate-in-place write this raced -- a reader could
    /// catch the file empty and read it as "no tokens", which is what made
    /// a just-issued token vanish. With write-temp-then-rename the reader
    /// sees the old complete file or the new one, so every observation
    /// must parse and every one must contain the token written before the
    /// hammering began.
    #[test]
    fn concurrent_readers_never_observe_a_torn_write() {
        let dir = std::env::temp_dir().join(format!("rrm_oauth_torn_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = std::sync::Arc::new(StateStore::new(dir.join("oauth.json")));
        store
            .put_token(TokenKind::Access, "anchor", json!({"n": 0}))
            .unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let (store, stop) = (store.clone(), stop.clone());
            std::thread::spawn(move || {
                for n in 1..=200 {
                    store
                        .put_token(TokenKind::Access, &format!("t{n}"), json!({"n": n}))
                        .expect("write");
                }
                stop.store(true, std::sync::atomic::Ordering::Release);
            })
        };

        let mut observations = 0u32;
        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            // The anchor was durably written before any concurrent writing
            // started, so it is present in every valid state. Reading it as
            // absent means a torn or truncated file was observed.
            assert!(
                store.get_token(TokenKind::Access, "anchor").is_some(),
                "observed a state file without the anchor token -- torn write"
            );
            observations += 1;
        }
        writer.join().expect("writer thread");
        assert!(observations > 0, "reader never ran");

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod compat {
    use super::*;

    #[test]
    fn token_hashes_are_lowercase_sha256_hex_as_remind_me_wrote_them() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_state_file_from_remind_me_loads_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let file = format!(
            r#"{{
  "access_tokens": {{
    "{access}": {{"client_id": "c1", "expires_at": 9999999999, "resource": null, "scopes": []}}
  }},
  "clients": {{
    "c1": {{"client_id": "c1", "client_name": "claude.ai", "client_id_issued_at": 1,
            "redirect_uris": ["https://claude.ai/cb"], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]}}
  }},
  "refresh_tokens": {{}}
}}
"#,
            access = hash_token("live-access")
        );
        fs::write(&path, file).unwrap();
        let store = StateStore::new(&path);
        let clients = store.list_clients();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].client_name.as_deref(), Some("claude.ai"));
        assert_eq!(clients[0].access_tokens, 1);
        let meta = store.get_token(TokenKind::Access, "live-access").unwrap();
        assert_eq!(meta["client_id"], "c1");
    }

    fn access(store: &StateStore, token: &str) -> io::Result<()> {
        store.put_token(
            TokenKind::Access,
            token,
            rusty_json::json!({ "client_id": "c", "subject": "claude" }),
        )
    }

    #[test]
    fn a_new_or_empty_state_file_is_bound_to_the_first_issuer_and_refuses_another() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        // A new file is claimed; restarting with the same issuer is fine.
        let store = StateStore::bound(&path, "https://a.example").unwrap();
        access(&store, "a").unwrap();
        let again = StateStore::bound(&path, "https://a.example").unwrap();
        assert!(again.get_token(TokenKind::Access, "a").is_some());
        // Another server pointed at it is refused, and changes nothing.
        let before = fs::read_to_string(&path).unwrap();
        let refused = StateStore::bound(&path, "https://b.example").err().unwrap();
        assert!(
            refused.to_string().contains("https://a.example"),
            "{refused}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        // An existing but empty, unbound file is claimed too.
        let empty = dir.path().join("empty.json");
        fs::write(&empty, "{}").unwrap();
        StateStore::bound(&empty, "https://a.example").unwrap();
        assert!(fs::read_to_string(&empty)
            .unwrap()
            .contains("https://a.example"));
    }

    #[test]
    fn an_unbound_state_file_with_grants_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        for seed in [
            r#"{"clients":{"c":{"client_id":"c"}}}"#,
            r#"{"access_tokens":{"a":{"client_id":"c","subject":"claude"}}}"#,
            r#"{"refresh_tokens":{"r":{"client_id":"c","subject":"claude"}}}"#,
        ] {
            let path = dir.path().join("legacy.json");
            fs::write(&path, seed).unwrap();
            let refused = StateStore::bound(&path, "https://a.example").err().unwrap();
            assert!(refused.to_string().contains("no known issuer"), "{refused}");
            assert_eq!(fs::read_to_string(&path).unwrap(), seed, "untouched");
        }
    }

    #[test]
    fn an_unreadable_state_file_is_refused_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        fs::write(&path, "not json").unwrap();
        assert!(StateStore::bound(&path, "https://a.example").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
    }

    /// Independent stores stand in for separate processes: each has its own
    /// lock, so only the file itself can settle who owns it.
    fn race(path: &Path, servers: usize) -> Vec<StateStore> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(servers));
        let handles: Vec<_> = (0..servers)
            .map(|i| {
                let (path, barrier) = (path.to_path_buf(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    StateStore::bound(path, &format!("https://{i}.example")).ok()
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect()
    }

    #[test]
    fn of_servers_creating_one_new_state_file_at_once_exactly_one_owns_it() {
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("oauth.json");
            let started = race(&path, 8);
            assert_eq!(started.len(), 1, "exactly one creates the file");
            access(&started[0], "t").unwrap();
        }
    }

    #[test]
    fn of_servers_claiming_one_empty_state_file_at_once_only_the_owner_can_use_it() {
        for round in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("oauth.json");
            fs::write(&path, "{}").unwrap();
            let started = race(&path, 8);
            // Several may have passed their startup check; the file names one.
            let usable: Vec<_> = started
                .iter()
                .enumerate()
                .filter(|(i, s)| access(s, &format!("t{round}-{i}")).is_ok())
                .collect();
            assert!(usable.len() <= 1, "at most one uses the file");
            for s in &started {
                let own = usable.iter().any(|(_, u)| std::ptr::eq(*u, s));
                let seen = (0..8).any(|i| {
                    s.get_token(TokenKind::Access, &format!("t{round}-{i}"))
                        .is_some()
                });
                assert_eq!(seen, own, "no store sees another's grant");
            }
        }
    }

    #[test]
    fn a_bound_store_refuses_a_file_replaced_or_rebound_after_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let store = StateStore::bound(&path, "https://a.example").unwrap();
        access(&store, "mine").unwrap();
        for foreign in [
            r#"{"issuer":"https://b.example","access_tokens":{"x":{"client_id":"c"}}}"#,
            r#"{"access_tokens":{"x":{"client_id":"c"}}}"#,
        ] {
            fs::write(&path, foreign).unwrap();
            assert!(store.get_client("c").is_none());
            assert!(store.list_clients().is_empty());
            assert!(access(&store, "more").is_err(), "writes are refused");
            assert!(store.delete_grant("c", None).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), foreign, "untouched");
        }
        fs::remove_file(&path).unwrap();
        assert!(
            access(&store, "more").is_err(),
            "a removed file is not re-made"
        );
        assert!(!path.exists());
    }

    #[test]
    fn an_unbound_store_does_not_use_a_bound_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let bound = StateStore::bound(&path, "https://a.example").unwrap();
        access(&bound, "mine").unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let legacy = StateStore::new(&path);
        assert!(legacy.get_token(TokenKind::Access, "mine").is_none());
        assert!(access(&legacy, "theirs").is_err());
        assert!(legacy
            .replace_token(TokenKind::Access, "mine", |_| Some(Vec::new()))
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "untouched");
        assert!(bound.get_token(TokenKind::Access, "mine").is_some());
    }
}
