//! The coding-agent CLIs through the real sandbox, with fake `codex` and
//! `claude` scripts: what they may reach, what comes back into the
//! worktree, how a Codex completion is metered, and how each fails.
#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{executor, Scratch};
use rsi_core::{ChatModel, Message, Proposer, Role};
use rsi_runtime::agent_cli::{CliAgent, CliConfig, CliProposer, PASSED_ENV};
use rsi_runtime::codex_model::CodexModel;
use rsi_runtime::RuntimeError;

const LIB: &str = "crates/apps/rusty_rsi/harness/src/lib.rs";

/// Writes an executable `bin/<name>` under `dir` running `body` in `sh`.
fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join("bin").join(name);
    std::fs::create_dir_all(program.parent().expect("bin")).expect("bin dir");
    std::fs::write(&program, format!("#!/bin/sh\n{body}")).expect("script");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    program
}

/// A fake that records what it saw in `$CODEX_HOME`, probes the sandbox,
/// edits the copy and replies.
const EDITOR: &str = r#"
tree=""; reply=""; prev=""
for a in "$@"; do
  case "$prev" in -C) tree="$a";; -o) reply="$a";; esac
  prev="$a"
done
printf '%s\n' "$@" > "$CODEX_HOME/args"
cat > "$CODEX_HOME/prompt"
printf 'HOME=%s\nTMPDIR=%s\n' "$HOME" "$TMPDIR" > "$CODEX_HOME/env"
probe() { name="$1"; shift; if "$@" 2>/dev/null; then echo "$name ok"; else echo "$name denied"; fi; }
{
  probe outside sh -c "echo x > $OUTSIDE/escape"
  probe private sh -c "cat $PRIVATE"
  probe tmp sh -c "echo x > $TMPDIR/scratch"
  probe null sh -c "echo x > /dev/null && head -c 8 /dev/urandom > /dev/null"
  probe device sh -c "head -c 1 /dev/zero"
  probe inet python3 -c "import socket; socket.socket().bind(('127.0.0.1', 0))"
} > "$CODEX_HOME/probes"
lib="$tree/crates/apps/rusty_rsi/harness/src/lib.rs"
echo '// tuned' >> "$lib"
echo 'pub fn added() {}' > "$tree/crates/apps/rusty_rsi/harness/src/added.rs"
rm "$tree/crates/apps/rusty_rsi/harness/src/gone.rs"
mkdir -p "$tree/.git/hooks" && echo evil > "$tree/.git/hooks/post-commit"
echo 'Tuned the search loop.' > "$reply"
"#;

struct World {
    scratch: Scratch,
    workspace: PathBuf,
    home: PathBuf,
    private: PathBuf,
    outside: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let scratch = Scratch::new(name);
        let workspace = scratch.path("ws");
        let src = workspace.join("crates/apps/rusty_rsi/harness/src");
        std::fs::create_dir_all(&src).expect("src");
        std::fs::write(workspace.join(".git"), "gitdir: /repo/.git/worktrees/x\n").expect("git");
        std::fs::write(src.join("lib.rs"), "pub fn main() {}\n").expect("lib");
        std::fs::write(src.join("gone.rs"), "// removed by codex\n").expect("gone");
        let home = scratch.path("codex-home");
        let private = scratch.path("tasks/t/private/labels.txt");
        std::fs::create_dir_all(private.parent().expect("dir")).expect("tasks");
        std::fs::write(&private, "secret").expect("labels");
        let outside = scratch.path("outside");
        for dir in [&home, &outside] {
            std::fs::create_dir_all(dir).expect("dir");
        }
        Self {
            scratch,
            workspace,
            home,
            private,
            outside,
        }
    }

    fn config(&self, body: &str) -> CliConfig {
        self.config_for(CliAgent::Codex, body)
    }

    fn config_for(&self, agent: CliAgent, body: &str) -> CliConfig {
        let body = format!(
            "OUTSIDE='{}'\nPRIVATE='{}'\n{body}",
            self.outside.display(),
            self.private.display()
        );
        CliConfig {
            agent,
            program: fake(&self.scratch.path(agent.name()), agent.name(), &body),
            model: Some("gpt-test".into()),
            home: self.home.clone(),
            wall: Duration::from_secs(30),
            env: vec![],
        }
    }

    fn protected(&self) -> Vec<PathBuf> {
        vec![
            self.scratch.path("tasks"),
            self.scratch.path("state"),
            self.workspace.clone(),
        ]
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.workspace.join(path)).ok()
    }

    fn seen(&self, file: &str) -> String {
        std::fs::read_to_string(self.home.join(file)).unwrap_or_default()
    }
}

#[test]
fn codex_edits_a_copy_and_only_its_harness_changes_come_back() {
    let world = World::new("codex-edit");
    let executor = executor(&world.scratch);
    let proposer = CliProposer::new(
        &executor,
        world.config(EDITOR),
        world.scratch.path("staging"),
        world.protected(),
    )
    .expect("proposer");
    assert_eq!(proposer.model().map(|m| m.as_str()), Some("codex:gpt-test"));

    let proposal = proposer.propose(&[], &world.workspace).expect("proposal");
    assert_eq!(proposal.summary, "Tuned the search loop.");

    assert_eq!(
        world.read(LIB).as_deref(),
        Some("pub fn main() {}\n// tuned\n")
    );
    assert_eq!(
        world
            .read("crates/apps/rusty_rsi/harness/src/added.rs")
            .as_deref(),
        Some("pub fn added() {}\n")
    );
    assert!(world
        .read("crates/apps/rusty_rsi/harness/src/gone.rs")
        .is_none());
    assert_eq!(
        world.read(".git").as_deref(),
        Some("gitdir: /repo/.git/worktrees/x\n"),
        "the worktree's .git is never touched"
    );
    assert!(!world.workspace.join(".git/hooks").exists());

    let args = world.seen("args");
    assert!(
        args.starts_with("exec\n--dangerously-bypass-approvals-and-sandbox\n"),
        "{args}"
    );
    assert!(args.contains("\n-m\ngpt-test\n"), "{args}");
    assert!(
        !args.contains(world.workspace.to_str().expect("utf-8")),
        "codex never sees the worktree: {args}"
    );
    assert!(world
        .seen("prompt")
        .contains("crates/apps/rusty_rsi/harness/src/"));
    let probes = world.seen("probes");
    assert_eq!(
        probes, "outside denied\nprivate denied\ntmp ok\nnull ok\ndevice denied\ninet ok\n",
        "no write outside, no private data, a private TMPDIR, no device but null and urandom, internet sockets"
    );
    assert!(!world.outside.join("escape").exists());
    let staging = world.scratch.path("staging");
    assert_eq!(
        std::fs::read_dir(&staging)
            .map(Iterator::count)
            .unwrap_or(0),
        0,
        "the staging copy is removed"
    );
}

#[test]
fn a_failed_codex_run_is_an_error_that_names_the_cause() {
    let world = World::new("codex-fail");
    let executor = executor(&world.scratch);
    let before = world.read(LIB);
    for (body, expected) in [
        (
            "echo 'ERROR: 401 Unauthorized' >&2\nexit 1\n",
            "codex login",
        ),
        ("cat > /dev/null\nexit 0\n", "wrote no reply"),
    ] {
        let proposer = CliProposer::new(
            &executor,
            world.config(body),
            world.scratch.path("staging"),
            world.protected(),
        )
        .expect("proposer");
        let error = proposer
            .propose(&[], &world.workspace)
            .expect_err("a failed run");
        assert!(
            matches!(&error, RuntimeError::Model(m) if m.contains(expected)),
            "{error}"
        );
        assert_eq!(world.read(LIB), before, "nothing comes back from a failure");
    }
}

#[test]
fn codex_does_not_start_when_its_sandbox_could_reach_a_protected_path() {
    let world = World::new("codex-protected");
    let executor = executor(&world.scratch);
    let mut protected = world.protected();
    protected.push(world.home.join("sessions"));
    let proposer = CliProposer::new(
        &executor,
        world.config(EDITOR),
        world.scratch.path("staging"),
        protected,
    )
    .expect("proposer");
    let error = proposer
        .propose(&[], &world.workspace)
        .expect_err("refused");
    assert!(matches!(error, RuntimeError::Sandbox(_)), "{error}");
    assert!(world.seen("args").is_empty(), "codex never ran");
}

/// A fake Claude Code that records its arguments, probes the sandbox from
/// its working directory (the copy), edits the copy and prints the JSON
/// envelope `claude -p --output-format json` prints.
const CLAUDE_EDITOR: &str = r#"
printf '%s\n' "$@" > "$CLAUDE_CONFIG_DIR/args"
pwd > "$CLAUDE_CONFIG_DIR/cwd"
cat > "$CLAUDE_CONFIG_DIR/prompt"
probe() { name="$1"; shift; if "$@" 2>/dev/null; then echo "$name ok"; else echo "$name denied"; fi; }
{
  probe outside sh -c "echo x > $OUTSIDE/escape"
  probe private sh -c "cat $PRIVATE"
} > "$CLAUDE_CONFIG_DIR/probes"
echo '// claude was here' >> crates/apps/rusty_rsi/harness/src/lib.rs
mkdir -p .git && echo evil > .git/config
printf '%s' '{"type":"result","subtype":"success","is_error":false,"result":"Pruned the tree search.\nDetails follow."}'
"#;

#[test]
fn claude_edits_a_copy_with_file_tools_only() {
    let world = World::new("claude-edit");
    let executor = executor(&world.scratch);
    let proposer = CliProposer::new(
        &executor,
        world.config_for(CliAgent::Claude, CLAUDE_EDITOR),
        world.scratch.path("staging"),
        world.protected(),
    )
    .expect("proposer");
    assert_eq!(
        proposer.model().map(|m| m.as_str()),
        Some("claude:gpt-test")
    );

    let proposal = proposer.propose(&[], &world.workspace).expect("proposal");
    assert_eq!(proposal.summary, "Pruned the tree search.");
    assert_eq!(
        world.read(LIB).as_deref(),
        Some("pub fn main() {}\n// claude was here\n")
    );
    assert_eq!(
        world.read(".git").as_deref(),
        Some("gitdir: /repo/.git/worktrees/x\n"),
        "the worktree's .git is never touched"
    );
    let args = world.seen("args");
    for needed in [
        "-p",
        "--restricted",
        "acceptEdits",
        "Read,Edit,Write,Glob,Grep",
    ] {
        assert!(args.lines().any(|a| a == needed), "{needed}: {args}");
    }
    assert!(
        world.seen("cwd").trim().ends_with("/tree"),
        "claude works in the copy: {}",
        world.seen("cwd")
    );
    assert!(world
        .seen("prompt")
        .contains("crates/apps/rusty_rsi/harness/src/"));
    assert_eq!(world.seen("probes"), "outside denied\nprivate denied\n");
}

#[test]
fn a_claude_error_envelope_is_an_error() {
    let world = World::new("claude-fail");
    let executor = executor(&world.scratch);
    let before = world.read(LIB);
    for (body, expected) in [
        (
            "cat > /dev/null\nprintf '%s' '{\"type\":\"result\",\"is_error\":true,\"result\":\"Prompt is too long\"}'\n",
            "Prompt is too long",
        ),
        (
            "cat > /dev/null\necho 'Invalid API key · Please run /login'\nexit 1\n",
            "CLAUDE_CONFIG_DIR",
        ),
    ] {
        let proposer = CliProposer::new(
            &executor,
            world.config_for(CliAgent::Claude, body),
            world.scratch.path("staging"),
            world.protected(),
        )
        .expect("proposer");
        let error = proposer
            .propose(&[], &world.workspace)
            .expect_err("a failed run");
        assert!(
            matches!(&error, RuntimeError::Model(m) if m.contains(expected)),
            "{error}"
        );
        assert_eq!(world.read(LIB), before, "nothing comes back from a failure");
    }
}

/// A fake `codex exec --json` that records its prompt, probes the sandbox,
/// writes the reply and prints Codex's event stream.
const CODEX_CHAT: &str = r#"
reply=""; prev=""
for a in "$@"; do
  case "$prev" in -o) reply="$a";; esac
  prev="$a"
done
printf '%s\n' "$@" > "$CODEX_HOME/args"
cat > "$CODEX_HOME/prompt"
probe() { name="$1"; shift; if "$@" 2>/dev/null; then echo "$name ok"; else echo "$name denied"; fi; }
{
  probe private sh -c "cat $PRIVATE"
  probe outside sh -c "echo x > $OUTSIDE/escape"
} > "$CODEX_HOME/probes"
printf 'def solve(x):\n    return x\n' > "$reply"
echo '{"type":"thread.started","thread_id":"t1"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"i0","type":"agent_message","text":"def solve(x)"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":321,"cached_input_tokens":300,"output_tokens":45}}'
"#;

fn messages() -> Vec<Message> {
    vec![
        Message {
            role: Role::System,
            content: "You write Python.".into(),
        },
        Message {
            role: Role::User,
            content: "Write solve(x).".into(),
        },
    ]
}

#[test]
fn codex_as_the_inner_model_is_metered_from_its_events() {
    let world = World::new("codex-model");
    let executor = executor(&world.scratch);
    let model = CodexModel::new(
        &executor,
        world.config(CODEX_CHAT),
        world.scratch.path("calls"),
        world.protected(),
    )
    .expect("model");
    assert_eq!(model.id().as_str(), "codex:gpt-test");

    let completion = model
        .complete(&messages(), 1000, Duration::from_secs(30))
        .expect("completion");
    assert_eq!(completion.text, "def solve(x):\n    return x\n");
    assert_eq!(
        (completion.prompt_tokens, completion.completion_tokens),
        (321, 45)
    );
    let args = world.seen("args");
    assert!(args.lines().any(|a| a == "--json"), "{args}");
    let prompt = world.seen("prompt");
    assert!(
        prompt.contains("## system\n\nYou write Python.")
            && prompt.contains("## user\n\nWrite solve(x)."),
        "{prompt}"
    );
    assert_eq!(world.seen("probes"), "private denied\noutside denied\n");
    assert_eq!(
        std::fs::read_dir(world.scratch.path("calls"))
            .map(Iterator::count)
            .unwrap_or(0),
        0,
        "each call's staging directory is removed"
    );
}

#[test]
fn an_unmetered_or_late_codex_call_is_an_error() {
    let world = World::new("codex-model-fail");
    let executor = executor(&world.scratch);
    let unmetered = "reply=\"\"; prev=\"\"\nfor a in \"$@\"; do case \"$prev\" in -o) reply=\"$a\";; esac; prev=\"$a\"; done\ncat > /dev/null\necho hi > \"$reply\"\necho '{\"type\":\"turn.started\"}'\n";
    let slow = "cat > /dev/null\nsleep 30\n";
    for (body, timeout, expected) in [
        (unmetered, Duration::from_secs(30), "cannot be metered"),
        (slow, Duration::from_secs(1), "TimedOut"),
    ] {
        let model = CodexModel::new(
            &executor,
            world.config(body),
            world.scratch.path("calls"),
            world.protected(),
        )
        .expect("model");
        let error = model
            .complete(&messages(), 1000, timeout)
            .expect_err("no completion");
        assert!(
            matches!(&error, RuntimeError::Model(m) if m.contains(expected)),
            "{error}"
        );
    }
}

/// The real agent named by `program_var`, with its home from `home_var`
/// (or `~/.<agent>`) and the model from `RSI_OUTER_MODEL`.
fn real(agent: CliAgent, program_var: &str) -> CliConfig {
    let program = std::env::var_os(program_var).unwrap_or_else(|| panic!("{program_var}"));
    let home = std::env::var_os(agent.home_var()).map_or_else(
        || {
            PathBuf::from(std::env::var_os("HOME").expect("HOME"))
                .join(format!(".{}", agent.name()))
        },
        PathBuf::from,
    );
    CliConfig {
        agent,
        program: PathBuf::from(program).canonicalize().expect("agent binary"),
        model: std::env::var("RSI_OUTER_MODEL").ok(),
        home: home.canonicalize().expect("agent home"),
        wall: Duration::from_secs(600),
        env: PASSED_ENV
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|v| ((*name).to_owned(), v)))
            .collect(),
    }
}

fn real_proposal(agent: CliAgent, program_var: &str) {
    let world = World::new(&format!("{}-real", agent.name()));
    let executor = executor(&world.scratch);
    let proposer = CliProposer::new(
        &executor,
        real(agent, program_var),
        world.scratch.path("staging"),
        world.protected(),
    )
    .expect("proposer");
    let proposal = proposer.propose(&[], &world.workspace).expect("proposal");
    assert!(!proposal.summary.is_empty());
    assert!(world.read(LIB).is_some(), "the harness is still there");
}

/// For a machine with `codex login` done: set `RSI_OUTER_CODEX` to the
/// native binary (and `CODEX_HOME` if it is not `~/.codex`), then
/// `cargo test -p rsi-cli --test agents -- --ignored`.
#[test]
#[ignore = "needs a logged-in Codex CLI and network access"]
fn a_real_codex_run_proposes_inside_the_sandbox() {
    real_proposal(CliAgent::Codex, "RSI_OUTER_CODEX");
}

/// As above for Claude Code: `RSI_OUTER_CLAUDE` and `CLAUDE_CONFIG_DIR`.
#[test]
#[ignore = "needs a logged-in Claude Code CLI and network access"]
fn a_real_claude_run_proposes_inside_the_sandbox() {
    real_proposal(CliAgent::Claude, "RSI_OUTER_CLAUDE");
}

/// A real Codex completion as the inner model (`RSI_OUTER_CODEX` names
/// the binary here too).
#[test]
#[ignore = "needs a logged-in Codex CLI and network access"]
fn a_real_codex_completion_is_metered() {
    let world = World::new("codex-model-real");
    let executor = executor(&world.scratch);
    let model = CodexModel::new(
        &executor,
        real(CliAgent::Codex, "RSI_OUTER_CODEX"),
        world.scratch.path("calls"),
        world.protected(),
    )
    .expect("model");
    let completion = model
        .complete(&messages(), 1000, Duration::from_secs(300))
        .expect("completion");
    assert!(!completion.text.is_empty());
    assert!(completion.prompt_tokens > 0 && completion.completion_tokens > 0);
}
