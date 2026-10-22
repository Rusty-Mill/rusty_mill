//! A git worktree per card, with commands run in it under `rusty_sandbox`.
//!
//! The command can write only its worktree and a scratch directory, and can reach
//! no local socket (Unix or abstract) at all; the internet only if its stage asks. It never
//! gets the repository's `.git` writable: the runner commits afterwards,
//! outside the sandbox, naming the git directory explicitly (so a rewritten
//! `.git` file in the worktree cannot point the runner's git somewhere else)
//! and skipping hooks.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rusty_mill_board::domain::Card;
use rusty_sandbox::{Limits, ProcessExecutor, SandboxSpec, Sockets, Termination};
use rusty_worktree::{Author, Repo};

use crate::config::Stage;
use crate::ports::{Outcome, Work};

/// Longest output tail kept for a comment, in characters.
const TAIL_CHARS: usize = 1_500;
const MEMORY_BYTES: u64 = 8 << 30;
const FILE_BYTES: u64 = 1 << 30;
const OPEN_FILES: u64 = 4_096;
const PROCESSES: u64 = 1_024;

/// Directories a program usually needs to read to start at all.
pub fn default_read_roots() -> Vec<PathBuf> {
    ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

/// Worktrees under `work`, branched from `base` of `repo`.
pub struct GitWork {
    repo: Repo,
    work: PathBuf,
    base: String,
    exec: ProcessExecutor,
    read_roots: Vec<PathBuf>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl GitWork {
    /// `helper` is this program's own path (it serves `__sandbox`); the
    /// executor's status files go under `work/.state`.
    ///
    /// # Errors
    /// If `repo` or `work` cannot be resolved.
    pub fn new(
        repo: &Path,
        work: &Path,
        base: &str,
        helper: PathBuf,
        extra_read_roots: Vec<PathBuf>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(work).map_err(|e| format!("work dir: {e}"))?;
        let repo = Repo::open(repo)?;
        let work = work.canonicalize().map_err(text)?;
        let exec = ProcessExecutor::new(helper, vec!["__sandbox".into()], work.join(".state"));
        let mut read_roots = default_read_roots();
        read_roots.extend(extra_read_roots);
        read_roots.push(repo.git_dir());
        Ok(Self {
            repo,
            work,
            base: base.to_owned(),
            exec,
            read_roots,
        })
    }

    fn id8(card: &Card) -> String {
        card.id.to_string().chars().take(8).collect()
    }
}

impl Work for GitWork {
    fn prepare(&self, card: &Card) -> Result<PathBuf, String> {
        let id8 = Self::id8(card);
        let (dir, tmp) = (self.work.join(&id8), self.work.join(format!("{id8}.tmp")));
        std::fs::create_dir_all(&tmp).map_err(|e| format!("scratch dir: {e}"))?;
        self.repo
            .add_branch(&dir, &format!("mill/{id8}"), &self.base)
    }

    fn run(
        &self,
        stage: &Stage,
        argv: &[String],
        dir: &Path,
        env: &[(String, String)],
    ) -> Result<Outcome, String> {
        let tmp = PathBuf::from(format!("{}.tmp", dir.display()));
        let mut env = env.to_vec();
        env.push(("TMPDIR".into(), tmp.display().to_string()));
        // Tools write caches and config under HOME; unless the operator passed one, it is the scratch dir.
        if !env.iter().any(|(k, _)| k == "HOME") {
            env.push(("HOME".into(), tmp.display().to_string()));
        }
        let wall = Duration::from_secs(stage.wall_secs);
        let limits = Limits::new(
            wall.max(Duration::from_secs(1)),
            wall,
            MEMORY_BYTES,
            FILE_BYTES,
            OPEN_FILES,
            PROCESSES,
        )
        .map_err(text)?;
        let write_roots = vec![dir.to_path_buf(), tmp, PathBuf::from("/dev/null")];
        let spec = SandboxSpec::new(
            self.read_roots.clone(),
            write_roots,
            dir.to_path_buf(),
            env,
            limits,
        )
        .map_err(text)?;
        // Never `NoInternet`: it leaves Unix and abstract sockets open, and a control
        // daemon behind one can act outside the filesystem confinement.
        let sockets = if stage.network {
            Sockets::InternetOnly
        } else {
            Sockets::NoEndpoints
        };
        let (program, args) = argv.split_first().ok_or("empty command")?;
        let out = self
            .exec
            .exec_with(&spec, program, args, Stdio::null(), sockets)
            .map_err(text)?;

        let summary = match out.termination {
            Termination::Exited(code) => format!("exit {code} in {:.0}s", out.wall.as_secs_f64()),
            Termination::Signaled(sig) => format!(
                "killed by signal {sig} after {:.0}s",
                out.wall.as_secs_f64()
            ),
            Termination::TimedOut => format!("timed out after {}s", stage.wall_secs),
        };
        let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
        all.push_str(&String::from_utf8_lossy(&out.stderr));
        let skip = all.chars().count().saturating_sub(TAIL_CHARS);
        Ok(Outcome {
            ok: out.termination.succeeded(),
            summary,
            tail: all.chars().skip(skip).collect(),
        })
    }

    fn changed(&self, dir: &Path) -> Result<Vec<String>, String> {
        self.repo.changed(dir)
    }

    fn discard(&self, dir: &Path) -> Result<(), String> {
        self.repo.discard(dir)
    }

    fn commit(&self, dir: &Path, message: &str) -> Result<bool, String> {
        let author = Author {
            name: "mill-runner",
            email: "mill-runner@localhost",
        };
        self.repo.commit(dir, message, author)
    }
}
