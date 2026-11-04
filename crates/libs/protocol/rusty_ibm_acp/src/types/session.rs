//! Distributed session types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::error::Error;

/// Identifier of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Generate a fresh random identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// The underlying UUID.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for SessionId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for SessionId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|err| Error::invalid_input(format!("invalid SessionId {s:?}: {err}")))
    }
}

/// A conversation spanning multiple runs, possibly across several ACP servers.
///
/// `history` and `state` hold *URLs*, not inline data: message bodies and agent
/// state live on resource servers so a session can move between server
/// instances. Use [`crate::client::AcpClient::fetch_session_history`] to
/// materialise the history into messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Unique identifier of the session.
    pub id: SessionId,
    /// URLs of the messages exchanged so far, in order.
    pub history: Vec<String>,
    /// URL of arbitrary state explicitly saved by an agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl Session {
    /// An empty session with a fresh identifier.
    pub fn new() -> Self {
        Self { id: SessionId::new(), history: Vec::new(), state: None }
    }

    /// An empty session with the given identifier.
    pub fn with_id(id: SessionId) -> Self {
        Self { id, history: Vec::new(), state: None }
    }

    /// Append a message URL to the history.
    pub fn push_history(&mut self, url: impl Into<String>) {
        self.history.push(url.into());
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
