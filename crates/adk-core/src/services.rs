//! Service traits the runtime depends on: sessions, artifacts, and memory.
//!
//! The traits live here, in the data-model crate, so that every other crate
//! can depend on the abstraction without depending on a particular backend.
//! In-memory implementations ship in `adk-sessions`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::content::Part;
use crate::error::Result;
use crate::event::Event;
use crate::session::Session;
use crate::state::State;

/// Persists conversation threads and applies the state deltas events carry.
#[async_trait]
pub trait SessionService: Send + Sync {
    /// Creates a session, optionally with starting state and a caller-chosen id.
    async fn create_session(
        &self,
        app_name: &str,
        user_id: &str,
        state: Option<State>,
        session_id: Option<String>,
    ) -> Result<Session>;

    /// Loads a session, or `None` if it does not exist.
    async fn get_session(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<Option<Session>>;

    /// Lists a user's sessions. Implementations may omit event history.
    async fn list_sessions(&self, app_name: &str, user_id: &str) -> Result<Vec<Session>>;

    /// Deletes a session and its history.
    async fn delete_session(&self, app_name: &str, user_id: &str, session_id: &str)
        -> Result<()>;

    /// Appends an event, merging its `state_delta` into the session.
    ///
    /// This is the only supported way to mutate state: writing to
    /// `session.state` directly bypasses event history and will not persist.
    /// Implementations must honour the prefix rules in [`crate::state`] —
    /// `temp:` keys are recorded on the event but never committed.
    ///
    /// The passed session is updated in place so the caller sees the result.
    async fn append_event(&self, session: &mut Session, event: Event) -> Result<()>;
}

/// A stored binary artifact, versioned per filename.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    /// Filename the artifact is stored under.
    pub filename: String,
    /// Monotonically increasing version, starting at 0.
    pub version: u64,
    /// The stored payload.
    pub part: Part,
}

/// Stores binary payloads that are too large or too opaque for session state.
#[async_trait]
pub trait ArtifactService: Send + Sync {
    /// Saves an artifact and returns its new version number.
    async fn save_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
        part: Part,
    ) -> Result<u64>;

    /// Loads an artifact, defaulting to the latest version.
    async fn load_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
        version: Option<u64>,
    ) -> Result<Option<Part>>;

    /// Lists the filenames visible to a session.
    async fn list_artifact_keys(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<Vec<String>>;

    /// Deletes every version of an artifact.
    async fn delete_artifact(
        &self,
        app_name: &str,
        user_id: &str,
        session_id: &str,
        filename: &str,
    ) -> Result<()>;
}

/// One hit from a memory search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// The remembered content.
    pub content: crate::content::Content,
    /// Who authored it.
    pub author: String,
    /// When it was recorded, in seconds since the Unix epoch.
    pub timestamp: f64,
    /// Backend-specific relevance, where the backend reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// Long-term recall across sessions.
#[async_trait]
pub trait MemoryService: Send + Sync {
    /// Ingests a completed session into long-term memory.
    async fn add_session_to_memory(&self, session: &Session) -> Result<()>;

    /// Searches a user's memory.
    async fn search_memory(
        &self,
        app_name: &str,
        user_id: &str,
        query: &str,
    ) -> Result<Vec<MemoryEntry>>;
}

/// The service bundle handed to every invocation.
#[derive(Clone)]
pub struct Services {
    /// Session persistence. Always present.
    pub session: Arc<dyn SessionService>,
    /// Artifact storage, when configured.
    pub artifact: Option<Arc<dyn ArtifactService>>,
    /// Long-term memory, when configured.
    pub memory: Option<Arc<dyn MemoryService>>,
}

impl Services {
    /// Bundles a session service with no artifact or memory backend.
    pub fn new(session: Arc<dyn SessionService>) -> Self {
        Self {
            session,
            artifact: None,
            memory: None,
        }
    }

    /// Adds an artifact backend.
    pub fn with_artifact(mut self, artifact: Arc<dyn ArtifactService>) -> Self {
        self.artifact = Some(artifact);
        self
    }

    /// Adds a memory backend.
    pub fn with_memory(mut self, memory: Arc<dyn MemoryService>) -> Self {
        self.memory = Some(memory);
        self
    }
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("artifact", &self.artifact.is_some())
            .field("memory", &self.memory.is_some())
            .finish_non_exhaustive()
    }
}
