//! The I/O shell: render, run `ollama`, parse.

use std::time::Duration;

use orch_core::board::Board;
use orch_core::task::{Agent, Role, Task};
use orch_dispatch::{AgentError, AgentRunner, Output};

use orch_cli::{excerpt, parse, CommandRunner, ExecError, StdCommand};

use crate::render;

/// [`Agent::Local`] over `ollama run <model> --format json`, prompt on stdin.
///
/// Serves non-implementation roles through `Agent::Local`; unsupported
/// agent/role pairs are rejected before any process is spawned. The argv is
/// a fixed vector with no shell and no interpolation.
#[derive(Debug, Clone)]
pub struct OllamaAgent<C = StdCommand> {
    model: String,
    timeout: Duration,
    runner: C,
}

impl OllamaAgent<StdCommand> {
    /// An agent over the real `ollama` binary on `PATH`.
    pub fn new(model: impl Into<String>, timeout: Duration) -> Self {
        Self::with_runner(model, timeout, StdCommand)
    }
}

impl<C: CommandRunner> OllamaAgent<C> {
    /// An agent over any [`CommandRunner`], e.g. a fake in tests.
    pub fn with_runner(model: impl Into<String>, timeout: Duration, runner: C) -> Self {
        Self {
            model: model.into(),
            timeout,
            runner,
        }
    }

    /// The runner, e.g. to inspect a fake in tests.
    pub fn runner(&self) -> &C {
        &self.runner
    }

    /// The argv this agent runs. Public so tests and docs can pin it.
    pub fn argv(&self) -> Vec<String> {
        ["ollama", "run", self.model.as_str(), "--format", "json"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

impl<C: CommandRunner> AgentRunner for OllamaAgent<C> {
    fn supports(&self, agent: Agent, role: Role) -> bool {
        agent == Agent::Local && !matches!(role, Role::Implement)
    }

    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError> {
        if !self.supports(agent, task.spec().role) {
            return Err(AgentError::Permanent(format!(
                "ollama adapter cannot serve {:?} through {agent:?}",
                task.spec().role
            )));
        }
        let prompt = render(task, board);
        // Ollama has no API-key path, so nothing is scrubbed from the child.
        let exit = self
            .runner
            .run_scrubbed(&self.argv(), prompt.as_bytes(), self.timeout, &[])
            .map_err(exec_error)?;
        if exit.status != 0 {
            return Err(AgentError::Transient(format!(
                "ollama exited with status {}: {}",
                exit.status,
                excerpt(&exit.stderr)
            )));
        }
        let stdout = String::from_utf8_lossy(&exit.stdout);
        if stdout.trim().is_empty() {
            return Err(AgentError::Transient(
                "ollama wrote nothing to stdout".to_owned(),
            ));
        }
        parse(&stdout, task.spec().role)
    }
}

fn exec_error(e: ExecError) -> AgentError {
    AgentError::Transient(format!("ollama: {e}"))
}
