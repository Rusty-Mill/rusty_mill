//! Confined harness launch.
//!
//! A launcher with a `sandbox` runs its harness under `rusty_sandbox`, through
//! this binary's own `__sandbox` helper: Landlock limits the filesystem to the
//! system directories, the `bbp` binary, the launcher's declared roots and a
//! fresh turn workspace under `--work`; seccomp allows internet sockets only,
//! so no local daemon can act for it; the environment is exactly what is
//! declared here. The task directory is never reachable, so the harness can
//! read neither `master` nor the log and cannot run `bbp human`. Its MCP
//! server is a relay to a broker the moderator starts outside the sandbox
//! (see [`crate::broker`]). A sandbox the kernel cannot enforce means the
//! helper exits before the program runs: confinement fails closed.

use crate::broker;
use crate::launchers::Facts;
use crate::moderator::{task_dir, Config};
use rusty_bbp::{PrincipalId, Sha256, TurnId};
use rusty_sandbox::{HelperRequest, Limits, SandboxSpec, Sockets};
use rusty_serde::{Deserialize, Serialize};
use rusty_worktree::Repo;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const MEMORY_BYTES: u64 = 8 << 30;
const FILE_BYTES: u64 = 1 << 30;
const OPEN_FILES: u64 = 4_096;
/// Per user on Linux, so a brake rather than a quota.
const PROCESSES: u64 = 4_096;
/// The system's programs and libraries, and its entropy source, which git
/// and TLS clients read when `getrandom` is not what they were built with.
const SYSTEM_ROOTS: [&str; 7] = [
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib64",
    "/etc",
    "/dev/urandom",
];

/// What a launcher's harness may reach beyond its turn workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSandbox {
    /// Absolute paths it may read and execute: the provider's installation
    /// and its login, for example.
    #[rusty_serde(default)]
    pub read: Vec<String>,
    /// Absolute paths it may also write.
    #[rusty_serde(default)]
    pub write: Vec<String>,
    /// Names of the moderator's environment variables it keeps. `PATH`
    /// defaults to `/usr/bin:/bin`, and `HOME` and `TMPDIR` to a scratch
    /// directory beside the workspace.
    #[rusty_serde(default)]
    pub env: Vec<String>,
    /// Make the workspace a git worktree of `--repo-path`, detached at its
    /// `HEAD`. Worktrees of every turn and task share the repository's
    /// object store, which the harness may read but not write; each harness
    /// may write only its own worktree.
    #[rusty_serde(default)]
    pub checkout: bool,
    /// #598: globs of paths this harness must not see or touch (the
    /// factory's `sensitive_paths`, for a cloud harness). With `checkout`
    /// the workspace is then a [filtered copy](crate::filtered) of `HEAD`
    /// in a fresh repository instead of a worktree, which the launcher
    /// clones from; a diff it posts that touches a hidden path is refused,
    /// and a candidate on the copy's commit is pinned to the source commit.
    /// No root it is granted, the system roots and `bbp`'s directory
    /// included, may reach the repository.
    #[rusty_serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

/// One confined turn's places: the workspace it may write, the pipes of its
/// MCP relay, and the helper's setup-status file, which it cannot reach.
pub(crate) struct Turn {
    pub workspace: PathBuf,
    /// `HOME` and `TMPDIR`, kept out of the workspace so a checkout's diff
    /// shows only the harness's edits.
    scratch: PathBuf,
    /// The repository the workspace is a worktree of, with `checkout`.
    repo: Option<Repo>,
    /// The sandbox's `hide` globs: the workspace is a filtered copy, removed
    /// at [`Turn::finish`], and the broker refuses diffs touching them.
    hide: Vec<String>,
    /// With `hide`, the copy's commit and the source commit it pins to.
    view: Option<crate::filtered::View>,
    pub pipes: PathBuf,
    pub mcp_config: PathBuf,
    /// The turn's launch [`Facts`], readable but not writable by the harness.
    pub facts: PathBuf,
    status: PathBuf,
}

impl Turn {
    /// Make the workspace and the pipes afresh and write the turn's MCP
    /// configuration and launch `facts` beside the workspace. A turn of
    /// provider session `session` keeps the session's home, where its
    /// conversation is, and gets a fresh workspace at the session's one path,
    /// which a provider may key its conversations by.
    pub fn prepare(
        cfg: &Config,
        sandbox: &HarnessSandbox,
        turn: TurnId,
        session: Option<&str>,
        facts: &Facts,
    ) -> Result<Turn, String> {
        // The sandbox takes absolute roots only; a relative `--work` is
        // resolved against the moderator's working directory.
        let work = std::path::absolute(&cfg.work)
            .map_err(|e| format!("work {}: {e}", cfg.work.display()))?;
        let base = work
            .join("turns")
            .join(Sha256::of(cfg.task.0.as_bytes()).hex());
        let place =
            session.map_or_else(|| format!("turn-{}", turn.0), |id| format!("session-{id}"));
        let workspace = base.join(&place);
        let pipes = base.join(format!("turn-{}.mcp", turn.0));
        let scratch = base.join(format!("{place}.home"));
        let facts_path = base.join(format!("turn-{}.facts.json", turn.0));
        let status =
            task_dir(&cfg.dir, "agents", &cfg.task).join(format!("turn-{}.sandbox", turn.0));
        for dir in [&base, &scratch] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let filtered = sandbox.checkout && !sandbox.hide.is_empty();
        let repo = (sandbox.checkout && !filtered)
            .then(|| Repo::open(&cfg.repo))
            .transpose()?;
        // What a session's earlier turn left, if its cleanup never ran.
        if workspace.exists() {
            if let Some(repo) = &repo {
                repo.remove(&workspace)?;
            }
            if workspace.exists() {
                std::fs::remove_dir_all(&workspace)
                    .map_err(|e| format!("{}: {e}", workspace.display()))?;
            }
        }
        let mut view = None;
        let repo = if let Some(repo) = repo {
            repo.add_detached(&workspace, "HEAD")?;
            Some(repo)
        } else if filtered {
            view = Some(crate::filtered::copy(
                &cfg.repo,
                "HEAD",
                &workspace,
                &sandbox.hide,
            )?);
            None
        } else {
            std::fs::create_dir_all(&workspace)
                .map_err(|e| format!("{}: {e}", workspace.display()))?;
            None
        };
        // A filtered harness never reaches the source repository: its
        // launcher clones the copy instead.
        let facts = match &view {
            Some(_) => &Facts {
                repo: workspace.to_string_lossy().into_owned(),
                ..facts.clone()
            },
            None => facts,
        };
        broker::make_pipes(&pipes)?;
        let mcp_config = scratch.join("mcp.json");
        let text = format!(
            "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{{{}:{}}}}}}}}}\n",
            json(&cfg.bbp.to_string_lossy())?,
            json(broker::RELAY_ENV)?,
            json(&pipes.to_string_lossy())?,
        );
        rusty_atomic_file::write(&mcp_config, text.as_bytes())
            .map_err(|e| format!("{}: {e}", mcp_config.display()))?;
        rusty_atomic_file::write(&facts_path, facts.to_json()?.as_bytes())
            .map_err(|e| format!("{}: {e}", facts_path.display()))?;
        if let Some(parent) = status.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&status, b"").map_err(|e| format!("{}: {e}", status.display()))?;
        Ok(Turn {
            workspace,
            scratch,
            repo,
            hide: sandbox.hide.clone(),
            view,
            pipes,
            mcp_config,
            facts: facts_path,
            status,
        })
    }

    /// `program args` under the helper, with exactly `env` plus what
    /// `sandbox` keeps. Refused when any granted path reaches the task
    /// directory.
    pub fn command(
        &self,
        cfg: &Config,
        sandbox: &HarnessSandbox,
        program: String,
        args: Vec<String>,
        mut env: Vec<(String, String)>,
    ) -> Result<Command, String> {
        for name in &sandbox.env {
            if let Ok(value) = std::env::var(name) {
                env.push((name.clone(), value));
            }
        }
        let scratch = self.scratch.to_string_lossy().into_owned();
        for (name, default) in [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", scratch.as_str()),
            ("TMPDIR", scratch.as_str()),
        ] {
            if !env.iter().any(|(k, _)| k == name) {
                env.push((name.into(), default.into()));
            }
        }
        let (requests, replies) = broker::pipes(&self.pipes);
        let bbp = canonical(&cfg.bbp)?;
        let mut read: Vec<PathBuf> = SYSTEM_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();
        read.push(bbp.parent().ok_or("bbp binary has no directory")?.into());
        read.push(replies);
        read.push(self.facts.clone());
        read.extend(self.repo.as_ref().map(Repo::git_dir));
        read.extend(sandbox.read.iter().map(PathBuf::from));
        let mut write = vec![
            self.workspace.clone(),
            self.scratch.clone(),
            requests,
            PathBuf::from("/dev/null"),
        ];
        write.extend(sandbox.write.iter().map(PathBuf::from));
        let cpu = cfg
            .max_wall
            .clamp(Duration::from_secs(1), Duration::from_secs(7 * 24 * 3600));
        let limits = Limits::new(cpu, cpu, MEMORY_BYTES, FILE_BYTES, OPEN_FILES, PROCESSES)
            .map_err(|e| e.to_string())?;
        let spec = SandboxSpec::new(read, write, self.workspace.clone(), env, limits)
            .map_err(|e| format!("harness sandbox: {e}"))?;
        refuse_reaching(&spec, &cfg.dir)?;
        if !self.hide.is_empty() {
            let roots: Vec<&Path> = spec
                .read_roots()
                .iter()
                .chain(spec.write_roots())
                .map(PathBuf::as_path)
                .collect();
            refuse_exposing(&roots, &cfg.repo)?;
        }
        let request = HelperRequest {
            status: self.status.clone(),
            cwd: spec.cwd().clone(),
            cpu_secs: limits.cpu().as_secs(),
            memory_bytes: limits.memory_bytes(),
            file_bytes: limits.file_bytes(),
            open_files: limits.open_files(),
            processes: limits.processes(),
            read: spec.read_roots().to_vec(),
            write: spec.write_roots().to_vec(),
            env: spec.env().to_vec(),
            sockets: Sockets::InternetOnly,
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        };
        let mut command = Command::new(&cfg.bbp);
        command.arg("__sandbox").args(request.encode()).env_clear();
        Ok(command)
    }

    /// Start the broker for this turn, watching process `harness`, in the
    /// harness's process group `group` so that killing the group kills it too.
    pub fn broker(
        &self,
        cfg: &Config,
        principal: &PrincipalId,
        turn: TurnId,
        harness: u32,
        group: u32,
        log: Stdio,
    ) -> Result<std::process::Child, String> {
        let mut command = Command::new(&cfg.bbp);
        command
            .arg("mcp-broker")
            .arg("--pipes")
            .arg(&self.pipes)
            .args(["--harness", &harness.to_string()])
            .env("BBP_DIR", &cfg.dir)
            .env("BBP_TASK", &cfg.task.0)
            .env("BBP_PRINCIPAL", &principal.0)
            .env("BBP_TURN", turn.0.to_string())
            .env(crate::mcp::HIDDEN_ENV, self.hide.join("\n"))
            .env(
                crate::mcp::VIEW_ENV,
                self.view
                    .as_ref()
                    .map_or_else(String::new, |v| format!("{} {}", v.head, v.base)),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, group as i32);
        // Without process groups the moderator stops the broker by its handle.
        #[cfg(not(unix))]
        let _ = group;
        command
            .spawn()
            .map_err(|e| format!("starting the MCP broker: {e}"))
    }

    /// Remove the turn's worktree, if it has one: its edits reached the task
    /// through MCP, and the object store keeps nothing of them.
    pub fn finish(&self) -> Result<(), String> {
        if let Some(repo) = &self.repo {
            return repo.remove(&self.workspace);
        }
        if self.hide.is_empty() || !self.workspace.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&self.workspace)
            .map_err(|e| format!("{}: {e}", self.workspace.display()))
    }

    /// Why the helper refused to run the harness, if it did.
    pub fn setup_error(&self) -> Option<String> {
        std::fs::read_to_string(&self.status)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
}

/// Refuse a sandbox from which `dir` (or anything in it) can be reached.
fn refuse_reaching(spec: &SandboxSpec, dir: &Path) -> Result<(), String> {
    let dir = canonical(dir)?;
    let roots = spec.read_roots().iter().chain(spec.write_roots());
    for root in roots {
        let root = canonical(root).unwrap_or_else(|_| root.clone());
        if dir.starts_with(&root) || root.starts_with(&dir) {
            return Err(format!(
                "harness sandbox: {} would expose the task directory {}",
                root.display(),
                dir.display()
            ));
        }
    }
    Ok(())
}

/// Refuse launchers whose harnesses could replace a path some rule names
/// before its rule is made: Landlock opens each root and follows a symlink
/// there, so a root beneath a harness's write root is that harness's to
/// redirect, for its own later turns or another role's. No sandbox path of
/// any launcher, no system root, the `bbp` directory, or (with a checkout)
/// the repository's git directory may lie strictly beneath any launcher's
/// write root, and no write root may lie at or beneath the work directory.
pub fn refuse_swappable(
    launchers: &crate::moderator::Launchers,
    bbp: &Path,
    work: &Path,
    repo: &Path,
) -> Result<(), String> {
    let sandboxes: Vec<&HarnessSandbox> = launchers
        .iter()
        .filter_map(|(_, l)| l.sandbox.as_ref())
        .collect();
    let writes = sandboxes
        .iter()
        .flat_map(|s| &s.write)
        .map(|p| resolved(Path::new(p)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut roots = sandboxes
        .iter()
        .flat_map(|s| s.read.iter().chain(&s.write))
        .map(|p| resolved(Path::new(p)))
        .collect::<Result<Vec<_>, _>>()?;
    roots.extend(
        SYSTEM_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists()),
    );
    roots.push(
        resolved(bbp)?
            .parent()
            .ok_or("bbp binary has no directory")?
            .into(),
    );
    if sandboxes.iter().any(|s| s.checkout) {
        roots.push(resolved(&Repo::open(repo)?.git_dir())?);
    }
    for s in sandboxes.iter().filter(|s| !s.hide.is_empty()) {
        refuse_seeing(s, bbp, repo)?;
    }
    let work = resolved(&std::path::absolute(work).map_err(|e| e.to_string())?)?;
    for write in &writes {
        if write.starts_with(&work) {
            return Err(format!(
                "harness sandbox: write root {} lies in the work directory",
                write.display()
            ));
        }
        if let Some(root) = roots.iter().find(|r| *r != write && r.starts_with(write)) {
            return Err(format!(
                "harness sandbox: {} lies beneath write root {}, which a harness could swap",
                root.display(),
                write.display()
            ));
        }
    }
    Ok(())
}

/// A sandbox with `hide` must name valid globs and grant no root that
/// reaches the repository, whose files and object store hold what it hides:
/// neither its own roots nor those every harness gets, the system roots and
/// `bbp`'s directory. [`Turn::command`] checks the complete roots again.
fn refuse_seeing(s: &HarnessSandbox, bbp: &Path, repo: &Path) -> Result<(), String> {
    for g in &s.hide {
        crate::profiles::check_glob(g).map_err(|e| format!("harness sandbox: hide {g:?}: {e}"))?;
    }
    let bbp_dir = resolved(bbp)?
        .parent()
        .ok_or("bbp binary has no directory")?
        .to_path_buf();
    let mut roots: Vec<PathBuf> = s.read.iter().chain(&s.write).map(PathBuf::from).collect();
    roots.extend(
        SYSTEM_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists()),
    );
    roots.push(bbp_dir);
    let roots: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    refuse_exposing(&roots, repo)
}

/// Refuse any of `roots` that, resolved, reaches the repository at `repo`
/// or any place its git storage lives: its `.git` and object store wherever
/// links lead them, the target of every link inside `.git`, and the object
/// stores it borrows (`objects/info/alternates`, followed transitively).
fn refuse_exposing(roots: &[&Path], repo: &Path) -> Result<(), String> {
    let repo = resolved(&std::path::absolute(repo).map_err(|e| e.to_string())?)?;
    let held = git_storage(&repo)?;
    for root in roots {
        let root = resolved(root)?;
        if let Some(h) = held
            .iter()
            .find(|h| h.starts_with(&root) || root.starts_with(h))
        {
            return Err(format!(
                "harness sandbox: {} reaches {}, which holds the paths it hides",
                root.display(),
                h.display()
            ));
        }
    }
    Ok(())
}

/// Every resolved place that holds `repo`'s files or objects, `repo`
/// included: `.git`, the target of every link reached from it, followed
/// transitively through linked directories (a linked `objects` whose `pack`
/// is linked again), and the object stores it borrows
/// (`objects/info/alternates`, also transitively). What has been scanned is
/// kept apart from what is held, so holding a place never skips scanning it,
/// and each place is scanned once, so link cycles end. Fails closed on a
/// layout it cannot follow: a `.git` that is not a directory (a `gitdir:`
/// file), one that names a `commondir`, or a directory it cannot read.
fn git_storage(repo: &Path) -> Result<Vec<PathBuf>, String> {
    let dot_git = repo.join(".git");
    let refuse = |why: &str| {
        Err(format!(
            "harness sandbox: {}: {why}, so a hiding sandbox cannot be checked against it",
            dot_git.display()
        ))
    };
    if !dot_git.is_dir() {
        return refuse("not a directory");
    }
    if dot_git.join("commondir").symlink_metadata().is_ok() {
        return refuse("its storage is shared through commondir");
    }
    let git = canonical(&dot_git)?;
    let mut held = vec![repo.to_path_buf()];
    let mut scanned = std::collections::BTreeSet::new();
    let mut stores = std::collections::BTreeSet::new();
    let mut todo = vec![git.clone()];
    let objects = git.join("objects");
    let mut alternates = match objects.symlink_metadata() {
        Ok(_) => vec![canonical(&objects)?],
        Err(_) => Vec::new(),
    };
    loop {
        if let Some(d) = todo.pop() {
            if scanned.insert(d.clone()) {
                for target in link_targets(&d)? {
                    if target.is_dir() {
                        todo.push(target.clone());
                    }
                    held.push(target);
                }
                held.push(d);
            }
            continue;
        }
        let Some(store) = alternates.pop() else {
            break;
        };
        if !stores.insert(store.clone()) {
            continue;
        }
        todo.push(store.clone());
        let file = store.join("info/alternates");
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            // Unreadable, not UTF-8, or not a file: the stores it may name
            // cannot be known, so fail closed.
            Err(e) => {
                return Err(format!(
                    "harness sandbox: {}: {e}, so a hiding sandbox cannot be checked against it",
                    file.display()
                ))
            }
        };
        for line in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            alternates.push(canonical(&store.join(line))?);
        }
    }
    Ok(held)
}

/// The resolved target of every symlink under `dir`, which is not followed
/// while walking.
fn link_targets(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", d.display()))?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                out.push(resolved(&entry.path())?);
            } else if kind.is_dir() {
                todo.push(entry.path());
            }
        }
    }
    Ok(out)
}

/// `path` with its existing ancestors resolved through symlinks.
fn resolved(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return canonical(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{}: no existing ancestor", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: not a normal path", path.display()))?;
    Ok(resolved(parent)?.join(name))
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn json(s: &str) -> Result<String, String> {
    rusty_serde::json::to_string(&s).map_err(|e| e.to_string())
}

// The swap hazard is a Unix symlink one; harnesses are confined only there.
#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::moderator::{Launcher, Launchers};

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bbp_swap_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }
    fn launcher(read: &[&Path], write: &[&Path]) -> Launcher {
        let paths = |ps: &[&Path]| {
            ps.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        };
        Launcher {
            program: "harness".into(),
            args: vec![],
            builtin_tools: false,
            sandbox: Some(HarnessSandbox {
                read: paths(read),
                write: paths(write),
                ..HarnessSandbox::default()
            }),
            session: false,
        }
    }
    fn check(root: &Path, launchers: Launchers) -> Result<(), String> {
        refuse_swappable(
            &launchers,
            &root.join("bin/bbp"),
            &root.join("work"),
            &root.join("repo"),
        )
    }

    #[test]
    fn no_root_may_lie_beneath_any_harnesss_write_root() {
        let root = dir("roots");
        let (src, sub) = (root.join("src"), root.join("src/sub"));
        // One role writes what another's rule names: refused either way round.
        let across = Launchers::default()
            .with("a", launcher(&[], &[&src]))
            .with("b", launcher(&[&sub], &[]));
        assert!(check(&root, across).unwrap_err().contains("beneath"));
        let own = Launchers::default().with("a", launcher(&[&sub], &[&src]));
        assert!(check(&root, own).is_err());
        let system = Launchers::default().with("a", launcher(&[], &[Path::new("/")]));
        assert!(check(&root, system).is_err());
        let work = Launchers::default().with("a", launcher(&[], &[&root.join("work")]));
        assert!(check(&root, work).unwrap_err().contains("work"));
        // Shared roots, and disjoint ones, are fine.
        let shared = Launchers::default()
            .with("a", launcher(&[], &[&src]))
            .with("b", launcher(&[&src, &root.join("tools")], &[]));
        assert_eq!(check(&root, shared), Ok(()));
    }

    #[test]
    fn a_hiding_sandbox_may_not_reach_the_repository_or_name_a_bad_glob() {
        let root = dir("hide");
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let hiding = |read: &[&Path], hide: &str| {
            let mut l = launcher(read, &[]);
            if let Some(s) = l.sandbox.as_mut() {
                s.hide = vec![hide.into()];
            }
            Launchers::default().with("a", l)
        };
        let e = check(&root, hiding(&[&root], "secrets")).unwrap_err();
        assert!(e.contains("reaches"), "{e}");
        let e = check(&root, hiding(&[], "a/[b]")).unwrap_err();
        assert!(e.contains("hide"), "{e}");
        assert!(check(&root, hiding(&[], "secrets/**")).is_ok());
        // Roots every harness gets count too: `bbp` installed in the repo.
        let e = refuse_swappable(
            &hiding(&[], "secrets"),
            &repo.join("bbp"),
            &root.join("work"),
            &repo,
        )
        .unwrap_err();
        assert!(e.contains("reaches"), "{e}");
        // And so does an object store the repository borrows from.
        let store = root.join("store");
        std::fs::create_dir_all(repo.join(".git/objects/info")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(
            repo.join(".git/objects/info/alternates"),
            format!("{}\n", store.display()),
        )
        .unwrap();
        let e = check(&root, hiding(&[&store], "secrets")).unwrap_err();
        assert!(e.contains("store"), "{e}");
    }

    #[test]
    fn a_hiding_sandbox_may_not_reach_git_storage_linked_out_of_the_repository() {
        use std::os::unix::fs::symlink;
        let hiding = |read: &Path| {
            let mut l = launcher(&[read], &[]);
            if let Some(s) = l.sandbox.as_mut() {
                s.hide = vec!["secrets".into()];
            }
            Launchers::default().with("a", l)
        };
        // `.git` itself linked to a separate backing directory.
        let root = dir("linked_git");
        let backing = root.join("backing");
        std::fs::create_dir_all(backing.join("objects")).unwrap();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        symlink(&backing, root.join("repo/.git")).unwrap();
        let e = check(&root, hiding(&backing)).unwrap_err();
        assert!(e.contains("backing"), "{e}");
        // Only the object store linked away.
        let root = dir("linked_objects");
        let store = root.join("store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        symlink(&store, root.join("repo/.git/objects")).unwrap();
        let e = check(&root, hiding(&store)).unwrap_err();
        assert!(e.contains("store"), "{e}");
        // A link deeper in `.git`, and an alternate of an alternate.
        let root = dir("linked_deep");
        let (pack, second) = (root.join("pack"), root.join("second"));
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::create_dir_all(root.join("first/info")).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::create_dir_all(root.join("repo/.git/objects/info")).unwrap();
        symlink(&pack, root.join("repo/.git/objects/pack")).unwrap();
        std::fs::write(
            root.join("repo/.git/objects/info/alternates"),
            format!("{}\n", root.join("first").display()),
        )
        .unwrap();
        std::fs::write(
            root.join("first/info/alternates"),
            format!("{}\n", second.display()),
        )
        .unwrap();
        assert!(check(&root, hiding(&pack)).unwrap_err().contains("pack"));
        assert!(check(&root, hiding(&second))
            .unwrap_err()
            .contains("second"));
        assert!(check(&root, hiding(&root.join("bin"))).is_ok());
        // Linked twice: `objects` to a store whose `pack` and a loose-object
        // directory are linked again, with a link cycle back into the store.
        let root = dir("linked_twice");
        let (store, packs, loose) = (root.join("store"), root.join("packs"), root.join("loose"));
        for d in [&store, &packs, &loose] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        symlink(&store, root.join("repo/.git/objects")).unwrap();
        symlink(&packs, store.join("pack")).unwrap();
        symlink(&loose, store.join("ab")).unwrap();
        symlink(&store, packs.join("back")).unwrap();
        assert!(check(&root, hiding(&packs)).unwrap_err().contains("packs"));
        assert!(check(&root, hiding(&loose)).unwrap_err().contains("loose"));
        assert!(check(&root, hiding(&root.join("bin"))).is_ok());
        // Alternates it cannot read fail closed: not UTF-8, or not a file.
        let root = dir("bad_alternates");
        std::fs::create_dir_all(root.join("repo/.git/objects/info")).unwrap();
        std::fs::write(
            root.join("repo/.git/objects/info/alternates"),
            b"\xff\xfe\n",
        )
        .unwrap();
        let e = check(&root, hiding(&root.join("bin"))).unwrap_err();
        assert!(e.contains("alternates"), "{e}");
        std::fs::remove_file(root.join("repo/.git/objects/info/alternates")).unwrap();
        std::fs::create_dir(root.join("repo/.git/objects/info/alternates")).unwrap();
        let e = check(&root, hiding(&root.join("bin"))).unwrap_err();
        assert!(e.contains("alternates"), "{e}");
        // Layouts it cannot follow fail closed.
        let root = dir("gitdir_file");
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::write(root.join("repo/.git"), "gitdir: /elsewhere\n").unwrap();
        let e = check(&root, hiding(&root.join("bin"))).unwrap_err();
        assert!(e.contains("not a directory"), "{e}");
    }

    #[test]
    fn a_symlinked_spelling_does_not_hide_a_root_beneath_a_write_root() {
        let root = dir("links");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::os::unix::fs::symlink(src.join("sub"), root.join("alias")).unwrap();
        let hidden = Launchers::default()
            .with("a", launcher(&[], &[&src]))
            .with("b", launcher(&[&root.join("alias")], &[]));
        assert!(check(&root, hidden).is_err());
    }
}
