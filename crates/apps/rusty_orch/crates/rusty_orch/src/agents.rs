//! The composite runner: one adapter per agent, chosen by the agent the
//! dispatcher routed. Forwards `run_classified`, so a recoverable "not
//! logged in" reaches the dispatcher intact (ADR-0008).

use std::path::PathBuf;
use std::time::Duration;

use orch_claude::ClaudeAgent;
use orch_codex::CodexAgent;
use orch_core::board::Board;
use orch_core::goal::StopRule;
use orch_core::task::{Agent, Role, Task};
use orch_dispatch::{AgentError, AgentRunner, ClassifiedError, Output};
use orch_ollama::OllamaAgent;

/// How to reach the three real backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsConfig {
    pub ollama_model: String,
    pub ollama_timeout: Duration,
    /// The repository Codex and Claude read, as their working directory.
    pub repo: PathBuf,
    pub codex_model: Option<String>,
    pub claude_model: Option<String>,
    /// The goal's stop rule, handed to every adapter (ADR-0011).
    pub stop: StopRule,
}

/// `Agent::Codex`, `Agent::Claude`, and `Agent::Local`; `Agent::Gemini` has
/// no adapter here.
pub struct Agents {
    codex: CodexAgent,
    claude: ClaudeAgent,
    local: OllamaAgent,
}

impl Agents {
    /// Adapters over the real binaries on `PATH`.
    pub fn new(config: AgentsConfig) -> Self {
        let mut codex = CodexAgent::new(config.repo.clone()).with_stop_rule(config.stop);
        if let Some(model) = config.codex_model {
            codex = codex.model(model);
        }
        let mut claude = ClaudeAgent::new(config.repo).with_stop_rule(config.stop);
        if let Some(model) = config.claude_model {
            claude = claude.model(model);
        }
        Self {
            codex,
            claude,
            local: OllamaAgent::new(config.ollama_model, config.ollama_timeout)
                .with_stop_rule(config.stop),
        }
    }

    fn adapter(&mut self, agent: Agent) -> Option<&mut dyn AgentRunner> {
        match agent {
            Agent::Codex => Some(&mut self.codex),
            Agent::Claude => Some(&mut self.claude),
            Agent::Local => Some(&mut self.local),
            Agent::Gemini => None,
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
            Agent::Claude => self.claude.supports(agent, role),
            Agent::Local => self.local.supports(agent, role),
            Agent::Gemini => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    use orch_core::task::{Plan, TaskSpec};
    use orch_core::{GoalId, Text};

    fn agents() -> Agents {
        Agents::new(AgentsConfig {
            ollama_model: "m".to_owned(),
            ollama_timeout: Duration::from_secs(1),
            repo: PathBuf::from("."),
            codex_model: None,
            claude_model: Some("opus".to_owned()),
            stop: StopRule::Checkpoint,
        })
    }

    #[test]
    fn three_agents_have_adapters_and_gemini_does_not() {
        let a = agents();
        for agent in [Agent::Codex, Agent::Claude, Agent::Local] {
            assert!(a.supports(agent, Role::Research), "{agent:?}");
            assert!(!a.supports(agent, Role::Implement), "{agent:?}");
        }
        assert!(!a.supports(Agent::Gemini, Role::Research));
    }

    #[test]
    fn a_missing_adapter_is_a_permanent_failure_before_any_process() {
        let mut plan = Plan::new(GoalId::from_raw(1));
        let id = plan
            .add(TaskSpec {
                role: Role::Research,
                instruction: Text::new("i").expect("non-blank"),
                acceptance: vec![Text::new("a").expect("non-blank")],
                refs: vec![],
                depends_on: vec![],
                max_calls: NonZeroU32::new(1).expect("non-zero"),
            })
            .expect("add");
        let board = Board::new(plan.goal());
        let err = agents()
            .run_classified(Agent::Gemini, plan.get(id).expect("task"), &board)
            .expect_err("no adapter");
        assert!(matches!(err, ClassifiedError::Permanent(e) if e.0 == "no adapter for Gemini"));
    }
}
