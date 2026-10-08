//! The sans-IO half of a client: request ids, the handshake, and the
//! per-request `_meta` of the stateless revision. It never reads or writes; a
//! transport carries the [`Message`]s it makes and hands back the ones that
//! arrive.

use crate::error::ClientError;
use rusty_json::Value;
use rusty_mcp_proto::lifecycle::{self, method};
use rusty_mcp_proto::{
    ClientCapabilities, DiscoverResult, ErrorData, Implementation, InitializeParams,
    InitializeResult, Message, ProtocolVersion, RequestId, ServerCapabilities, Wire,
};
use std::collections::HashSet;

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_INFO: &str = "io.modelcontextprotocol/clientInfo";
const META_CAPS: &str = "io.modelcontextprotocol/clientCapabilities";

/// Who the client is and which revisions it will speak.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Name and version sent to the server.
    pub info: Implementation,
    /// What the client can do (for example `elicitation`, or the tasks
    /// extension under `extensions`).
    pub capabilities: ClientCapabilities,
    /// Revisions in order of preference, newest first.
    pub versions: Vec<ProtocolVersion>,
}

impl ClientConfig {
    /// A client called `name`, speaking every revision this crate knows,
    /// stateless first.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            info: Implementation::new(name, version),
            capabilities: ClientCapabilities::default(),
            versions: [
                ProtocolVersion::V_2026_07_28,
                ProtocolVersion::V_2025_11_25,
                ProtocolVersion::V_2025_06_18,
                ProtocolVersion::V_2025_03_26,
            ]
            .map(ProtocolVersion::new)
            .to_vec(),
        }
    }
}

/// What arrived, classified.
#[derive(Debug)]
pub enum Incoming {
    /// The answer to a request this session made.
    Response {
        /// Which request.
        id: RequestId,
        /// The result, or the server's error.
        result: Result<Value, ErrorData>,
    },
    /// A notification from the server.
    Notification {
        /// Its method.
        method: String,
        /// Its parameters.
        params: Option<Value>,
    },
    /// A request from the server, which wants an answer.
    Request {
        /// Its id.
        id: RequestId,
        /// Its method.
        method: String,
        /// Its parameters.
        params: Option<Value>,
    },
    /// A response nobody asked for, or an error with no id.
    Stray(Message),
}

/// One client's side of a conversation.
#[derive(Debug)]
pub struct ClientSession {
    config: ClientConfig,
    next_id: i64,
    pending: HashSet<RequestId>,
    negotiated: Option<ProtocolVersion>,
    server_info: Option<Implementation>,
    server_capabilities: Option<ServerCapabilities>,
    instructions: Option<String>,
}

impl ClientSession {
    /// A session before any handshake.
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            next_id: 0,
            pending: HashSet::new(),
            negotiated: None,
            server_info: None,
            server_capabilities: None,
            instructions: None,
        }
    }

    /// The revision in force, once the handshake is done.
    pub fn negotiated(&self) -> Option<&ProtocolVersion> {
        self.negotiated.as_ref()
    }

    /// Whether requests carry their own `_meta` (2026-07-28 and later).
    pub fn is_stateless(&self) -> bool {
        self.negotiated
            .as_ref()
            .is_some_and(ProtocolVersion::is_stateless)
    }

    /// Who the server says it is.
    pub fn server_info(&self) -> Option<&Implementation> {
        self.server_info.as_ref()
    }

    /// What the server says it can do.
    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_capabilities.as_ref()
    }

    /// The server's usage hints for a model.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Whether the config prefers a stateless revision, so the handshake
    /// should try `server/discover` first.
    pub fn prefers_stateless(&self) -> bool {
        self.config
            .versions
            .iter()
            .any(ProtocolVersion::is_stateless)
    }

    /// Whether any classic revision is configured, so `initialize` can be the
    /// fallback.
    pub fn allows_classic(&self) -> bool {
        self.config.versions.iter().any(|v| !v.is_stateless())
    }

    fn allocate(&mut self) -> RequestId {
        self.next_id += 1;
        let id = RequestId::Number(self.next_id);
        self.pending.insert(id.clone());
        id
    }

    /// `server/discover`.
    pub fn discover_request(&mut self) -> (RequestId, Message) {
        let id = self.allocate();
        let message = Message::Request {
            id: id.clone(),
            method: method::DISCOVER.to_owned(),
            params: None,
        };
        (id, message)
    }

    /// Read the answer to `server/discover`: pick the newest revision both
    /// sides speak. Returns `false` when that revision is a classic one, which
    /// `initialize` must still settle: the server is not stateless after all.
    ///
    /// # Errors
    /// [`ClientError::NoCommonVersion`], or a malformed result.
    pub fn on_discover(&mut self, result: &Value) -> Result<bool, ClientError> {
        let found = DiscoverResult::from_value(result)?;
        let chosen = lifecycle::pick_common(&self.config.versions, &found.supported_versions)
            .ok_or(ClientError::NoCommonVersion)?;
        if !chosen.is_stateless() {
            return Ok(false);
        }
        self.server_info = found.server_info();
        self.server_capabilities = Some(found.capabilities);
        self.instructions = found.instructions;
        self.negotiated = Some(chosen);
        Ok(true)
    }

    /// Whether an error to `server/discover` means "this server is classic".
    pub fn falls_back_to_initialize(&self, error: &ErrorData) -> bool {
        self.allows_classic() && lifecycle::falls_back_to_initialize(error)
    }

    /// `initialize`, asking for the newest configured revision (a classic
    /// server answers with the one it picks; a server that also speaks
    /// 2026-07-28 may accept it here).
    ///
    /// # Errors
    /// [`ClientError::NoCommonVersion`] when no revision is configured.
    pub fn initialize_request(&mut self) -> Result<(RequestId, Message), ClientError> {
        let newest = self
            .config
            .versions
            .iter()
            .max()
            .cloned()
            .ok_or(ClientError::NoCommonVersion)?;
        let params = InitializeParams {
            protocol_version: newest,
            capabilities: self.config.capabilities.clone(),
            client_info: self.config.info.clone(),
            meta: None,
        };
        let id = self.allocate();
        let message = Message::Request {
            id: id.clone(),
            method: method::INITIALIZE.to_owned(),
            params: Some(params.to_value()),
        };
        Ok((id, message))
    }

    /// Read the answer to `initialize`.
    ///
    /// # Errors
    /// [`ClientError::NoCommonVersion`] if the server chose a revision the
    /// config does not allow.
    pub fn on_initialize(&mut self, result: &Value) -> Result<(), ClientError> {
        let init = InitializeResult::from_value(result)?;
        if !self.config.versions.contains(&init.protocol_version) {
            return Err(ClientError::NoCommonVersion);
        }
        self.negotiated = Some(init.protocol_version);
        self.server_info = Some(init.server_info);
        self.server_capabilities = Some(init.capabilities);
        self.instructions = init.instructions;
        Ok(())
    }

    /// `notifications/initialized`, sent after a classic `initialize`.
    pub fn initialized_notification(&self) -> Message {
        Message::Notification {
            method: method::INITIALIZED.to_owned(),
            params: None,
        }
    }

    /// A request. On a stateless revision its `_meta` names the revision, the
    /// client and its capabilities (keeping anything the caller put there,
    /// such as a progress token).
    pub fn request(&mut self, method: &str, params: Option<Value>) -> (RequestId, Message) {
        let params = if self.is_stateless() {
            Some(self.with_meta(params))
        } else {
            params
        };
        let id = self.allocate();
        let message = Message::Request {
            id: id.clone(),
            method: method.to_owned(),
            params,
        };
        (id, message)
    }

    /// A notification. Notifications carry no `_meta` of their own.
    pub fn notification(&self, method: &str, params: Option<Value>) -> Message {
        Message::Notification {
            method: method.to_owned(),
            params,
        }
    }

    /// `notifications/cancelled` for a request still in flight.
    pub fn cancel(&self, id: &RequestId, reason: Option<&str>) -> Message {
        let mut params = Value::object();
        params.insert("requestId", id.to_value());
        if let Some(reason) = reason {
            params.insert("reason", reason);
        }
        self.notification("notifications/cancelled", Some(params))
    }

    fn with_meta(&self, params: Option<Value>) -> Value {
        let mut params = match params {
            Some(p) if p.as_object().is_some() => p,
            _ => Value::object(),
        };
        let mut meta = params
            .get("_meta")
            .filter(|m| m.as_object().is_some())
            .cloned()
            .unwrap_or_else(Value::object);
        if let Some(version) = &self.negotiated {
            meta.insert(META_VERSION, version.as_str());
        }
        meta.insert(META_INFO, self.config.info.to_value());
        meta.insert(META_CAPS, self.config.capabilities.to_value());
        params.insert("_meta", meta);
        params
    }

    /// Classify a message that arrived, forgetting the request it answers.
    pub fn accept(&mut self, message: Message) -> Incoming {
        match message {
            Message::Response { id, result } if self.pending.remove(&id) => Incoming::Response {
                id,
                result: Ok(result),
            },
            Message::Error {
                id: Some(id),
                error,
            } if self.pending.remove(&id) => Incoming::Response {
                id,
                result: Err(error),
            },
            Message::Notification { method, params } => Incoming::Notification { method, params },
            Message::Request { id, method, params } => Incoming::Request { id, method, params },
            other => Incoming::Stray(other),
        }
    }

    /// Stop waiting for `id` (the call timed out or was cancelled), so a
    /// late answer is a [`Incoming::Stray`] rather than a mix-up.
    pub fn forget(&mut self, id: &RequestId) {
        self.pending.remove(id);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn cfg() -> ClientConfig {
        ClientConfig::new("c", "1")
    }

    #[test]
    fn ids_increase_and_answers_are_matched_once() {
        let mut s = ClientSession::new(cfg());
        let (a, _) = s.request("ping", None);
        let (b, _) = s.request("ping", None);
        assert_ne!(a, b);
        let reply = Message::Response {
            id: a.clone(),
            result: Value::object(),
        };
        assert!(matches!(s.accept(reply.clone()), Incoming::Response { .. }));
        assert!(
            matches!(s.accept(reply), Incoming::Stray(_)),
            "answered twice"
        );
    }

    #[test]
    fn a_stateless_request_carries_its_meta_and_keeps_the_callers() {
        let mut s = ClientSession::new(cfg());
        s.negotiated = Some(ProtocolVersion::new("2026-07-28"));
        let mut params = Value::object();
        let mut meta = Value::object();
        meta.insert("progressToken", "t1");
        params.insert("_meta", meta);
        params.insert("name", "x");
        let (_, Message::Request { params, .. }) = s.request("tools/call", Some(params)) else {
            panic!("not a request");
        };
        let params = params.unwrap();
        assert_eq!(params["name"].as_str(), Some("x"));
        let meta = &params["_meta"];
        assert_eq!(meta["progressToken"].as_str(), Some("t1"));
        assert_eq!(meta[META_VERSION].as_str(), Some("2026-07-28"));
        assert_eq!(meta[META_INFO]["name"].as_str(), Some("c"));
        assert!(meta[META_CAPS].as_object().is_some());
    }

    #[test]
    fn a_classic_request_is_sent_as_given() {
        let mut s = ClientSession::new(cfg());
        s.negotiated = Some(ProtocolVersion::new("2025-06-18"));
        let (_, Message::Request { params, .. }) = s.request("tools/list", None) else {
            panic!("not a request");
        };
        assert!(params.is_none());
    }

    #[test]
    fn discover_picks_the_newest_common_revision_or_fails() {
        let mut s = ClientSession::new(cfg());
        let found = DiscoverResult::new(
            vec![
                ProtocolVersion::new("2025-06-18"),
                ProtocolVersion::new("2026-07-28"),
            ],
            ServerCapabilities::default(),
        );
        assert!(s.on_discover(&found.to_value()).unwrap());
        assert_eq!(s.negotiated().unwrap().as_str(), "2026-07-28");
        assert!(s.is_stateless());

        // A server that answers discover but lists only classic revisions is
        // not stateless: initialize must settle it.
        let mut classic = ClientSession::new(cfg());
        let found = DiscoverResult::new(
            vec![ProtocolVersion::new("2025-06-18")],
            ServerCapabilities::default(),
        );
        assert!(!classic.on_discover(&found.to_value()).unwrap());
        assert!(classic.negotiated().is_none());

        let mut only_old = ClientSession::new(cfg());
        let found = DiscoverResult::new(
            vec![ProtocolVersion::new("2024-11-05")],
            ServerCapabilities::default(),
        );
        assert!(matches!(
            only_old.on_discover(&found.to_value()),
            Err(ClientError::NoCommonVersion)
        ));
    }

    #[test]
    fn initialize_asks_for_the_newest_and_refuses_an_unconfigured_answer() {
        let mut s = ClientSession::new(cfg());
        let (_, request) = s.initialize_request().unwrap();
        let Message::Request { params, .. } = request else {
            panic!("not a request");
        };
        assert_eq!(
            params.unwrap()["protocolVersion"].as_str(),
            Some("2026-07-28")
        );
        let answer = InitializeResult {
            protocol_version: ProtocolVersion::new("2024-11-05"),
            capabilities: ServerCapabilities::default(),
            server_info: Implementation::new("s", "1"),
            instructions: None,
            meta: None,
        };
        assert!(matches!(
            s.on_initialize(&answer.to_value()),
            Err(ClientError::NoCommonVersion)
        ));
    }

    #[test]
    fn cancel_names_the_request() {
        let s = ClientSession::new(cfg());
        let Message::Notification { method, params } = s.cancel(&RequestId::Number(4), Some("x"))
        else {
            panic!("not a notification");
        };
        assert_eq!(method, "notifications/cancelled");
        assert_eq!(params.unwrap()["requestId"].as_i64(), Some(4));
    }
}
