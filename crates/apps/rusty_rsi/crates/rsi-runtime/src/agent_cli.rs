//! Coding-agent CLIs (`codex exec`, Claude Code's `claude -p`) run inside
//! this crate's sandbox (ADR-0005 §8): [`CliProposer`] rewrites the harness
//! with either, and [`crate::codex_model::CodexModel`] uses Codex as the
//! inner agent's model.
//!
//! The sandbox, not the agent's own, is the boundary:
//!
//! - **Files.** The agent works in a staging copy with no `.git`, so it
//!   cannot redirect the commit. It may write only that copy, its home
//!   (`CODEX_HOME` or `CLAUDE_CONFIG_DIR`), a private `TMPDIR` and
//!   `/dev/null`. It may read the system directories, `/etc` (DNS and
//!   certificates), `/dev/urandom`, its install directory and the
//!   directory of `SSL_CERT_FILE` if set; no other device. Private task
//!   data, the run directory and every other protected path are out of
//!   reach, and nothing starts if the spec could reach one.
//! - **Network.** Open ([`Sockets::Internet`]): the agent must reach its
//!   model. Process-group, `io_uring` and x32 locks still apply.
//! - **Codex.** Its Linux sandbox (bubblewrap) cannot start inside a
//!   Landlock domain, so it runs with
//!   `--dangerously-bypass-approvals-and-sandbox`.
//! - **Claude Code.** It runs `--restricted` with the file tools only
//!   (`Read,Edit,Write,Glob,Grep`: nothing that runs commands), edits
//!   accepted, every other permission prompt denied, and no settings, MCP
//!   servers, slash commands or session files.
//! - **Back into the worktree.** Every change in the copy is replayed
//!   through [`write_inside`], [`link_inside`] and [`remove_inside`], which
//!   never leave the worktree and never touch `.git`. The outer loop's
//!   allowlist then decides, as for any proposer.
//!
//! Each agent signs in with its own login under its home (`codex login`,
//! or `/login` in `claude`); no key passes through `rsi`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rsi_core::{
    CostUsage, ExecOutcome, Limits, ModelId, Precedent, Proposal, Proposer, SandboxSpec,
};
use rusty_json::Value;

use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;
use crate::grading::SYSTEM_READ_ROOTS;
#[cfg(unix)]
use crate::proposer::link_inside;
use crate::proposer::{describe, remove_inside, write_inside};
use crate::sandbox::Sockets;

/// Environment variables passed through to the agent when set: proxy and
/// certificate settings it needs to reach its model.
pub const PASSED_ENV: [&str; 10] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// Bytes of the agent's output quoted in an error.
const EXCERPT_BYTES: usize = 2000;

/// Address-space limit for the agent and the tools it runs.
const MEMORY_BYTES: u64 = 8 << 30;

/// The most model turns Claude Code may take for one proposal.
const CLAUDE_MAX_TURNS: u32 = 60;

/// Claude Code's tools: reading and editing files, nothing that runs code.
const CLAUDE_TOOLS: &str = "Read,Edit,Write,Glob,Grep";

static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

const CONTRACT: &str = "You improve the inner agent of a self-improvement \
harness: the Rust crate in crates/apps/rusty_rsi/harness (standard library \
only, compiled as `rustc --edition 2021 -O --crate-type bin src/lib.rs`, \
entry point `pub fn main` in src/lib.rs). It runs in a sandbox with \
`task.md` in its working directory and a broker socket on standard input \
(see the protocol in src/broker.rs). Through the broker it may call a \
language model, run candidate solutions on public data to get a score, and \
submit its best solution; the latest submission counts. It is graded on \
held-out data it never sees, under a fixed token and time budget. Make it \
find better solutions within that budget.\n\n\
Edit the files in your working directory directly. Only files under \
crates/apps/rusty_rsi/harness/src/ may change; any other change rejects \
the candidate. There is no git repository here. End with a one-line \
summary of what you changed.";

/// Which coding-agent CLI to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAgent {
    /// OpenAI's Codex CLI (`codex exec`).
    Codex,
    /// Anthropic's Claude Code (`claude -p`).
    Claude,
}

impl CliAgent {
    /// The agent's name, as used in model ids and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    /// The variable naming the agent's home (login and state).
    #[must_use]
    pub const fn home_var(self) -> &'static str {
        match self {
            Self::Codex => "CODEX_HOME",
            Self::Claude => "CLAUDE_CONFIG_DIR",
        }
    }

    const fn login_hint(self) -> &'static str {
        match self {
            Self::Codex => "run `codex login` with this CODEX_HOME",
            Self::Claude => "run `claude` and `/login` with this CLAUDE_CONFIG_DIR",
        }
    }
}

/// How to run a coding-agent CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliConfig {
    /// Which agent the program is.
    pub agent: CliAgent,
    /// The agent's binary, absolute.
    pub program: PathBuf,
    /// The model (`-m` / `--model`); the agent's default when `None`.
    pub model: Option<String>,
    /// The agent's home: its login and state, absolute.
    pub home: PathBuf,
    /// How long one proposal may take.
    pub wall: Duration,
    /// Extra environment, such as the [`PASSED_ENV`] that are set.
    pub env: Vec<(String, String)>,
}

impl CliConfig {
    /// The directory the agent is installed in: the binary's directory, or
    /// its parent when that is a `bin` directory (an npm package or a
    /// prefix keeps the agent's helpers beside `bin`).
    #[must_use]
    pub fn install_root(&self) -> PathBuf {
        let dir = self.program.parent().unwrap_or(Path::new("/"));
        match dir.file_name() {
            Some(name) if name == "bin" => dir.parent().unwrap_or(dir).to_path_buf(),
            _ => dir.to_path_buf(),
        }
    }

    /// The model id recorded in lineage: the agent, then the model if set.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] if the model name is not a valid id.
    pub fn model_id(&self) -> Result<ModelId, RuntimeError> {
        let name = self.model.as_ref().map_or_else(
            || self.agent.name().to_owned(),
            |m| format!("{}:{m}", self.agent.name()),
        );
        ModelId::parse(&name).map_err(|e| RuntimeError::Model(e.to_string()))
    }

    /// The arguments for a proposal in the copy at `tree`; Codex writes
    /// its reply to `reply`, Claude Code to standard output.
    #[must_use]
    pub fn proposer_args(&self, tree: &Path, reply: &Path) -> Vec<String> {
        match self.agent {
            CliAgent::Codex => self.codex_args(tree, reply),
            CliAgent::Claude => {
                let mut args: Vec<String> = [
                    "-p",
                    "--output-format",
                    "json",
                    "--tools",
                    CLAUDE_TOOLS,
                    "--permission-mode",
                    "acceptEdits",
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
                args.push(CLAUDE_MAX_TURNS.to_string());
                if let Some(model) = &self.model {
                    args.extend(["--model".to_owned(), model.clone()]);
                }
                args
            }
        }
    }

    /// `codex exec` in `tree`, the reply to `reply`, the prompt on
    /// standard input, and events (with token usage) on standard output.
    pub(crate) fn codex_args(&self, tree: &Path, reply: &Path) -> Vec<String> {
        let mut args: Vec<String> = [
            "exec",
            "--dangerously-bypass-approvals-and-sandbox",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--color",
            "never",
            "--json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.extend(["-C".to_owned(), tree.display().to_string()]);
        if let Some(model) = &self.model {
            args.extend(["-m".to_owned(), model.clone()]);
        }
        args.extend(["-o".to_owned(), reply.display().to_string(), "-".to_owned()]);
        args
    }

    /// Checks that every path the agent needs is absolute.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] naming the first relative one.
    pub(crate) fn check_paths(&self, scratch: &Path) -> Result<(), RuntimeError> {
        for (what, path) in [
            ("program", self.program.as_path()),
            ("home", self.home.as_path()),
            ("scratch", scratch),
        ] {
            if !path.is_absolute() {
                return Err(RuntimeError::Model(format!(
                    "{} {what} {} is not absolute",
                    self.agent.name(),
                    path.display()
                )));
            }
        }
        Ok(())
    }

    /// The sandbox for one run in `staging`, at most `wall` long; refused
    /// if it could reach any `protected` path.
    ///
    /// # Errors
    /// [`RuntimeError::Sandbox`] for a reachable protected path;
    /// [`RuntimeError::Model`] for an invalid spec.
    pub(crate) fn sandbox(
        &self,
        staging: &Staging,
        wall: Duration,
        protected: &[PathBuf],
    ) -> Result<SandboxSpec, RuntimeError> {
        let mut read: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
        read.push("/etc".into());
        read.push(self.install_root());
        let cert_dir = self
            .env
            .iter()
            .find(|(name, _)| name == "SSL_CERT_FILE")
            .and_then(|(_, file)| Path::new(file).parent().map(Path::to_path_buf));
        read.extend(cert_dir);
        read.push("/dev/urandom".into());
        read.retain(|root| root.exists());
        let tmp = staging.tmp();
        let mut env = vec![
            ("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned()),
            ("HOME".to_owned(), tmp.display().to_string()),
            ("TMPDIR".to_owned(), tmp.display().to_string()),
            (
                self.agent.home_var().to_owned(),
                self.home.display().to_string(),
            ),
        ];
        env.extend(self.env.iter().cloned());
        let limits = Limits::new(
            Duration::from_secs(wall.as_secs().max(1)),
            wall,
            MEMORY_BYTES,
            1 << 30,
            1024,
            512,
        )
        .map_err(|e| RuntimeError::Model(e.to_string()))?;
        let spec = SandboxSpec::new(
            read,
            vec![
                staging.tree(),
                tmp,
                self.home.clone(),
                PathBuf::from("/dev/null"),
            ],
            staging.tree(),
            env,
            limits,
        )
        .map_err(|e| RuntimeError::Model(e.to_string()))?;
        if let Some(path) = protected.iter().find(|p| spec.can_reach(p)) {
            return Err(RuntimeError::Sandbox(format!(
                "the {} sandbox could reach {}",
                self.agent.name(),
                path.display()
            )));
        }
        Ok(spec)
    }

    /// Runs the agent in `staging` with `args`, the prompt file on standard
    /// input, at most `wall` long.
    ///
    /// # Errors
    /// As [`ProcessExecutor::exec_with`], or a refused sandbox.
    pub(crate) fn run(
        &self,
        executor: &ProcessExecutor,
        staging: &Staging,
        args: &[String],
        prompt: &Path,
        wall: Duration,
        protected: &[PathBuf],
    ) -> Result<ExecOutcome, RuntimeError> {
        let spec = self.sandbox(staging, wall, protected)?;
        let stdin = File::open(prompt).map_err(|e| {
            RuntimeError::io(format!("opening the {} prompt", self.agent.name()), e)
        })?;
        Ok(executor.exec_with(
            &spec,
            &self.program.display().to_string(),
            args,
            Stdio::from(stdin),
            Sockets::Internet,
        )?)
    }

    /// A failed run as an error, with a hint for the common cases.
    pub(crate) fn failure(&self, outcome: &ExecOutcome) -> RuntimeError {
        let mut text = String::from_utf8_lossy(&outcome.stderr).to_lowercase();
        text.push_str(&String::from_utf8_lossy(&outcome.stdout).to_lowercase());
        let hint = if [
            "401 unauthorized",
            "not logged in",
            "/login",
            "invalid api key",
        ]
        .iter()
        .any(|needle| text.contains(needle))
        {
            format!(" (not logged in: {})", self.agent.login_hint())
        } else if ["429", "usage_limit", "rate limit"]
            .iter()
            .any(|needle| text.contains(needle))
        {
            " (rate limited)".to_owned()
        } else {
            String::new()
        };
        RuntimeError::Model(format!(
            "{} ended with {:?}{hint}: {}",
            self.agent.name(),
            outcome.termination,
            excerpt(&outcome.stderr, &outcome.stdout)
        ))
    }
}

/// Bytes of an agent's output kept: enough for a long Codex event stream,
/// whose `turn.completed` event comes last.
pub(crate) const EVENT_BYTES: usize = 8 << 20;

/// Prompt and completion tokens from the `turn.completed` events in
/// Codex's JSONL output, summed.
///
/// # Errors
/// [`RuntimeError::Model`] for a `turn.failed` event, or when no turn
/// reported its usage.
pub(crate) fn codex_usage(stdout: &[u8]) -> Result<(u64, u64), RuntimeError> {
    let text = String::from_utf8_lossy(stdout);
    let mut total: Option<(u64, u64)> = None;
    for line in text.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        let Ok(event) = Value::parse(line) else {
            continue;
        };
        match event.pointer("/type").and_then(Value::as_str) {
            Some("turn.failed") => {
                let message = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no message");
                return Err(RuntimeError::Model(format!("codex turn failed: {message}")));
            }
            Some("turn.completed") => {
                let count = |key: &str| {
                    event
                        .pointer(&format!("/usage/{key}"))
                        .and_then(Value::as_u64)
                };
                let (Some(input), Some(output)) = (count("input_tokens"), count("output_tokens"))
                else {
                    return Err(RuntimeError::Model(
                        "codex's turn.completed event has no usage".into(),
                    ));
                };
                let (p, c) = total.unwrap_or((0, 0));
                total = Some((p.saturating_add(input), c.saturating_add(output)));
            }
            _ => {}
        }
    }
    total.ok_or_else(|| {
        RuntimeError::Model("codex reported no token usage, so the run cannot be metered".into())
    })
}

/// Rewrites the harness with a coding-agent CLI, confined by the sandbox.
#[derive(Debug)]
pub struct CliProposer {
    executor: ProcessExecutor,
    config: CliConfig,
    scratch: PathBuf,
    protected: Vec<PathBuf>,
    id: ModelId,
}

impl CliProposer {
    /// A proposer that stages each proposal under `scratch` and refuses to
    /// run the agent if its sandbox could reach any `protected` path.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] if the program, home or scratch path is
    /// relative, or the model name is not a valid id.
    pub fn new(
        executor: &ProcessExecutor,
        config: CliConfig,
        scratch: PathBuf,
        protected: Vec<PathBuf>,
    ) -> Result<Self, RuntimeError> {
        config.check_paths(&scratch)?;
        let id = config.model_id()?;
        Ok(Self {
            executor: executor.clone().with_capture_bytes(EVENT_BYTES),
            config,
            scratch,
            protected,
            id,
        })
    }

    /// The reply text and the prompt and completion tokens: Codex's
    /// last-message file and event stream, or Claude Code's JSON envelope.
    fn reply(
        &self,
        outcome: &ExecOutcome,
        reply: &Path,
    ) -> Result<(String, (u64, u64)), RuntimeError> {
        match self.config.agent {
            CliAgent::Codex => {
                let text = std::fs::read_to_string(reply).map_err(|e| {
                    RuntimeError::Model(format!(
                        "codex wrote no reply ({e}): {}",
                        excerpt(&outcome.stderr, &outcome.stdout)
                    ))
                })?;
                Ok((text, codex_usage(&outcome.stdout)?))
            }
            CliAgent::Claude => claude_result(&outcome.stdout),
        }
    }
}

impl Proposer for CliProposer {
    type Error = RuntimeError;

    fn model(&self) -> Option<&ModelId> {
        Some(&self.id)
    }

    fn propose(&self, history: &[Precedent], workspace: &Path) -> Result<Proposal, RuntimeError> {
        let staging = Staging::create(&self.scratch, self.config.agent)?;
        copy_tree(workspace, &staging.tree())?;
        let prompt = staging.write_prompt(&format!("{CONTRACT}\n\n{}", describe(history)))?;
        let reply = staging.tmp().join("reply.md");
        let args = self.config.proposer_args(&staging.tree(), &reply);
        let outcome = self.config.run(
            &self.executor,
            &staging,
            &args,
            &prompt,
            self.config.wall,
            &self.protected,
        )?;
        if !outcome.termination.succeeded() {
            return Err(self.config.failure(&outcome));
        }
        let (text, (prompt_tokens, completion_tokens)) = self.reply(&outcome, &reply)?;
        mirror(&staging.tree(), workspace)?;
        Ok(Proposal {
            summary: summary_line(&text),
            usage: CostUsage {
                prompt_tokens,
                completion_tokens,
                ..CostUsage::default()
            },
        })
    }
}

/// The `result` of Claude Code's `--output-format json` envelope and its
/// prompt and completion tokens. Prompt tokens include cache writes and
/// reads, which Claude Code counts apart from `input_tokens`.
///
/// # Errors
/// [`RuntimeError::Model`] for an envelope marked `is_error` (carrying its
/// result), or one without its token usage.
fn claude_result(stdout: &[u8]) -> Result<(String, (u64, u64)), RuntimeError> {
    let text = String::from_utf8_lossy(stdout);
    let envelope = Value::parse(text.trim()).map_err(|e| {
        RuntimeError::Model(format!(
            "claude printed no JSON result ({e}): {}",
            excerpt(&[], stdout)
        ))
    })?;
    let result = envelope
        .pointer("/result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if envelope.pointer("/is_error").and_then(Value::as_bool) == Some(true) {
        return Err(RuntimeError::Model(format!(
            "claude reported an error: {result}"
        )));
    }
    let count = |key: &str| {
        envelope
            .pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
    };
    let (Some(input), Some(output)) = (count("input_tokens"), count("output_tokens")) else {
        return Err(RuntimeError::Model(
            "claude reported no token usage, so the run cannot be metered".into(),
        ));
    };
    let prompt = ["cache_creation_input_tokens", "cache_read_input_tokens"]
        .into_iter()
        .filter_map(count)
        .fold(input, u64::saturating_add);
    Ok((result, (prompt, output)))
}

/// The end of `stderr`, or of `stdout` when stderr is empty.
fn excerpt(stderr: &[u8], stdout: &[u8]) -> String {
    let bytes = if stderr.iter().all(u8::is_ascii_whitespace) {
        stdout
    } else {
        stderr
    };
    let start = bytes.len().saturating_sub(EXCERPT_BYTES);
    String::from_utf8_lossy(&bytes[start..]).trim().to_owned()
}

/// The reply's first non-empty line, shortened for a commit message.
fn summary_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("agent proposal");
    line.chars().take(72).collect()
}

/// One run's private directory: `tree/` (the copy the agent works in),
/// `tmp/` and the prompt beside them. Removed on drop.
pub(crate) struct Staging {
    root: PathBuf,
}

impl Staging {
    /// A fresh, empty staging directory under `scratch`.
    pub(crate) fn create(scratch: &Path, agent: CliAgent) -> Result<Self, RuntimeError> {
        let n = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
        let root = scratch.join(format!("{}-{}-{n}", agent.name(), std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| RuntimeError::io("clearing an old staging dir", e))?;
        }
        for dir in [root.join("tree"), root.join("tmp")] {
            std::fs::create_dir_all(&dir)
                .map_err(|e| RuntimeError::io(format!("creating {}", dir.display()), e))?;
        }
        Ok(Self { root })
    }

    /// Writes the prompt beside the copy, out of the agent's write reach.
    pub(crate) fn write_prompt(&self, text: &str) -> Result<PathBuf, RuntimeError> {
        let prompt = self.root.join("prompt.md");
        std::fs::write(&prompt, text).map_err(|e| RuntimeError::io("writing the prompt", e))?;
        Ok(prompt)
    }

    pub(crate) fn tree(&self) -> PathBuf {
        self.root.join("tree")
    }

    pub(crate) fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // Best effort: a leftover directory is harmless and is cleared by
        // the next run with the same name.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// What a tree holds at one relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    File { bytes: Vec<u8>, executable: bool },
    Link(PathBuf),
}

/// Every file and symlink under `root`, by relative path, skipping `.git`
/// (in any letter case) at every level and never following a link.
fn entries(root: &Path) -> Result<Vec<(String, Entry)>, RuntimeError> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Entry)>) -> Result<(), RuntimeError> {
        let list = std::fs::read_dir(dir)
            .map_err(|e| RuntimeError::io(format!("listing {}", dir.display()), e))?;
        for item in list {
            let item = item.map_err(|e| RuntimeError::io("listing a tree", e))?;
            let name = item.file_name();
            if name.eq_ignore_ascii_case(".git") {
                continue;
            }
            let name = name.to_str().ok_or_else(|| {
                RuntimeError::Model(format!("{} has a non-UTF-8 name", item.path().display()))
            })?;
            let path = format!("{prefix}{name}");
            let kind = item
                .file_type()
                .map_err(|e| RuntimeError::io(format!("inspecting {path}"), e))?;
            if kind.is_dir() {
                walk(&item.path(), &format!("{path}/"), out)?;
            } else if kind.is_symlink() {
                let target = std::fs::read_link(item.path())
                    .map_err(|e| RuntimeError::io(format!("reading link {path}"), e))?;
                out.push((path, Entry::Link(target)));
            } else if kind.is_file() {
                let bytes = std::fs::read(item.path())
                    .map_err(|e| RuntimeError::io(format!("reading {path}"), e))?;
                let executable = is_executable(&item.path())?;
                out.push((path, Entry::File { bytes, executable }));
            } else {
                return Err(RuntimeError::Model(format!(
                    "{path} is neither a file, a directory nor a symlink"
                )));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> Result<bool, RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| RuntimeError::io(format!("inspecting {}", path.display()), e))?;
    Ok(meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> Result<bool, RuntimeError> {
    Ok(false)
}

/// Writes `entry` at `path` inside `root`.
fn put(root: &Path, path: &str, entry: &Entry) -> Result<(), RuntimeError> {
    match entry {
        Entry::File { bytes, executable } => {
            write_inside(root, path, bytes)?;
            set_executable(&root.join(path), *executable)
        }
        Entry::Link(target) => put_link(root, path, target),
    }
}

#[cfg(unix)]
fn put_link(root: &Path, path: &str, target: &Path) -> Result<(), RuntimeError> {
    link_inside(root, path, target)
}

#[cfg(not(unix))]
fn put_link(_root: &Path, path: &str, _target: &Path) -> Result<(), RuntimeError> {
    Err(RuntimeError::Model(format!(
        "{path}: symlinks are only supported on Unix"
    )))
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| RuntimeError::io(format!("setting the mode of {}", path.display()), e))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), RuntimeError> {
    Ok(())
}

/// Copies the worktree at `from` (but not its `.git`) into the empty
/// directory `to`.
fn copy_tree(from: &Path, to: &Path) -> Result<(), RuntimeError> {
    for (path, entry) in entries(from)? {
        put(to, &path, &entry)?;
    }
    Ok(())
}

/// Replays into `workspace` every difference between it and the edited
/// copy `tree`: changed and new entries are written, missing ones removed.
/// `.git` is never read from the copy nor touched in the workspace.
fn mirror(tree: &Path, workspace: &Path) -> Result<(), RuntimeError> {
    let edited = entries(tree)?;
    let current = entries(workspace)?;
    for (path, entry) in &edited {
        let unchanged = current
            .binary_search_by(|(p, _)| p.as_str().cmp(path))
            .is_ok_and(|at| current[at].1 == *entry);
        if !unchanged {
            put(workspace, path, entry)?;
        }
    }
    for (path, _) in &current {
        if edited
            .binary_search_by(|(p, _)| p.as_str().cmp(path))
            .is_err()
        {
            remove_inside(workspace, path)?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use rsi_core::Termination;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rsi-agent-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn the_copy_leaves_git_behind_and_the_mirror_replays_every_change() {
        let root = scratch("mirror");
        let ws = root.join("ws");
        write(&ws.join(".git"), "gitdir: /somewhere\n");
        write(&ws.join("h/src/lib.rs"), "old");
        write(&ws.join("h/src/gone.rs"), "bye");
        write(&ws.join("h/Cargo.toml"), "same");
        let tree = root.join("tree");
        std::fs::create_dir_all(&tree).expect("tree");
        copy_tree(&ws, &tree).expect("copy");
        assert!(!tree.join(".git").exists(), "no .git in the copy");
        assert_eq!(
            std::fs::read_to_string(tree.join("h/Cargo.toml"))
                .ok()
                .as_deref(),
            Some("same")
        );

        write(&tree.join("h/src/lib.rs"), "new");
        std::fs::remove_file(tree.join("h/src/gone.rs")).expect("rm");
        write(&tree.join("h/src/added/mod.rs"), "added");
        write(
            &tree.join(".git/config"),
            "[core]\n\thooksPath = /tmp/evil\n",
        );
        write(&tree.join("h/.GIT/x"), "case");
        std::os::unix::fs::symlink("/etc/passwd", tree.join("h/src/leak.rs")).expect("link");
        set_executable(&tree.join("h/src/lib.rs"), true).expect("chmod");
        mirror(&tree, &ws).expect("mirror");

        let read = |p: &str| std::fs::read_to_string(ws.join(p)).ok();
        assert_eq!(read(".git").as_deref(), Some("gitdir: /somewhere\n"));
        assert_eq!(read("h/src/lib.rs").as_deref(), Some("new"));
        assert!(is_executable(&ws.join("h/src/lib.rs")).expect("mode"));
        assert_eq!(read("h/src/added/mod.rs").as_deref(), Some("added"));
        assert!(!ws.join("h/src/gone.rs").exists());
        assert!(!ws.join("h/.GIT").exists(), ".git in any case is skipped");
        let link = std::fs::symlink_metadata(ws.join("h/src/leak.rs")).expect("link kept");
        assert!(
            link.file_type().is_symlink(),
            "recreated, for the allowlist to reject"
        );
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn the_mirror_cannot_write_through_a_symlinked_directory() {
        let root = scratch("through");
        let ws = root.join("ws");
        let outside = root.join("outside");
        write(&outside.join("sentinel"), "keep");
        std::fs::create_dir_all(&ws).expect("ws");
        std::os::unix::fs::symlink(&outside, ws.join("dir")).expect("link");
        let tree = root.join("tree");
        write(&tree.join("dir/sentinel"), "overwritten");
        assert!(mirror(&tree, &ws).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("sentinel"))
                .ok()
                .as_deref(),
            Some("keep")
        );
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    fn codex() -> CliConfig {
        CliConfig {
            agent: CliAgent::Codex,
            program: "/opt/codex/bin/codex".into(),
            model: Some("gpt-x".into()),
            home: "/home/u/.codex".into(),
            wall: Duration::from_secs(60),
            env: vec![],
        }
    }

    #[test]
    fn codex_args_pin_the_bypass_and_the_isolation_flags() {
        let config = codex();
        let args = config.proposer_args(Path::new("/s/tree"), Path::new("/s/tmp/reply.md"));
        assert_eq!(
            args,
            [
                "exec",
                "--dangerously-bypass-approvals-and-sandbox",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--json",
                "-C",
                "/s/tree",
                "-m",
                "gpt-x",
                "-o",
                "/s/tmp/reply.md",
                "-",
            ]
        );
        assert_eq!(config.install_root(), PathBuf::from("/opt/codex"));
        assert_eq!(config.model_id().expect("id").as_str(), "codex:gpt-x");
        let bare = CliConfig {
            program: "/opt/tools/codex".into(),
            model: None,
            ..config
        };
        assert_eq!(bare.install_root(), PathBuf::from("/opt/tools"));
        assert_eq!(bare.model_id().expect("id").as_str(), "codex");
    }

    #[test]
    fn claude_args_allow_file_tools_only() {
        let config = CliConfig {
            agent: CliAgent::Claude,
            program: "/opt/claude-code/bin/claude".into(),
            model: Some("opus".into()),
            home: "/home/u/.claude".into(),
            ..codex()
        };
        let args = config.proposer_args(Path::new("/s/tree"), Path::new("/unused"));
        let flag = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|at| args.get(at + 1))
                .map(String::as_str)
        };
        assert_eq!(flag("--tools"), Some("Read,Edit,Write,Glob,Grep"));
        assert_eq!(flag("--permission-mode"), Some("acceptEdits"));
        assert_eq!(flag("--permission-prompts"), Some("none"));
        assert_eq!(flag("--setting-sources"), Some(""));
        assert_eq!(flag("--model"), Some("opus"));
        for needed in [
            "-p",
            "--restricted",
            "--strict-mcp-config",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|a| a == needed), "{needed}: {args:?}");
        }
        assert!(
            !args
                .iter()
                .any(|a| a.contains("Bash") || a.contains("dangerously")),
            "{args:?}"
        );
        assert_eq!(config.agent.home_var(), "CLAUDE_CONFIG_DIR");
        assert_eq!(config.model_id().expect("id").as_str(), "claude:opus");
    }

    #[test]
    fn usage_comes_from_turn_completed_and_is_required() {
        let events = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n\
{\"type\":\"turn.started\"}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi\"}}\n\
{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":120,\"cached_input_tokens\":100,\"output_tokens\":7}}\n";
        assert_eq!(codex_usage(events).expect("usage"), (120, 7));
        let e = codex_usage(b"{\"type\":\"turn.started\"}\n").expect_err("no usage");
        assert!(e.to_string().contains("cannot be metered"), "{e}");
        let e = codex_usage(b"{\"type\":\"turn.failed\",\"error\":{\"message\":\"quota\"}}\n")
            .expect_err("failed");
        assert!(e.to_string().contains("quota"), "{e}");
        let e = codex_usage(b"{\"type\":\"turn.completed\",\"usage\":{}}\n").expect_err("empty");
        assert!(e.to_string().contains("no usage"), "{e}");
    }

    #[test]
    fn the_claude_envelope_gives_the_result_or_its_error() {
        let ok = br#"{"type":"result","is_error":false,"result":"Tuned the search.\nmore",
"usage":{"input_tokens":12,"cache_creation_input_tokens":300,"cache_read_input_tokens":4000,"output_tokens":90}}"#;
        assert_eq!(
            claude_result(ok).expect("result"),
            ("Tuned the search.\nmore".to_owned(), (4_312, 90))
        );
        let unmetered = br#"{"type":"result","is_error":false,"result":"done"}"#;
        let e = claude_result(unmetered).expect_err("no usage");
        assert!(e.to_string().contains("no token usage"), "{e}");
        let failed = br#"{"type":"result","is_error":true,"result":"Prompt is too long"}"#;
        let e = claude_result(failed).expect_err("an error");
        assert!(e.to_string().contains("Prompt is too long"), "{e}");
        assert!(claude_result(b"not json").is_err());
    }

    #[test]
    fn failures_carry_a_hint_and_the_end_of_the_output() {
        let outcome = |termination, stderr: &[u8], stdout: &[u8]| ExecOutcome {
            termination,
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            wall: Duration::ZERO,
        };
        let e = codex().failure(&outcome(
            Termination::Exited(1),
            b"...\nERROR: 401 Unauthorized",
            b"",
        ));
        assert!(e.to_string().contains("codex login"), "{e}");
        let claude = CliConfig {
            agent: CliAgent::Claude,
            ..codex()
        };
        let e = claude.failure(&outcome(
            Termination::Exited(1),
            b"",
            b"Invalid API key \xc2\xb7 Please run /login",
        ));
        assert!(e.to_string().contains("CLAUDE_CONFIG_DIR"), "{e}");
        let e = codex().failure(&outcome(Termination::TimedOut, b"slow", b""));
        assert!(
            e.to_string().contains("TimedOut") && e.to_string().contains("slow"),
            "{e}"
        );
        assert_eq!(
            summary_line("\n  Tuned the search.\nmore"),
            "Tuned the search."
        );
        assert_eq!(summary_line(""), "agent proposal");
    }
}
