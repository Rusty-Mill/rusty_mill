//! [`ToolContext`] — what a tool sees when it runs.

use adk_core::{
    AdkError, Args, ArtifactVersion, Content, EventActions, InvocationContext, MemoryEntry, Part,
    Result, State, ToolConfirmation,
};
use serde_json::Value;
use std::sync::{Arc, RwLock};

/// The context handed to a tool for one invocation of that tool.
///
/// It exposes the session state, the artifact and memory services, and the
/// [`EventActions`] block that will ride out on the event carrying this tool's
/// result. Mutating actions here is how a tool influences the agent's control
/// flow — skipping summarization, transferring to another agent, or escalating
/// out of a loop.
///
/// Cloning shares the same action accumulator, so a helper that takes a clone
/// still contributes to the same event.
#[derive(Clone)]
pub struct ToolContext {
    /// The enclosing invocation.
    pub invocation: InvocationContext,
    /// Id of the function call being serviced.
    pub function_call_id: Option<String>,
    /// Id of the event that carried the function call.
    pub function_call_event_id: Option<String>,
    /// The user's answer to a confirmation request, on a resumed call.
    ///
    /// `None` on the first call. A tool that requires confirmation should
    /// check this, request confirmation when absent, and read the payload when
    /// present.
    pub tool_confirmation: Option<ToolConfirmation>,
    /// Credentials supplied in response to an auth request, if any.
    pub auth_response: Option<Value>,

    actions: Arc<RwLock<EventActions>>,
}

impl ToolContext {
    /// Builds a context for a tool call within `invocation`.
    pub fn new(invocation: InvocationContext) -> Self {
        Self {
            invocation,
            function_call_id: None,
            function_call_event_id: None,
            tool_confirmation: None,
            auth_response: None,
            actions: Arc::new(RwLock::new(EventActions::default())),
        }
    }

    /// Associates this context with a specific function call.
    pub fn with_function_call_id(mut self, id: impl Into<String>) -> Self {
        self.function_call_id = Some(id.into());
        self
    }

    /// Associates this context with the event that carried the call.
    pub fn with_function_call_event_id(mut self, id: impl Into<String>) -> Self {
        self.function_call_event_id = Some(id.into());
        self
    }

    /// Supplies a confirmation answer, making this a resumed call.
    pub fn with_confirmation(mut self, confirmation: ToolConfirmation) -> Self {
        self.tool_confirmation = Some(confirmation);
        self
    }

    /// Supplies credentials obtained for this call.
    pub fn with_auth_response(mut self, auth: Value) -> Self {
        self.auth_response = Some(auth);
        self
    }

    // ---- state ----

    /// Reads a state key.
    pub fn state(&self, key: &str) -> Option<Value> {
        self.invocation.get_state(key)
    }

    /// Reads a state key as `T`, falling back to `default`.
    pub fn state_or<T: for<'de> serde::Deserialize<'de>>(&self, key: &str, default: T) -> T {
        self.invocation.with_state(|s| s.get_or(key, default))
    }

    /// Stages a state write. Persisted when the resulting event is processed.
    pub fn set_state(&self, key: impl Into<String>, value: impl Into<Value>) {
        self.invocation.set_state(key, value);
    }

    /// Reads or mutates state directly.
    pub fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        self.invocation.with_state_mut(f)
    }

    // ---- actions ----

    /// Reads the accumulated actions.
    pub fn actions(&self) -> EventActions {
        self.actions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Mutates the accumulated actions.
    pub fn with_actions<R>(&self, f: impl FnOnce(&mut EventActions) -> R) -> R {
        let mut guard = self.actions.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Returns this tool's result to the caller verbatim, without asking the
    /// model to summarize it.
    pub fn skip_summarization(&self) {
        self.with_actions(|a| a.skip_summarization = true);
    }

    /// Hands control to another agent by name.
    pub fn transfer_to_agent(&self, agent_name: impl Into<String>) {
        let name = agent_name.into();
        self.with_actions(|a| a.transfer_to_agent = Some(name));
    }

    /// Terminates the enclosing loop, or escalates to the parent agent.
    pub fn escalate(&self) {
        self.with_actions(|a| a.escalate = true);
    }

    /// Asks the user to approve this tool call.
    ///
    /// Records the request on the outgoing event and returns
    /// [`AdkError::ConfirmationRequired`]. The tool should propagate that
    /// error; the runtime suspends the call and re-runs it with
    /// [`ToolContext::tool_confirmation`] populated once the user answers.
    pub fn request_confirmation(
        &self,
        hint: impl Into<String>,
        payload: Option<Value>,
    ) -> AdkError {
        let call_id = self
            .function_call_id
            .clone()
            .unwrap_or_else(|| adk_core::new_id("call"));
        let mut confirmation = ToolConfirmation::new(hint);
        confirmation.payload = payload;
        self.with_actions(|a| {
            a.requested_tool_confirmations
                .insert(call_id.clone(), confirmation);
        });
        AdkError::ConfirmationRequired {
            function_call_id: call_id,
        }
    }

    /// Requests credentials for an authenticated call.
    ///
    /// Like [`ToolContext::request_confirmation`], this records the request and
    /// returns an error the tool should propagate.
    pub fn request_credential(&self, auth_config: Value) -> AdkError {
        let call_id = self
            .function_call_id
            .clone()
            .unwrap_or_else(|| adk_core::new_id("call"));
        self.with_actions(|a| {
            a.requested_auth_configs.insert(call_id.clone(), auth_config);
        });
        AdkError::ConfirmationRequired {
            function_call_id: call_id,
        }
    }

    /// Whether the user has approved this call.
    pub fn is_confirmed(&self) -> bool {
        self.tool_confirmation
            .as_ref()
            .map(|c| c.confirmed)
            .unwrap_or(false)
    }

    /// The payload the user submitted alongside their approval.
    pub fn confirmation_payload(&self) -> Option<&Value> {
        self.tool_confirmation.as_ref()?.payload.as_ref()
    }

    // ---- artifacts ----

    /// Saves an artifact and records the new version in the outgoing actions.
    ///
    /// Fails with [`AdkError::Config`] when no artifact service is configured.
    pub async fn save_artifact(&self, filename: &str, part: Part) -> Result<u64> {
        let service = self
            .invocation
            .services()
            .artifact
            .as_ref()
            .ok_or_else(|| AdkError::Config("no artifact service configured".into()))?;
        let version = service
            .save_artifact(
                &self.invocation.app_name,
                &self.invocation.user_id,
                &self.invocation.session_id,
                filename,
                part,
            )
            .await?;
        self.with_actions(|a| {
            a.artifact_delta.insert(filename.to_string(), version);
        });
        Ok(version)
    }

    /// Loads an artifact, defaulting to its latest version.
    pub async fn load_artifact(&self, filename: &str, version: Option<u64>) -> Result<Option<Part>> {
        let service = self
            .invocation
            .services()
            .artifact
            .as_ref()
            .ok_or_else(|| AdkError::Config("no artifact service configured".into()))?;
        service
            .load_artifact(
                &self.invocation.app_name,
                &self.invocation.user_id,
                &self.invocation.session_id,
                filename,
                version,
            )
            .await
    }

    /// Lists the artifacts visible to this session.
    pub async fn list_artifacts(&self) -> Result<Vec<String>> {
        let service = self
            .invocation
            .services()
            .artifact
            .as_ref()
            .ok_or_else(|| AdkError::Config("no artifact service configured".into()))?;
        service
            .list_artifact_keys(
                &self.invocation.app_name,
                &self.invocation.user_id,
                &self.invocation.session_id,
            )
            .await
    }

    // ---- memory ----

    /// Searches long-term memory.
    ///
    /// Fails with [`AdkError::Config`] when no memory service is configured.
    pub async fn search_memory(&self, query: &str) -> Result<Vec<MemoryEntry>> {
        let service = self
            .invocation
            .services()
            .memory
            .as_ref()
            .ok_or_else(|| AdkError::Config("no memory service configured".into()))?;
        service
            .search_memory(&self.invocation.app_name, &self.invocation.user_id, query)
            .await
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("invocation_id", &self.invocation.invocation_id)
            .field("function_call_id", &self.function_call_id)
            .field("confirmed", &self.is_confirmed())
            .finish_non_exhaustive()
    }
}

/// Re-exported for tools that build content directly.
pub type ToolContent = Content;

/// Re-exported for tools that inspect artifact versions.
pub type ToolArtifactVersion = ArtifactVersion;

/// Re-exported so tool crates need not depend on `adk-core` for the arg type.
pub type ToolArgs = Args;
