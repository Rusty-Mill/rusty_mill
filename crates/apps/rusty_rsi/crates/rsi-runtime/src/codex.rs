//! [`CodexProposer`]: harness rewrites by the Codex CLI (`codex exec`),
//! confined by this crate's sandbox (ADR-0005 §8).
//!
//! Codex's own Linux sandbox (bubblewrap) cannot start inside a Landlock
//! domain, so Codex runs with `--dangerously-bypass-approvals-and-sandbox`
//! and this sandbox is the boundary:
//!
//! - **Files.** Codex edits a staging copy of the worktree with no `.git`,
//!   so it cannot redirect the commit. It may write only that copy, its
//!   `CODEX_HOME`, a private `TMPDIR` and `/dev/null`. It may read the
//!   system directories, `/etc` (DNS and certificates), `/dev/urandom`,
//!   its install directory and the directory of `SSL_CERT_FILE` if set;
//!   no other device. Private task data, the run
//!   directory and every other protected path are out of reach, and the
//!   proposer refuses to start if the spec could reach one.
//! - **Network.** Open ([`Sockets::Internet`]): Codex must reach its
//!   model. Process-group, `io_uring` and x32 locks still apply.
//! - **Back into the worktree.** Every change in the copy is replayed
//!   through [`write_inside`], [`link_inside`] and [`remove_inside`], which
//!   never leave the worktree and never touch `.git`. The outer loop's
//!   allowlist then decides, as for any proposer.
//!
//! Codex signs in with its own login under `CODEX_HOME` (`codex login`);
//! no key passes through `rsi`. Its token use is not reported back.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rsi_core::{CostUsage, Limits, ModelId, Precedent, Proposal, Proposer, SandboxSpec};

use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;
use crate::grading::SYSTEM_READ_ROOTS;
#[cfg(unix)]
use crate::proposer::link_inside;
use crate::proposer::{describe, remove_inside, write_inside};
use crate::sandbox::Sockets;

/// Environment variables passed through to Codex when set: proxy and
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

/// Bytes of Codex's stderr quoted in an error.
const EXCERPT_BYTES: usize = 2000;

/// Address-space limit for Codex and the tools it runs.
const MEMORY_BYTES: u64 = 8 << 30;

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

/// How to run Codex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexConfig {
    /// The Codex binary, absolute.
    pub program: PathBuf,
    /// The model (`-m`); Codex's default when `None`.
    pub model: Option<String>,
    /// Codex's home: its login and state (`CODEX_HOME`), absolute.
    pub home: PathBuf,
    /// How long one proposal may take.
    pub wall: Duration,
    /// Extra environment, such as the [`PASSED_ENV`] that are set.
    pub env: Vec<(String, String)>,
}

impl CodexConfig {
    /// The directory Codex is installed in: the binary's directory, or its
    /// parent when that is a `bin` directory (an npm package or a prefix
    /// keeps Codex's helpers beside `bin`).
    #[must_use]
    pub fn install_root(&self) -> PathBuf {
        let dir = self.program.parent().unwrap_or(Path::new("/"));
        match dir.file_name() {
            Some(name) if name == "bin" => dir.parent().unwrap_or(dir).to_path_buf(),
            _ => dir.to_path_buf(),
        }
    }

    /// The `codex exec` arguments for a copy at `tree` and a reply file.
    #[must_use]
    pub fn args(&self, tree: &Path, reply: &Path) -> Vec<String> {
        let mut args: Vec<String> = [
            "exec",
            "--dangerously-bypass-approvals-and-sandbox",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--color",
            "never",
            "-C",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(tree.display().to_string());
        if let Some(model) = &self.model {
            args.extend(["-m".to_owned(), model.clone()]);
        }
        args.extend(["-o".to_owned(), reply.display().to_string(), "-".to_owned()]);
        args
    }
}

/// Rewrites the harness with `codex exec`, confined by the sandbox.
#[derive(Debug)]
pub struct CodexProposer<'a> {
    executor: &'a ProcessExecutor,
    config: CodexConfig,
    scratch: PathBuf,
    protected: Vec<PathBuf>,
    id: ModelId,
}

impl<'a> CodexProposer<'a> {
    /// A proposer that stages each proposal under `scratch` and refuses to
    /// run Codex if its sandbox could reach any `protected` path.
    ///
    /// # Errors
    /// [`RuntimeError::Model`] if the program, home or scratch path is
    /// relative, or the model name is not a valid id.
    pub fn new(
        executor: &'a ProcessExecutor,
        config: CodexConfig,
        scratch: PathBuf,
        protected: Vec<PathBuf>,
    ) -> Result<Self, RuntimeError> {
        for (what, path) in [
            ("program", &config.program),
            ("home", &config.home),
            ("scratch", &scratch),
        ] {
            if !path.is_absolute() {
                return Err(RuntimeError::Model(format!(
                    "codex {what} {} is not absolute",
                    path.display()
                )));
            }
        }
        let name = config
            .model
            .as_ref()
            .map_or_else(|| "codex".to_owned(), |m| format!("codex:{m}"));
        let id = ModelId::parse(&name).map_err(|e| RuntimeError::Model(e.to_string()))?;
        Ok(Self {
            executor,
            config,
            scratch,
            protected,
            id,
        })
    }

    fn spec(&self, staging: &Staging) -> Result<SandboxSpec, RuntimeError> {
        let mut read: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
        read.push("/etc".into());
        read.push(self.config.install_root());
        let cert_dir = self
            .config
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
                "CODEX_HOME".to_owned(),
                self.config.home.display().to_string(),
            ),
        ];
        env.extend(self.config.env.iter().cloned());
        let secs = self.config.wall.as_secs().max(1);
        let limits = Limits::new(
            Duration::from_secs(secs),
            self.config.wall,
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
                self.config.home.clone(),
                PathBuf::from("/dev/null"),
            ],
            staging.tree(),
            env,
            limits,
        )
        .map_err(|e| RuntimeError::Model(e.to_string()))?;
        if let Some(path) = self.protected.iter().find(|p| spec.can_reach(p)) {
            return Err(RuntimeError::Sandbox(format!(
                "the codex sandbox could reach {}",
                path.display()
            )));
        }
        Ok(spec)
    }
}

impl Proposer for CodexProposer<'_> {
    type Error = RuntimeError;

    fn model(&self) -> Option<&ModelId> {
        Some(&self.id)
    }

    fn propose(&self, history: &[Precedent], workspace: &Path) -> Result<Proposal, RuntimeError> {
        let staging = Staging::create(&self.scratch)?;
        copy_tree(workspace, &staging.tree())?;
        let prompt = staging.root.join("prompt.md");
        std::fs::write(&prompt, format!("{CONTRACT}\n\n{}", describe(history)))
            .map_err(|e| RuntimeError::io("writing the codex prompt", e))?;
        let reply = staging.tmp().join("reply.md");
        let spec = self.spec(&staging)?;
        let stdin =
            File::open(&prompt).map_err(|e| RuntimeError::io("opening the codex prompt", e))?;
        let program = self.config.program.display().to_string();
        let outcome = self.executor.exec_with(
            &spec,
            &program,
            &self.config.args(&staging.tree(), &reply),
            Stdio::from(stdin),
            Sockets::Internet,
        )?;
        if !outcome.termination.succeeded() {
            return Err(failure(&outcome.termination, &outcome.stderr));
        }
        let text = std::fs::read_to_string(&reply).map_err(|e| {
            RuntimeError::Model(format!(
                "codex wrote no reply ({e}): {}",
                excerpt(&outcome.stderr)
            ))
        })?;
        mirror(&staging.tree(), workspace)?;
        Ok(Proposal {
            summary: summary_line(&text),
            usage: CostUsage::default(),
        })
    }
}

/// A failed run as an error, with a hint for the common cases.
fn failure(termination: &rsi_core::Termination, stderr: &[u8]) -> RuntimeError {
    let text = String::from_utf8_lossy(stderr).to_lowercase();
    let hint = if text.contains("401 unauthorized") || text.contains("not logged in") {
        " (not logged in: run `codex login` with this CODEX_HOME)"
    } else if text.contains("429") || text.contains("usage_limit") || text.contains("rate limit") {
        " (rate limited)"
    } else {
        ""
    };
    RuntimeError::Model(format!(
        "codex ended with {termination:?}{hint}: {}",
        excerpt(stderr)
    ))
}

fn excerpt(stderr: &[u8]) -> String {
    let start = stderr.len().saturating_sub(EXCERPT_BYTES);
    String::from_utf8_lossy(&stderr[start..]).trim().to_owned()
}

/// The reply's first non-empty line, shortened for a commit message.
fn summary_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("codex proposal");
    line.chars().take(72).collect()
}

/// A proposal's private directory: `tree/` (the copy Codex edits) and
/// `tmp/`. Removed on drop.
struct Staging {
    root: PathBuf,
}

impl Staging {
    fn create(scratch: &Path) -> Result<Self, RuntimeError> {
        let n = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
        let root = scratch.join(format!("codex-{}-{n}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| RuntimeError::io("clearing an old codex staging dir", e))?;
        }
        for dir in [root.join("tree"), root.join("tmp")] {
            std::fs::create_dir_all(&dir)
                .map_err(|e| RuntimeError::io(format!("creating {}", dir.display()), e))?;
        }
        Ok(Self { root })
    }

    fn tree(&self) -> PathBuf {
        self.root.join("tree")
    }

    fn tmp(&self) -> PathBuf {
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
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rsi-codex-{name}-{}", std::process::id()));
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

    #[test]
    fn args_pin_the_bypass_and_the_isolation_flags() {
        let config = CodexConfig {
            program: "/opt/codex/bin/codex".into(),
            model: Some("gpt-x".into()),
            home: "/home/u/.codex".into(),
            wall: Duration::from_secs(60),
            env: vec![],
        };
        let args = config.args(Path::new("/s/tree"), Path::new("/s/tmp/reply.md"));
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
        let bare = CodexConfig {
            program: "/opt/tools/codex".into(),
            ..config
        };
        assert_eq!(bare.install_root(), PathBuf::from("/opt/tools"));
    }

    #[test]
    fn failures_carry_a_hint_and_the_end_of_stderr() {
        let e = failure(
            &rsi_core::Termination::Exited(1),
            b"...\nERROR: 401 Unauthorized",
        );
        assert!(e.to_string().contains("codex login"), "{e}");
        let e = failure(&rsi_core::Termination::TimedOut, b"slow");
        assert!(
            e.to_string().contains("TimedOut") && e.to_string().contains("slow"),
            "{e}"
        );
        assert_eq!(
            summary_line("\n  Tuned the search.\nmore"),
            "Tuned the search."
        );
        assert_eq!(summary_line(""), "codex proposal");
    }
}
