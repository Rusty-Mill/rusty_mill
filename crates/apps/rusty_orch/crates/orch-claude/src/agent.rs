//! The I/O shell: probe the login, render, run `claude -p`, read the
//! envelope, parse.

use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_core::board::Board;
use orch_core::goal::StopRule;
use orch_core::task::{Agent, Role, Task};
use orch_dispatch::{AgentError, AgentRunner, ClassifiedError, Output};
use rusty_json::Value;

use orch_cli::{excerpt, output_schema, parse, CommandRunner, ExecError, Exit, StdCommand};

use crate::render;

/// Claude reads the repo and reasons before answering; ten minutes by default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// `claude auth status` reads local state only; a minute is generous.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(60);

/// Agentic turns per run. Research with three read-only tools needs several;
/// the cap is what stops a run that never converges.
pub const DEFAULT_MAX_TURNS: u32 = 20;

/// The built-in tools a run may use. No edit, write, shell, or web tool
/// exists in the session, so nothing can be modified or fetched.
pub const TOOLS: &str = "Read,Grep,Glob";

/// Removed from the child's environment on every run. A stray key or
/// provider override would route Claude Code onto the paid API path or a
/// third-party provider instead of the subscription login.
pub const SCRUBBED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

/// [`Agent::Claude`] over `claude -p`, read-only tools, prompt on stdin.
///
/// Serves non-implementation roles through `Agent::Claude`; unsupported
/// agent/role pairs are rejected before any process is spawned. The argv is
/// a fixed vector with no shell and no interpolation; the model is Claude
/// Code's default unless [`ClaudeAgent::model`] sets one. The child runs
/// with the repository root as its working directory, which `--restricted`
/// confines the file tools to.
///
/// Each run is two processes: `claude auth status`, whose non-zero exit is
/// the unavailable "not logged in" prerequisite, then the run itself.
#[derive(Debug, Clone)]
pub struct ClaudeAgent<C = StdCommand> {
    repo_root: PathBuf,
    timeout: Duration,
    model: Option<String>,
    max_turns: u32,
    stop: StopRule,
    runner: C,
}

impl ClaudeAgent<StdCommand> {
    /// An agent over the real `claude` binary on `PATH`, working in `repo_root`.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_runner(repo_root, StdCommand)
    }
}

impl<C: CommandRunner> ClaudeAgent<C> {
    /// An agent over any [`CommandRunner`], e.g. a fake in tests.
    pub fn with_runner(repo_root: impl Into<PathBuf>, runner: C) -> Self {
        Self {
            repo_root: repo_root.into(),
            timeout: DEFAULT_TIMEOUT,
            model: None,
            max_turns: DEFAULT_MAX_TURNS,
            stop: StopRule::Checkpoint,
            runner,
        }
    }

    /// The goal's stop rule, which decides whether cards may ask questions
    /// and shapes the output schema (ADR-0011). Defaults to
    /// [`StopRule::Checkpoint`].
    pub fn with_stop_rule(mut self, stop: StopRule) -> Self {
        self.stop = stop;
        self
    }

    /// Replace the default ten-minute deadline.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Pin a model instead of Claude Code's default.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Replace the default turn cap.
    pub fn max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// The runner, e.g. to inspect a fake in tests.
    pub fn runner(&self) -> &C {
        &self.runner
    }

    /// The repository root the child runs in.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// The argv this agent runs. Public so tests and docs can pin it. The
    /// schema travels in the argv itself; `--json-schema` takes text.
    pub fn argv(&self) -> Vec<String> {
        let mut argv: Vec<String> = [
            "claude",
            "-p",
            "--output-format",
            "json",
            "--tools",
            TOOLS,
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--restricted",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--max-turns",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        argv.push(self.max_turns.to_string());
        if let Some(model) = &self.model {
            argv.push("--model".to_owned());
            argv.push(model.clone());
        }
        argv.push("--json-schema".to_owned());
        argv.push(output_schema(self.stop));
        argv
    }

    /// The login probe run before every card: exit 0 when logged in.
    pub fn auth_argv() -> Vec<String> {
        ["claude", "auth", "status"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

impl<C: CommandRunner> AgentRunner for ClaudeAgent<C> {
    fn supports(&self, agent: Agent, role: Role) -> bool {
        agent == Agent::Claude && !matches!(role, Role::Implement)
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

impl<C: CommandRunner> ClaudeAgent<C> {
    fn run_with_policy(
        &mut self,
        agent: Agent,
        task: &Task,
        board: &Board,
    ) -> Result<Vec<Output>, ClassifiedError> {
        if !self.supports(agent, task.spec().role) {
            return Err(ClassifiedError::Permanent(AgentError(format!(
                "claude adapter cannot serve {:?} through {agent:?}",
                task.spec().role
            ))));
        }
        self.probe_login()?;
        let prompt = render(task, board, self.stop);
        let exit = self
            .runner
            .run_in(
                Some(&self.repo_root),
                &self.argv(),
                prompt.as_bytes(),
                self.timeout,
                SCRUBBED_ENV,
            )
            .map_err(exec_error)
            .map_err(ClassifiedError::Transient)?;
        let reply = extract(&exit)?;
        parse(&reply, task.spec().role, self.stop).map_err(ClassifiedError::Transient)
    }

    /// `claude auth status` exits non-zero when nobody is logged in. The
    /// probe runs with the same environment as the run, so a scrubbed key
    /// cannot make it report a login the run will not have.
    fn probe_login(&self) -> Result<(), ClassifiedError> {
        let exit = self
            .runner
            .run_in(
                Some(&self.repo_root),
                &Self::auth_argv(),
                b"",
                AUTH_TIMEOUT,
                SCRUBBED_ENV,
            )
            .map_err(exec_error)
            .map_err(ClassifiedError::Transient)?;
        if exit.status == 0 {
            return Ok(());
        }
        Err(not_logged_in(&excerpt(&exit.stdout)))
    }
}

/// The reply text from the result envelope, or the classified failure.
///
/// `claude -p --output-format json` prints one JSON object with `is_error`,
/// `subtype`, `result` (the final text) and, under `--json-schema`,
/// `structured_output` (the validated object). Auth and quota failures are
/// reported inside that envelope with `is_error: true`, usually with exit 0,
/// so the envelope is read before the exit status. Verified against Claude
/// Code 2.1.289.
fn extract(exit: &Exit) -> Result<String, ClassifiedError> {
    let stdout = String::from_utf8_lossy(&exit.stdout);
    let Ok(envelope) = Value::parse(stdout.trim()) else {
        if exit.status != 0 {
            return Err(classify(
                exit.status,
                "exit",
                &excerpt(if exit.stderr.is_empty() {
                    &exit.stdout
                } else {
                    &exit.stderr
                }),
            ));
        }
        return Err(ClassifiedError::Transient(AgentError(format!(
            "claude wrote no result envelope: {}",
            excerpt(&exit.stdout)
        ))));
    };
    let subtype = envelope
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let result = envelope.get("result").and_then(Value::as_str).unwrap_or("");
    let is_error = envelope.get("is_error").and_then(Value::as_bool) == Some(true);
    if exit.status != 0 || is_error {
        let detail = if result.is_empty() {
            excerpt(&exit.stderr)
        } else {
            excerpt(result.as_bytes())
        };
        return Err(classify(exit.status, subtype, &detail));
    }
    match envelope.get("structured_output") {
        Some(reply) if !reply.is_null() => Ok(reply.to_json_string()),
        _ => Err(ClassifiedError::Transient(AgentError(format!(
            "claude returned no structured output ({subtype}): {}",
            excerpt(result.as_bytes())
        )))),
    }
}

/// Auth failures name a login; quota failures name a limit. Anything else
/// is a counted transient failure carrying the status and subtype.
fn classify(status: i32, subtype: &str, detail: &str) -> ClassifiedError {
    let text = detail.to_lowercase();
    if text.contains("not logged in")
        || text.contains("/login")
        || text.contains("authentication")
        || text.contains("401")
        || text.contains("invalid api key")
    {
        return not_logged_in(detail);
    }
    if text.contains("429")
        || text.contains("rate limit")
        || text.contains("rate_limit")
        || text.contains("usage limit")
        || text.contains("too many requests")
        || text.contains("overloaded")
    {
        return ClassifiedError::Transient(AgentError(format!("claude: rate limited: {detail}")));
    }
    ClassifiedError::Transient(AgentError(format!(
        "claude exited with status {status} ({subtype}): {detail}"
    )))
}

fn not_logged_in(detail: &str) -> ClassifiedError {
    ClassifiedError::Unavailable(AgentError(format!(
        "claude: not logged in (run `claude auth login`): {detail}"
    )))
}

fn exec_error(e: ExecError) -> AgentError {
    AgentError(format!("claude: {e}"))
}
