//! A git worktree per card, with commands run in it under `rusty_sandbox`.
//!
//! The command can write only its worktree and a scratch directory, and can reach
//! no local socket (Unix or abstract) at all; the internet only if its stage asks. It never
//! gets the repository's `.git` writable: the runner commits afterwards,
//! outside the sandbox, naming the git directory explicitly (so a rewritten
//! `.git` file in the worktree cannot point the runner's git somewhere else)
//! and skipping hooks.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use rusty_mill_board::domain::Card;
use rusty_sandbox::{Limits, ProcessExecutor, SandboxSpec, Sockets, Termination};

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
    repo: PathBuf,
    work: PathBuf,
    base: String,
    exec: ProcessExecutor,
    read_roots: Vec<PathBuf>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Run git with `args`; stdout on success, stderr on failure.
fn git(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.first().map_or("", |a| a.to_str().unwrap_or("")),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
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
        let repo = repo
            .canonicalize()
            .map_err(|e| format!("repo {}: {e}", repo.display()))?;
        let work = work.canonicalize().map_err(text)?;
        let exec = ProcessExecutor::new(helper, vec!["__sandbox".into()], work.join(".state"));
        let mut read_roots = default_read_roots();
        read_roots.extend(extra_read_roots);
        read_roots.push(repo.join(".git"));
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

    fn gitdir(&self, dir: &Path) -> Result<PathBuf, String> {
        let name = dir.file_name().ok_or("worktree has no name")?;
        let gitdir = self.repo.join(".git").join("worktrees").join(name);
        gitdir
            .is_dir()
            .then_some(gitdir)
            .ok_or_else(|| format!("no git directory for worktree {}", dir.display()))
    }
}

/// The paths in `git status --porcelain=v1 -z` output: `XY path\0`, and for a rename or
/// copy the original path follows as its own field. Both sides count.
fn parse_status(out: &str) -> Vec<String> {
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    let mut paths = Vec::new();
    while let Some(entry) = fields.next() {
        let (Some(status), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        paths.push(path.to_owned());
        if status.contains(['R', 'C']) {
            paths.extend(fields.next().map(str::to_owned));
        }
    }
    paths
}

impl Work for GitWork {
    fn prepare(&self, card: &Card) -> Result<PathBuf, String> {
        let id8 = Self::id8(card);
        let (dir, tmp) = (self.work.join(&id8), self.work.join(format!("{id8}.tmp")));
        std::fs::create_dir_all(&tmp).map_err(|e| format!("scratch dir: {e}"))?;
        if !dir.exists() {
            let branch = format!("mill/{id8}");
            let repo = self.repo.as_os_str();
            let has_branch = git(&[
                os("-C"),
                repo,
                os("rev-parse"),
                os("--verify"),
                os("--quiet"),
                os(&format!("refs/heads/{branch}")),
            ])
            .is_ok();
            if has_branch {
                git(&[
                    os("-C"),
                    repo,
                    os("worktree"),
                    os("add"),
                    dir.as_os_str(),
                    os(&branch),
                ])?;
            } else {
                git(&[
                    os("-C"),
                    repo,
                    os("worktree"),
                    os("add"),
                    os("-b"),
                    os(&branch),
                    dir.as_os_str(),
                    os(&self.base),
                ])?;
            }
        }
        dir.canonicalize().map_err(text)
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
        let gitdir = self.gitdir(dir)?;
        let out = git(&[
            os("--git-dir"),
            gitdir.as_os_str(),
            os("--work-tree"),
            dir.as_os_str(),
            os("status"),
            os("--porcelain=v1"),
            os("-z"),
            os("--untracked-files=all"),
        ])?;
        Ok(parse_status(&out))
    }

    fn discard(&self, dir: &Path) -> Result<(), String> {
        let gitdir = self.gitdir(dir)?;
        let base = [
            os("--git-dir"),
            gitdir.as_os_str(),
            os("--work-tree"),
            dir.as_os_str(),
        ];
        git(&[&base[..], &[os("reset"), os("--hard"), os("-q")]].concat())?;
        git(&[&base[..], &[os("clean"), os("-fdq")]].concat())?;
        Ok(())
    }

    fn commit(&self, dir: &Path, message: &str) -> Result<bool, String> {
        let gitdir = self.gitdir(dir)?;
        let base = [
            os("--git-dir"),
            gitdir.as_os_str(),
            os("--work-tree"),
            dir.as_os_str(),
        ];
        let run = |tail: &[&std::ffi::OsStr]| git(&[&base[..], tail].concat());
        run(&[os("add"), os("-A")])?;
        if run(&[os("diff"), os("--cached"), os("--quiet")]).is_ok() {
            return Ok(false);
        }
        run(&[
            os("-c"),
            os("user.name=mill-runner"),
            os("-c"),
            os("user.email=mill-runner@localhost"),
            os("-c"),
            os("commit.gpgsign=false"),
            os("-c"),
            os("core.hooksPath=/dev/null"),
            os("commit"),
            os("--no-verify"),
            os("-q"),
            os("-m"),
            os(message),
        ])?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_status;

    #[test]
    fn status_paths_include_both_sides_of_a_rename_and_odd_names() {
        let out = " M src/a.rs\0?? new dir/b.rs\0R  to.rs\0from.rs\0A  c.rs\0";
        assert_eq!(
            parse_status(out),
            ["src/a.rs", "new dir/b.rs", "to.rs", "from.rs", "c.rs"]
        );
        assert!(parse_status("").is_empty());
        // A truncated entry is skipped rather than invented.
        assert_eq!(parse_status("x\0 M ok.rs\0"), ["ok.rs"]);
    }
}
