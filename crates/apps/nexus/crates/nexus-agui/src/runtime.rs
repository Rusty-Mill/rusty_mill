//! The port to the Nexus runtime, and its kernel adapter.
//!
//! [`NexusAgent`](crate::NexusAgent) needs three things of the runtime:
//! submit a session, watch the typed event stream, and deliver a round
//! decision. [`Runtime`] names them; [`KernelRuntime`] does them over a
//! plugin context's IPC and bus. Tests supply a scripted one.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use nexus_ai_runtime::events::AiEvent;
use nexus_ai_runtime::{AiRuntimeSubmitReply, BUS_TOPIC_PREFIX};
use nexus_kernel::{EventFilter, Events as _, Ipc as _, KernelPluginContext, NexusEvent};
use serde_json::{json, Value};

/// The agent plugin whose `round_decide` lifts an approval gate.
pub const AGENT_PLUGIN_ID: &str = "com.nexus.agent";

/// What the agent asks of the runtime.
#[async_trait]
pub trait Runtime: Send + Sync + 'static {
    /// Starts watching the typed event stream. Called before [`submit`](Self::submit)
    /// so the first events are not missed; the stream carries every task's
    /// events and the agent filters by task id.
    fn subscribe(&self) -> BoxStream<'static, AiEvent>;

    /// Submits one agent session and returns its task id.
    async fn submit(&self, args: Value) -> Result<uuid::Uuid, String>;

    /// Delivers a round decision to the session waiting on it.
    async fn decide(&self, session_id: &str, decision: Value) -> Result<(), String>;
}

/// [`Runtime`] over a kernel plugin context: `com.nexus.ai.runtime::submit`,
/// the `com.nexus.ai.runtime.*` bus topics, and `com.nexus.agent::round_decide`.
///
/// The context needs `ipc.call` and the runtime's `ai.runtime.submit`
/// capability; `nexus_bootstrap`'s invoker context has them.
pub struct KernelRuntime {
    context: Arc<KernelPluginContext>,
    ipc_timeout: Duration,
}

impl KernelRuntime {
    /// Wraps a plugin context. IPC calls time out after thirty seconds;
    /// the session itself runs on the runtime's pool, not inside the call.
    #[must_use]
    pub fn new(context: Arc<KernelPluginContext>) -> Self {
        Self {
            context,
            ipc_timeout: Duration::from_secs(30),
        }
    }

    /// Sets the IPC timeout for `submit` and `round_decide`.
    #[must_use]
    pub fn with_ipc_timeout(mut self, timeout: Duration) -> Self {
        self.ipc_timeout = timeout;
        self
    }
}

#[async_trait]
impl Runtime for KernelRuntime {
    fn subscribe(&self) -> BoxStream<'static, AiEvent> {
        let subscription = self
            .context
            .subscribe(EventFilter::CustomPrefix(BUS_TOPIC_PREFIX.to_string()));
        futures::stream::unfold(subscription, |mut subscription| async move {
            loop {
                match subscription.recv().await {
                    Ok(published) => {
                        if let NexusEvent::Custom { payload, .. } = &published.event {
                            if let Ok(event) = serde_json::from_value::<AiEvent>(payload.clone()) {
                                return Some((event, subscription));
                            }
                        }
                        // Not a typed event (or one this build cannot read): skip it.
                    }
                    // A lagging subscriber lost events; the ones still coming are
                    // worth more than giving up.
                    Err(nexus_kernel::RecvError::Lagged(_)) => {}
                    Err(nexus_kernel::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }

    async fn submit(&self, args: Value) -> Result<uuid::Uuid, String> {
        let body = json!({ "task": { "kind": "session", "args": args } });
        let reply = self
            .context
            .ipc_call(
                nexus_ai_runtime::PLUGIN_ID,
                "submit",
                body,
                self.ipc_timeout,
            )
            .await
            .map_err(|error| format!("submit: {error}"))?;
        let reply: AiRuntimeSubmitReply =
            serde_json::from_value(reply).map_err(|error| format!("submit reply: {error}"))?;
        Ok(reply.task_id)
    }

    async fn decide(&self, session_id: &str, decision: Value) -> Result<(), String> {
        let mut body = decision;
        if let Value::Object(map) = &mut body {
            map.insert("session_id".into(), Value::String(session_id.to_string()));
        }
        self.context
            .ipc_call(AGENT_PLUGIN_ID, "round_decide", body, self.ipc_timeout)
            .await
            .map(|_| ())
            .map_err(|error| format!("round_decide: {error}"))
    }
}
