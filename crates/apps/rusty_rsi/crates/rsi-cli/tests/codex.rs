//! The Codex proposer through the real sandbox, with a fake `codex`: what
//! it may reach, what comes back into the worktree, and how it fails.
#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{executor, Scratch};
use rsi_core::Proposer;
use rsi_runtime::codex::{CodexConfig, CodexProposer};
use rsi_runtime::RuntimeError;

const LIB: &str = "crates/apps/rusty_rsi/harness/src/lib.rs";

/// Writes an executable `bin/codex` under `dir` running `body` in `sh`.
fn fake(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join("bin/codex");
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

    fn config(&self, body: &str) -> CodexConfig {
        let body = format!(
            "OUTSIDE='{}'\nPRIVATE='{}'\n{body}",
            self.outside.display(),
            self.private.display()
        );
        CodexConfig {
            program: fake(&self.scratch.path("codex"), &body),
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
    let proposer = CodexProposer::new(
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
        let proposer = CodexProposer::new(
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
    let proposer = CodexProposer::new(
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

/// A real Codex run, for a machine with `codex login` done: set
/// `RSI_OUTER_CODEX` to the native binary (and `CODEX_HOME` if it is not
/// `~/.codex`), then `cargo test -p rsi-cli --test codex -- --ignored`.
#[test]
#[ignore = "needs a logged-in Codex CLI and network access"]
fn a_real_codex_run_proposes_inside_the_sandbox() {
    let world = World::new("codex-real");
    let executor = executor(&world.scratch);
    let program = std::env::var_os("RSI_OUTER_CODEX").expect("RSI_OUTER_CODEX");
    let home = std::env::var_os("CODEX_HOME").map_or_else(
        || PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".codex"),
        PathBuf::from,
    );
    let config = CodexConfig {
        program: PathBuf::from(program).canonicalize().expect("codex binary"),
        model: std::env::var("RSI_OUTER_MODEL").ok(),
        home: home.canonicalize().expect("CODEX_HOME"),
        wall: Duration::from_secs(600),
        env: rsi_runtime::codex::PASSED_ENV
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|v| ((*name).to_owned(), v)))
            .collect(),
    };
    let proposer = CodexProposer::new(
        &executor,
        config,
        world.scratch.path("staging"),
        world.protected(),
    )
    .expect("proposer");
    let proposal = proposer.propose(&[], &world.workspace).expect("proposal");
    assert!(!proposal.summary.is_empty());
    assert!(world.read(LIB).is_some(), "the harness is still there");
}
