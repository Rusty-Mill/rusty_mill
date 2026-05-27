//! `Session` — the centre of the harness (ARCHITECTURE §6). Phase-1 minimal:
//! wires config + a policy-vetted tool registry + a model, and runs one turn per
//! [`Session::send`]. Memory, verification, and the post-turn join land later.

use std::sync::Arc;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_config::{Config, HarnessLevel};
use rk_constrain::{PolicyChain, ToolDispatch, WorkspacePolicy};
use rk_feed::{register_builtins, ToolRegistry};
use rk_kernel::run_turn;

/// A live conversation against a model, bound to one workspace + policy.
pub struct Session<M> {
    model: M,
    dispatch: Arc<dyn ToolDispatch>,
    system: String,
}

impl<M> Session<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone,
{
    /// Build a session: workspace policy + built-in tools + the static system
    /// prompt for the configured harness level.
    pub fn new(config: &Config, model: M) -> Self {
        let policy =
            PolicyChain::new().with(Arc::new(WorkspacePolicy::new(config.workspace.clone())));
        let mut registry = ToolRegistry::new(Arc::new(policy));
        register_builtins(&mut registry, config.workspace.clone());

        Self {
            model,
            dispatch: Arc::new(registry),
            system: system_prompt(config.harness_level),
        }
    }

    /// Run one user turn to completion; returns the model's final reply.
    pub async fn send(&self, prompt: &str) -> Result<String, rk_kernel::KernelError> {
        run_turn(self.model.clone(), &self.system, prompt, self.dispatch.clone()).await
    }

    /// The advertised tool names (for the startup banner / diagnostics).
    pub fn tool_names(&self) -> Vec<String> {
        self.dispatch.schemas().into_iter().map(|(n, _)| n).collect()
    }
}

/// The static, per-session system prompt. Belongs in `feed` long-term; kept
/// minimal here for the Phase-1 skeleton (H1 layer only).
fn system_prompt(level: HarnessLevel) -> String {
    let mut s = String::from(
        "You are Rusty Keys, an autonomous engineering agent operating inside a \
         workspace. Use the provided tools to inspect and act. Prefer minimal, \
         reversible actions and report what you did.",
    );
    if level >= HarnessLevel::H1 {
        s.push_str(
            "\n\nTool-use: call tools by name with JSON arguments. A blocked or \
             errored tool returns a structured result to observe and recover from, \
             not a hard stop. The workspace is the policy boundary.",
        );
    }
    s
}
