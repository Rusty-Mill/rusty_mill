//! The I/O shell: render, run `codex exec`, read the last message, parse.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use orch_core::board::Board;
use orch_core::task::{Agent, Role, Task};
use orch_dispatch::{AgentError, AgentRunner, ClassifiedError, Output};

use orch_cli::{excerpt, parse, CommandRunner, ExecError, StdCommand};

use crate::{render, OUTPUT_SCHEMA};

/// Codex reads the repo and reasons before answering; ten minutes by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Removed from the child's environment on every run. A stray key would
/// route Codex onto the paid API path instead of the subscription login.
pub const SCRUBBED_ENV: &[&str] = &["OPENAI_API_KEY"];

static RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// [`Agent::Codex`] over `codex exec`, read-only, prompt on stdin.
///
/// Serves non-implementation roles through `Agent::Codex`; unsupported
/// agent/role pairs are rejected before scratch files or processes. The
/// argv is a fixed vector with no shell and no interpolation; the model is
/// Codex's default unless [`CodexAgent::model`] sets one.
///
/// `--ignore-rules` matters as much as `--sandbox read-only`: a saved
/// execpolicy allow rule in the user's Codex home can run a matching command
/// outside the sandbox, so user and project `.rules` files are never loaded.
#[derive(Debug, Clone)]
pub struct CodexAgent<C = StdCommand> {
    repo_root: PathBuf,
    timeout: Duration,
    model: Option<String>,
    runner: C,
}

impl CodexAgent<StdCommand> {
    /// An agent over the real `codex` binary on `PATH`, working in `repo_root`.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_runner(repo_root, StdCommand)
    }
}

impl<C: CommandRunner> CodexAgent<C> {
    /// An agent over any [`CommandRunner`], e.g. a fake in tests.
    pub fn with_runner(repo_root: impl Into<PathBuf>, runner: C) -> Self {
        Self {
            repo_root: repo_root.into(),
            timeout: DEFAULT_TIMEOUT,
            model: None,
            runner,
        }
    }

    /// Replace the default ten-minute deadline.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Pin a model instead of Codex's default.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// The runner, e.g. to inspect a fake in tests.
    pub fn runner(&self) -> &C {
        &self.runner
    }

    /// The argv this agent runs for the given scratch files. Public so tests
    /// and docs can pin it.
    pub fn argv(&self, schema: &Path, reply: &Path) -> Vec<String> {
        let mut argv: Vec<String> = [
            "codex",
            "exec",
            "--sandbox",
            "read-only",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "-C",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        argv.push(self.repo_root.display().to_string());
        if let Some(model) = &self.model {
            argv.push("-m".to_owned());
            argv.push(model.clone());
        }
        argv.push("--output-schema".to_owned());
        argv.push(schema.display().to_string());
        argv.push("--output-last-message".to_owned());
        argv.push(reply.display().to_string());
        argv.push("-".to_owned());
        argv
    }
}

impl<C: CommandRunner> AgentRunner for CodexAgent<C> {
    fn supports(&self, agent: Agent, role: Role) -> bool {
        agent == Agent::Codex && !matches!(role, Role::Implement)
    }

    fn run(&mut self, agent: Agent, task: &Task, board: &Board) -> Result<Vec<Output>, AgentError> {
        self.run_with_policy(agent, task, board)
            .map_err(ClassifiedError::into_error)
    }

    fn run_classified(
        &mut self,
        agent: Agent,
        task: &Task,
        board: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        self.run_with_policy(agent, task, board)
    }
}

impl<C: CommandRunner> CodexAgent<C> {
    fn run_with_policy(
        &mut self,
        agent: Agent,
        task: &Task,
        board: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        if !self.supports(agent, task.spec().role) {
            return Err(ClassifiedError::Permanent(AgentError(format!(
                "codex adapter cannot serve {:?} through {agent:?}",
                task.spec().role
            ))));
        }
        let scratch = Scratch::create().map_err(ClassifiedError::Transient)?;
        let prompt = render(task, board);
        let exit = self
            .runner
            .run_scrubbed(
                &self.argv(&scratch.schema, &scratch.reply),
                prompt.as_bytes(),
                self.timeout,
                SCRUBBED_ENV,
            )
            .map_err(exec_error)
            .map_err(ClassifiedError::Transient)?;
        if exit.status != 0 {
            return Err(classify(exit.status, &exit.stderr));
        }
        let reply = fs::read_to_string(&scratch.reply).map_err(|e| {
            ClassifiedError::Transient(AgentError(format!(
                "codex wrote no last message ({e}): {}",
                excerpt(&exit.stderr)
            )))
        })?;
        if reply.trim().is_empty() {
            return Err(ClassifiedError::Transient(AgentError(
                "codex wrote an empty last message".to_owned(),
            )));
        }
        parse(&reply, task.spec().role).map_err(ClassifiedError::Transient)
    }
}

/// Codex reports auth and quota failures on stderr, not by exit code
/// (both exit 1). Verified against codex-cli 0.160.0: a missing login ends
/// with `401 Unauthorized`; quota exhaustion carries `429`,
/// `rate_limit_reached` or `usage_limit_reached`.
fn classify(status: i32, stderr: &[u8]) -> ClassifiedError {
    let text = String::from_utf8_lossy(stderr).to_lowercase();
    let short = excerpt(stderr);
    if text.contains("401 unauthorized")
        || text.contains("not logged in")
        || text.contains("codex login")
    {
        return ClassifiedError::Unavailable(AgentError(format!(
            "codex: not logged in (run `codex login`): {short}"
        )));
    }
    if text.contains("429")
        || text.contains("rate limit")
        || text.contains("rate_limit")
        || text.contains("usage_limit")
        || text.contains("too many requests")
    {
        return ClassifiedError::Transient(AgentError(format!("codex: rate limited: {short}")));
    }
    ClassifiedError::Transient(AgentError(format!(
        "codex exited with status {status}: {short}"
    )))
}

fn exec_error(e: ExecError) -> AgentError {
    AgentError(format!("codex: {e}"))
}

/// The schema file Codex reads and the last-message file it writes, both
/// in the OS temp dir and removed on drop. One pair per run.
struct Scratch {
    schema: PathBuf,
    reply: PathBuf,
}

impl Scratch {
    fn create() -> Result<Self, AgentError> {
        let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
        let stem = format!("orch-codex-{}-{seq}", std::process::id());
        let dir = std::env::temp_dir();
        let schema = dir.join(format!("{stem}.schema.json"));
        let reply = dir.join(format!("{stem}.reply.json"));
        fs::write(&schema, OUTPUT_SCHEMA)
            .map_err(|e| AgentError(format!("codex: cannot write schema file: {e}")))?;
        Ok(Self { schema, reply })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.schema);
        let _ = fs::remove_file(&self.reply);
    }
}
