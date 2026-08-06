//! In-memory service implementations, for development and tests.
//!
//! Nothing here survives process exit. The semantics — especially the state
//! prefix routing in [`InMemorySessionService::append_event`] — match what a
//! persistent backend must do, so an agent that works against these will work
//! against a real store.

use adk_core::{
    AdkError, ArtifactService, Content, Event, MemoryEntry, MemoryService, Part, Result, Session,
    SessionService, State, StateScope, APP_PREFIX, USER_PREFIX,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Key identifying one conversation thread.
type SessionKey = (String, String, String);

#[derive(Default)]
struct SessionStore {
    sessions: BTreeMap<SessionKey, Session>,
    /// App-scoped state, keyed by app name. Shared by every user.
    app_state: BTreeMap<String, BTreeMap<String, Value>>,
    /// User-scoped state, keyed by (app, user). Shared across their sessions.
    user_state: BTreeMap<(String, String), BTreeMap<String, Value>>,
}

/// A [`SessionService`] backed by process memory.
#[derive(Default)]
pub struct InMemorySessionService {
    store: Mutex<SessionStore>,
}

impl InMemorySessionService {
    /// Builds an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Overlays the app- and user-scoped state onto a session's own state.
    ///
    /// Scoped values live outside the session so that a write from one thread
    /// is visible to the others; this reassembles the flat view an agent sees.
    fn hydrate(store: &SessionStore, session: &mut Session) {
        let mut merged: BTreeMap<String, Value> = BTreeMap::new();

        if let Some(app) = store.app_state.get(&session.app_name) {
            merged.extend(app.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let user_key = (session.app_name.clone(), session.user_id.clone());
        if let Some(user) = store.user_state.get(&user_key) {
            merged.extend(user.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        // Session-scoped values win over nothing here — the scopes are
        // disjoint by prefix — but ordering keeps the merge deterministic.
        merged.extend(session.state.to_map());

        session.state = State::from_map(merged);
    }
}

#[async_trait]
impl SessionService for InMemorySessionService {
    async fn create_session(
        &self,
        app_name: &str,
        user_id: &str,
        state: Option<State>,
        session_id: Option<String>,
    ) -> Result<Session> {
        let id = session_id.unwrap_or_else(|| adk_core::new_id("session"));
        let mut session = Session::new(&id, app_name, user_id);
        if let Some(state) = state {
            session.state = state;
        }

        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());

        // Starting state may include scoped keys; route them before storing.
        let initial = session.state.to_map();
        let mut own = BTreeMap::new();
        for (key, value) in initial {
            match StateScope::of(&key) {
                StateScope::App => {
                    store
                        .app_state
                        .entry(app_name.to_string())
                        .or_default()
                        .insert(key, value);
                }
                StateScope::User => {
                    store
                        .user_state
                        .entry((app_name.to_string(), user_id.to_string()))
                        .or_default()
                        .insert(key, value);
                }
                StateScope::Temp => {}
                StateScope::Session => {
                    own.insert(key, value);
                }
            }
        }
        session.state = State::from_map(own);

        let key = (app_name.to_string(), user_id.to_string(), id);
        store.sessions.insert(key, session.clone());
        Self::hydrate(&store, &mut session);
        Ok(session)
    }

    async fn get_session(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<Option<Session>> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            app_name.to_string(),
            user_id.to_string(),
            session_id.to_string(),
        );
        Ok(store.sessions.get(&key).map(|s| {
            let mut session = s.clone();
            Self::hydrate(&store, &mut session);
            session
        }))
    }

    async fn list_sessions(&self, app_name: &str, user_id: &str) -> Result<Vec<Session>> {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        Ok(store
            .sessions
            .iter()
            .filter(|((app, user, _), _)| app == app_name && user == user_id)
            .map(|(_, session)| {
                // Listings omit history: callers use this to pick a thread, and
                // materializing every event would be wasteful.
                let mut summary = session.clone();
                summary.events.clear();
                Self::hydrate(&store, &mut summary);
                summary
            })
            .collect())
    }

    async fn delete_session(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            app_name.to_string(),
            user_id.to_string(),
            session_id.to_string(),
        );
        store
            .sessions
            .remove(&key)
            .map(|_| ())
            .ok_or_else(|| AdkError::SessionNotFound(session_id.to_string()))
    }

    async fn append_event(&self, session: &mut Session, event: Event) -> Result<()> {
        // A partial event is a streaming chunk. Forward it, but do not commit
        // its actions or record it — the final aggregated event carries both.
        if event.is_partial() {
            return Ok(());
        }

        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            session.app_name.clone(),
            session.user_id.clone(),
            session.id.clone(),
        );
        let stored = store
            .sessions
            .get_mut(&key)
            .ok_or_else(|| AdkError::SessionNotFound(session.id.clone()))?;

        // Route the delta by prefix. `temp:` keys stay on the event for
        // observability but are never persisted anywhere.
        let mut app_writes = Vec::new();
        let mut user_writes = Vec::new();
        for (state_key, value) in &event.actions.state_delta {
            match StateScope::of(state_key) {
                StateScope::App => app_writes.push((state_key.clone(), value.clone())),
                StateScope::User => user_writes.push((state_key.clone(), value.clone())),
                StateScope::Temp => {}
                StateScope::Session => {
                    stored.state.commit([(state_key.clone(), value.clone())]);
                }
            }
        }

        stored.events.push(event.clone());
        stored.last_update_time = adk_core::now_seconds();
        let mut updated = stored.clone();

        if !app_writes.is_empty() {
            store
                .app_state
                .entry(session.app_name.clone())
                .or_default()
                .extend(app_writes);
        }
        if !user_writes.is_empty() {
            store
                .user_state
                .entry((session.app_name.clone(), session.user_id.clone()))
                .or_default()
                .extend(user_writes);
        }

        // Reflect the committed result back into the caller's handle, so code
        // resuming after the yield observes persisted state.
        Self::hydrate(&store, &mut updated);
        *session = updated;
        Ok(())
    }
}

/// An [`ArtifactService`] backed by process memory, versioning each filename.
#[derive(Default)]
pub struct InMemoryArtifactService {
    /// Keyed by (app, user, session-or-empty, filename) to a version list.
    artifacts: Mutex<BTreeMap<(String, String, String, String), Vec<Part>>>,
}

impl InMemoryArtifactService {
    /// Builds an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Artifacts named `user:<name>` are shared across a user's sessions,
    /// mirroring the `user:` state prefix. Others are session-scoped.
    fn scope_key(session_id: &str, filename: &str) -> String {
        if filename.starts_with(USER_PREFIX) {
            String::new()
        } else {
            session_id.to_string()
        }
    }
}

#[async_trait]
impl ArtifactService for InMemoryArtifactService {
    async fn save_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
        part: Part,
    ) -> Result<u64> {
        let mut store = self.artifacts.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            app_name.to_string(),
            user_id.to_string(),
            Self::scope_key(session_id, filename),
            filename.to_string(),
        );
        let versions = store.entry(key).or_default();
        versions.push(part);
        Ok(versions.len() as u64 - 1)
    }

    async fn load_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
        version: Option<u64>,
    ) -> Result<Option<Part>> {
        let store = self.artifacts.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            app_name.to_string(),
            user_id.to_string(),
            Self::scope_key(session_id, filename),
            filename.to_string(),
        );
        let Some(versions) = store.get(&key) else {
            return Ok(None);
        };
        Ok(match version {
            Some(v) => versions.get(v as usize).cloned(),
            None => versions.last().cloned(),
        })
    }

    async fn list_artifact_keys(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<Vec<String>> {
        let store = self.artifacts.lock().unwrap_or_else(|e| e.into_inner());
        let mut keys: Vec<String> = store
            .keys()
            .filter(|(app, user, scope, _)| {
                app == app_name
                    && user == user_id
                    && (scope.is_empty() || scope == session_id)
            })
            .map(|(_, _, _, filename)| filename.clone())
            .collect();
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    async fn delete_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
    ) -> Result<()> {
        let mut store = self.artifacts.lock().unwrap_or_else(|e| e.into_inner());
        let key = (
            app_name.to_string(),
            user_id.to_string(),
            Self::scope_key(session_id, filename),
            filename.to_string(),
        );
        store
            .remove(&key)
            .map(|_| ())
            .ok_or_else(|| AdkError::ArtifactNotFound(filename.to_string()))
    }
}

/// A [`MemoryService`] backed by process memory with keyword matching.
///
/// Retrieval is a case-insensitive term overlap, not a semantic search. It is
/// enough to exercise memory-dependent agent logic in tests; swap in a real
/// vector store for production recall.
#[derive(Default)]
pub struct InMemoryMemoryService {
    entries: Mutex<BTreeMap<(String, String), Vec<MemoryEntry>>>,
}

impl InMemoryMemoryService {
    /// Builds an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn terms(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    }
}

#[async_trait]
impl MemoryService for InMemoryMemoryService {
    async fn add_session_to_memory(&self, session: &Session) -> Result<()> {
        let mut store = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = store
            .entry((session.app_name.clone(), session.user_id.clone()))
            .or_default();

        for event in &session.events {
            if event.is_partial() {
                continue;
            }
            let Some(content) = &event.content else {
                continue;
            };
            if content.text().trim().is_empty() {
                continue;
            }
            bucket.push(MemoryEntry {
                content: content.clone(),
                author: event.author.clone(),
                timestamp: event.timestamp,
                score: None,
            });
        }
        Ok(())
    }

    async fn search_memory(
        &self,
        app_name: &str,
        user_id: &str,
        query: &str,
    ) -> Result<Vec<MemoryEntry>> {
        let store = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let Some(bucket) = store.get(&(app_name.to_string(), user_id.to_string())) else {
            return Ok(Vec::new());
        };

        let query_terms = Self::terms(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits: Vec<MemoryEntry> = bucket
            .iter()
            .filter_map(|entry| {
                let text = entry.content.text();
                let entry_terms = Self::terms(&text);
                let overlap = query_terms
                    .iter()
                    .filter(|t| entry_terms.contains(t))
                    .count();
                (overlap > 0).then(|| MemoryEntry {
                    score: Some(overlap as f64 / query_terms.len() as f64),
                    ..entry.clone()
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(hits)
    }
}

/// Convenience: an in-memory memory entry from raw text.
pub fn memory_entry(author: impl Into<String>, text: impl Into<String>) -> MemoryEntry {
    MemoryEntry {
        content: Content::user_text(text),
        author: author.into(),
        timestamp: adk_core::now_seconds(),
        score: None,
    }
}

/// Re-exported so downstream crates can name the app-state prefix.
pub const APP_STATE_PREFIX: &str = APP_PREFIX;
