//! The composite runner: one adapter per agent, chosen by the agent the
//! dispatcher routed. Forwards `run_classified`, so Codex's recoverable
//! "not logged in" reaches the dispatcher intact (ADR-0008).

use std::path::PathBuf;
use std::time::Duration;

use orch_codex::CodexAgent;
use orch_core::board::Board;
use orch_core::goal::StopRule;
use orch_core::task::{Agent, Role, Task};
use orch_dispatch::{AgentError, AgentRunner, ClassifiedError, Output};
use orch_ollama::OllamaAgent;

/// How to reach the two real backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsConfig {
    pub ollama_model: String,
    pub ollama_timeout: Duration,
    pub codex_repo: PathBuf,
    pub codex_model: Option<String>,
    /// The goal's stop rule, handed to both adapters (ADR-0011).
    pub stop: StopRule,
}

/// `Agent::Codex` and `Agent::Local`; anything else has no adapter here.
pub struct Agents {
    codex: CodexAgent,
    local: OllamaAgent,
}

impl Agents {
    /// Adapters over the real binaries on `PATH`.
    pub fn new(config: AgentsConfig) -> Self {
        let mut codex = CodexAgent::new(config.codex_repo).with_stop_rule(config.stop);
        if let Some(model) = config.codex_model {
            codex = codex.model(model);
        }
        Self {
            codex,
            local: OllamaAgent::new(config.ollama_model, config.ollama_timeout)
                .with_stop_rule(config.stop),
        }
    }

    fn adapter(&mut self, agent: Agent) -> Option<&mut dyn AgentRunner> {
        match agent {
            Agent::Codex => Some(&mut self.codex),
            Agent::Local => Some(&mut self.local),
            Agent::Claude | Agent::Gemini => None,
        }
    }
}

fn no_adapter(agent: Agent) -> AgentError {
    AgentError(format!("no adapter for {agent:?}"))
}

impl AgentRunner for Agents {
    fn supports(&self, agent: Agent, role: Role) -> bool {
        match agent {
            Agent::Codex => self.codex.supports(agent, role),
            Agent::Local => self.local.supports(agent, role),
            Agent::Claude | Agent::Gemini => false,
        }
    }

    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError> {
        match self.adapter(agent) {
            Some(inner) => inner.run(agent, task, board),
            None => Err(no_adapter(agent)),
        }
    }

    fn run_classified(
        &mut self,
        agent: Agent,
        task: &Task,
        board: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        match self.adapter(agent) {
            Some(inner) => inner.run_classified(agent, task, board),
            None => Err(ClassifiedError::Permanent(no_adapter(agent))),
        }
    }
}
