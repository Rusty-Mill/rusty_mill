//! The alien lens's probe runner (#708, D89, D90): orch builds and runs
//! each model-written reproducer itself, sandboxed with no network.
//!
//! [`SandboxProbes`] is the shell's [`Probes`]. Per probe it makes a
//! scratch worktree of the candidate (base plus each diff, applied in
//! order), puts the probe at `tests/<file>` in the crate the site names,
//! and runs `cargo test --offline -p <crate> --test <probe> --no-run`, then
//! the test itself. Only when the candidate fails does it do the same on a
//! base tree. Both trees are removed when the probe is done, whatever
//! happened.
//!
//! How runs map (D90): on the candidate, a timeout or a kill by signal is
//! [`Run::BuildError`], so a hang is an `error` row and never a finding;
//! on base, either is an `Err`, in the build or the run, so the probe gets
//! no row. Failures to set up a tree or the sandbox are `Err` too.
//!
//! Isolation (D90): each tree has its own temp dir, `HOME` and cargo
//! home, and base builds into its own target dir, so nothing the candidate
//! wrote reaches the base run. Host-side writes into a tree refuse to go
//! through a symlink, since the candidate's diffs can add them.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use orch_lens::alien::Run;
use orch_lens::hunt::{Probe, Probes, Runs};
use rusty_sandbox::{
    ExecOutcome, Executor, Limits, ProcessExecutor, SandboxSpec, Sockets, Termination,
};
use rusty_worktree::Repo;

/// Wall-clock limit on each cargo invocation, the build and the run (D90).
pub const PROBE_CARGO_LIMIT: Duration = Duration::from_secs(10 * 60);

/// Granted read-write: std opens it whenever a child gets a null stdio.
const DEV_NULL: &str = "/dev/null";
/// System directories every toolchain reads.
const SYSTEM_ROOTS: [&str; 5] = ["/bin", "/usr", "/lib", "/lib64", "/etc"];
/// What cargo's registry and git caches are called under a cargo home.
const CARGO_CACHES: [&str; 2] = ["registry", "git"];

/// The sandboxed executor: `rusty_sandbox` with no socket endpoints at
/// all (anonymous socketpairs, which rustc needs to start its linker,
/// still work), the way the BBP runner confines a test workload.
pub struct NoNet(ProcessExecutor);

impl NoNet {
    /// This binary as the helper, re-invoked as `rusty_orch __sandbox`.
    /// `state` is the sandbox's own state dir, outside every probe's reach.
    pub fn current(state: &Path) -> Result<Self, String> {
        let me = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        Ok(Self(ProcessExecutor::new(
            me,
            vec!["__sandbox".into()],
            state.to_path_buf(),
        )))
    }
}

impl Executor for NoNet {
    type Error = rusty_sandbox::Error;
    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, rusty_sandbox::Error> {
        self.0
            .exec_with(spec, program, args, Stdio::null(), Sockets::NoEndpoints)
    }
}

/// The Rust toolchain a probe builds with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// The toolchain's own `bin` (holding `cargo` and `rustc`), not a
    /// rustup proxy, which would need rustup's home.
    pub bin: PathBuf,
    /// The toolchain root, read-only.
    pub sysroot: PathBuf,
    /// The user's cargo home: its `registry` and `git` caches are read,
    /// never written. Probes build `--offline`, so dependencies must be in
    /// these caches already.
    pub cargo_home: PathBuf,
}

impl Toolchain {
    /// `rustc --print sysroot` and `$CARGO_HOME` (else `$HOME/.cargo`).
    pub fn detect() -> Result<Self, String> {
        let out = Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .map_err(|e| format!("rustc --print sysroot: {e}"))?;
        if !out.status.success() {
            return Err("rustc --print sysroot failed".to_owned());
        }
        let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .ok_or("neither CARGO_HOME nor HOME is set")?;
        let canon = |p: &Path| {
            p.canonicalize()
                .map_err(|e| format!("{}: {e}", p.display()))
        };
        Ok(Self {
            bin: canon(&sysroot.join("bin"))?,
            sysroot: canon(&sysroot)?,
            cargo_home: canon(&home)?,
        })
    }
}

/// The code one hunt's probes run against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Full commit id of the candidate's base.
    pub base: String,
    /// Each diff's text, applied in order.
    pub diffs: Vec<String>,
}

/// Where one hunt's probes run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// `<state>/alien/probes/<task>`: one `<claim>-<n>` per probe.
    pub probes: PathBuf,
    /// The candidate's build cache, kept between this hunt's probes only
    /// (`<state>/alien/target/<task>/<claim>`). Base never uses it.
    pub target: PathBuf,
}

/// [`Probes`] over a sandbox executor: see the module docs.
pub struct SandboxProbes<'a, E> {
    exec: &'a E,
    repo: &'a Repo,
    toolchain: &'a Toolchain,
    candidate: Candidate,
    dirs: Dirs,
}

impl<'a, E: Executor> SandboxProbes<'a, E>
where
    E::Error: std::fmt::Display,
{
    pub fn new(
        exec: &'a E,
        repo: &'a Repo,
        toolchain: &'a Toolchain,
        candidate: Candidate,
        dirs: Dirs,
    ) -> Self {
        Self {
            exec,
            repo,
            toolchain,
            candidate,
            dirs,
        }
    }

    fn runs(&self, probe: &Probe<'_>, trees: &mut Trees<'_>) -> Result<Runs, String> {
        let cand = trees.add("candidate", &self.candidate.base)?;
        for (i, diff) in self.candidate.diffs.iter().enumerate() {
            apply(self.repo, &cand.src, diff).map_err(|e| format!("diff {}: {e}", i + 1))?;
        }
        let Some(pkg) = place(&cand.src, probe)? else {
            return Ok(Runs {
                candidate: Run::BuildError,
                base: None,
            });
        };
        let candidate = match self.cargo(&cand, &self.dirs.target, &pkg, probe)? {
            Ran::Built(Termination::Exited(0)) => Run::Passed,
            Ran::Built(Termination::Exited(_)) => Run::Failed,
            Ran::Built(_) | Ran::Unbuilt(_) => Run::BuildError,
        };
        if candidate != Run::Failed {
            return Ok(Runs {
                candidate,
                base: None,
            });
        }
        let base = trees.add("base", &self.candidate.base)?;
        let target = base.tmp.join("target");
        let base = match place(&base.src, probe)? {
            None => Run::BuildError,
            Some(pkg) => match self.cargo(&base, &target, &pkg, probe)? {
                Ran::Unbuilt(t @ (Termination::TimedOut | Termination::Signaled(_))) => {
                    return Err(format!("building on base: {t:?}"));
                }
                Ran::Unbuilt(_) => Run::BuildError,
                Ran::Built(Termination::Exited(0)) => Run::Passed,
                Ran::Built(Termination::Exited(_)) => Run::Failed,
                Ran::Built(t) => return Err(format!("on base: {t:?}")),
            },
        };
        Ok(Runs {
            candidate,
            base: Some(base),
        })
    }

    /// Build the probe, then, if it built, run it.
    fn cargo(
        &self,
        tree: &Tree,
        target: &Path,
        pkg: &str,
        probe: &Probe<'_>,
    ) -> Result<Ran, String> {
        let spec = self.spec(tree, target)?;
        let program = self.toolchain.bin.join("cargo");
        let program = program.to_str().ok_or("non-UTF-8 cargo path")?;
        let built = self.exec(&spec, program, &cargo_args(pkg, probe, true))?;
        if built != Termination::Exited(0) {
            return Ok(Ran::Unbuilt(built));
        }
        self.exec(&spec, program, &cargo_args(pkg, probe, false))
            .map(Ran::Built)
    }

    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<Termination, String> {
        self.exec
            .exec(spec, program, args)
            .map(|out| out.termination)
            .map_err(|e| format!("sandbox: {e}"))
    }

    /// Read the toolchain and the tree; write the tree, its target dir,
    /// its own temp dir (holding its private cargo home), and `/dev/null`.
    fn spec(&self, tree: &Tree, target: &Path) -> Result<SandboxSpec, String> {
        let tmp = &tree.tmp;
        let t = self.toolchain;
        let mut read: Vec<PathBuf> = SYSTEM_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();
        read.push(t.sysroot.clone());
        read.extend(
            CARGO_CACHES
                .iter()
                .map(|c| t.cargo_home.join(c))
                .filter(|p| p.exists()),
        );
        let home = tmp.join("cargo-home");
        cargo_home(&home, &t.cargo_home)?;
        std::fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let env = vec![
            ("PATH".to_owned(), format!("{}:/usr/bin:/bin", path(&t.bin))),
            ("CARGO_HOME".to_owned(), path(&home)),
            ("CARGO_TARGET_DIR".to_owned(), path(target)),
            ("HOME".to_owned(), path(tmp)),
            ("TMPDIR".to_owned(), path(tmp)),
            ("CARGO_TERM_COLOR".to_owned(), "never".to_owned()),
        ];
        let limits = Limits::new(
            PROBE_CARGO_LIMIT,
            PROBE_CARGO_LIMIT,
            16 << 30,
            4 << 30,
            4096,
            4096,
        )
        .map_err(|e| e.to_string())?;
        let write = vec![
            tree.src.clone(),
            target.to_path_buf(),
            tmp.clone(),
            PathBuf::from(DEV_NULL),
        ];
        SandboxSpec::new(read, write, tree.src.clone(), env, limits).map_err(|e| e.to_string())
    }
}

impl<E: Executor> Probes for SandboxProbes<'_, E>
where
    E::Error: std::fmt::Display,
{
    fn run(&mut self, probe: &Probe<'_>) -> Result<Runs, String> {
        let name = probe_dir(&probe.file)?;
        let mut trees = Trees::new(self.repo, self.dirs.probes.join(name))?;
        self.runs(probe, &mut trees)
    }
}

/// How one cargo pair ended: the build's termination when it did not
/// build, else the run's.
enum Ran {
    Unbuilt(Termination),
    Built(Termination),
}

/// `cargo test` for one probe, the build alone or the run.
fn cargo_args(pkg: &str, probe: &Probe<'_>, no_run: bool) -> Vec<String> {
    let test = probe.file.trim_end_matches(".rs");
    let mut args = ["test", "--offline", "-p", pkg, "--test", test]
        .map(str::to_owned)
        .to_vec();
    if no_run {
        args.push("--no-run".to_owned());
    }
    args
}

/// `probe_<claim>_<n>.rs` as the D90 directory name `<claim>-<n>`.
fn probe_dir(file: &str) -> Result<String, String> {
    file.strip_prefix("probe_")
        .and_then(|s| s.strip_suffix(".rs"))
        .and_then(|s| s.rsplit_once('_'))
        .filter(|(c, n)| {
            [c, n]
                .iter()
                .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|(c, n)| format!("{c}-{n}"))
        .ok_or_else(|| format!("{file:?} is not a probe file name"))
}

/// One scratch tree: its source and its own temp dir.
struct Tree {
    src: PathBuf,
    tmp: PathBuf,
}

/// One probe's scratch trees, removed when dropped, whatever happened.
struct Trees<'a> {
    repo: &'a Repo,
    root: PathBuf,
    added: Vec<PathBuf>,
}

impl<'a> Trees<'a> {
    fn new(repo: &'a Repo, root: PathBuf) -> Result<Self, String> {
        if root.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        }
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        Ok(Self {
            repo,
            root,
            added: Vec::new(),
        })
    }

    /// A fresh worktree `name` detached at `rev`, with a new, empty temp
    /// dir `<name>-tmp` beside it.
    fn add(&mut self, name: &str, rev: &str) -> Result<Tree, String> {
        let dir = self.root.join(name);
        let src = self.repo.add_detached(&dir, rev)?;
        self.added.push(dir);
        let tmp = self.root.join(format!("{name}-tmp"));
        std::fs::create_dir(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let tmp = tmp
            .canonicalize()
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        Ok(Tree { src, tmp })
    }
}

impl Drop for Trees<'_> {
    fn drop(&mut self) {
        for dir in self.added.drain(..) {
            if let Err(e) = self.repo.remove(&dir) {
                eprintln!("rusty_orch: removing probe tree {}: {e}", dir.display());
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&self.root) {
            eprintln!("rusty_orch: removing {}: {e}", self.root.display());
        }
    }
}

/// Apply one diff to the worktree at `tree`, through its admin dir (its
/// `.git` file is not trusted), with inherited `GIT_*` variables removed.
fn apply(repo: &Repo, tree: &Path, diff: &str) -> Result<(), String> {
    let admin = repo.admin_dir(tree)?;
    let mut cmd = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd
        .arg("--git-dir")
        .arg(&admin)
        .arg("--work-tree")
        .arg(tree)
        .args(["apply", "-"])
        .current_dir(tree)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git apply: {e}"))?;
    if let Some(mut pipe) = child.stdin.take() {
        use std::io::Write;
        // A git that stops reading early reports its own reason below.
        let _ = pipe.write_all(diff.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git apply: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git apply: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Write the probe into the crate its site names in `tree`, and give that
/// crate's package name. `None` when no crate in the tree matches, which
/// is a probe that cannot build there. Refuses to write through a
/// symlink: every directory down to `tests` must be a real one, and the
/// file must be new.
fn place(tree: &Path, probe: &Probe<'_>) -> Result<Option<String>, String> {
    let Some((dir, pkg)) = crate_of(tree, probe.site)? else {
        return Ok(None);
    };
    let tests = dir.join("tests");
    match tests.symlink_metadata() {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", tests.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&tests).map_err(|e| format!("{}: {e}", tests.display()))?;
        }
        Err(e) => return Err(format!("{}: {e}", tests.display())),
    }
    if !real_dirs(tree, &tests) {
        return Err(format!("{} is reached through a symlink", tests.display()));
    }
    let file = tests.join(&probe.file);
    // `create_new` (O_CREAT | O_EXCL) fails on any existing entry,
    // symlinks included, without following it.
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    use std::io::Write;
    out.write_all(probe.source.as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(Some(pkg))
}

/// Whether `dir` is `tree` or under it with every directory on the way,
/// `dir` included, a real directory and not a symlink.
fn real_dirs(tree: &Path, dir: &Path) -> bool {
    let Ok(rel) = dir.strip_prefix(tree) else {
        return false;
    };
    let mut at = tree.to_path_buf();
    for part in rel.components() {
        let Component::Normal(name) = part else {
            return false;
        };
        at.push(name);
        if !at.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            return false;
        }
    }
    true
}

/// The crate `site` names in `tree`: for a file path, the nearest package
/// above it; for `crate::module::fn`, the package whose library name
/// (hyphens as underscores) is the first segment. Its directory and
/// package name.
fn crate_of(tree: &Path, site: &str) -> Result<Option<(PathBuf, String)>, String> {
    if let Some((krate, _)) = site.split_once("::") {
        return Ok(packages(tree)?
            .into_iter()
            .find(|(_, name)| name.replace('-', "_") == krate));
    }
    let rel = Path::new(site);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Ok(None);
    }
    let mut dir = tree.join(rel);
    while dir.pop() && dir.starts_with(tree) {
        if !real_dirs(tree, &dir) {
            continue;
        }
        if let Some(name) = package_name(&dir.join("Cargo.toml"))? {
            return Ok(Some((dir, name)));
        }
    }
    Ok(None)
}

/// Every package in `tree`, by directory and name. Symlinks, `target` and
/// hidden directories are not followed.
fn packages(tree: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    let mut dirs = vec![tree.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if let Some(name) = package_name(&dir.join("Cargo.toml"))? {
            out.push((dir.clone(), name));
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let skip = entry.file_name().to_string_lossy().starts_with('.')
                || entry.file_name() == OsStr::new("target");
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && !skip {
                dirs.push(entry.path());
            }
        }
    }
    Ok(out)
}

/// The `[package]` name in a manifest, if it is a regular file with one.
fn package_name(manifest: &Path) -> Result<Option<String>, String> {
    match manifest.symlink_metadata() {
        Ok(m) if m.is_file() => {}
        _ => return Ok(None),
    }
    let text =
        std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        let Some(value) = line.strip_prefix("name").map(str::trim_start) else {
            continue;
        };
        let Some(value) = value.strip_prefix('=').map(str::trim) else {
            continue;
        };
        if in_package {
            return Ok(Some(value.trim_matches('"').to_owned()));
        }
    }
    Ok(None)
}

/// A new private cargo home at `home`: the user's registry and git caches
/// linked in (read-only through the sandbox), so cargo's lock and any
/// state it writes stay in the tree's temp dir. `home` must not exist yet,
/// so nothing a sandboxed run left there is followed.
fn cargo_home(home: &Path, user: &Path) -> Result<(), String> {
    std::fs::create_dir(home).map_err(|e| format!("{}: {e}", home.display()))?;
    for cache in CARGO_CACHES {
        let from = user.join(cache);
        if from.exists() {
            symlink(&from, &home.join(cache))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(from: &Path, to: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(from, to).map_err(|e| format!("{}: {e}", to.display()))
}

#[cfg(not(unix))]
fn symlink(_from: &Path, to: &Path) -> Result<(), String> {
    Err(format!("{}: the probe sandbox needs Linux", to.display()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// How the fake answers an exec, by (tree, build?).
    type Answer = fn(&str, bool) -> Termination;

    const LIB: &str = "pub fn f() -> u32 {\n    1\n}\n";
    const DIFF: &str = "--- a/crates/a/src/lib.rs\n+++ b/crates/a/src/lib.rs\n@@ -1,3 +1,3 @@\n pub fn f() -> u32 {\n-    1\n+    2\n }\n";
    const PROBE: &str = "#[test]\nfn t() {}\n";

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A workspace with one package, `my-crate`, at `crates/a`.
    fn repo(tag: &str) -> (PathBuf, Repo) {
        let root = std::env::temp_dir().join(format!("orch_probes_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("crates/a/src")).expect("mkdir");
        std::fs::write(
            repo.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\"]\n",
        )
        .expect("write");
        std::fs::write(
            repo.join("crates/a/Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .expect("write");
        std::fs::write(repo.join("crates/a/src/lib.rs"), LIB).expect("write");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        let repo = Repo::open(&repo).expect("open");
        (root, repo)
    }

    /// One exec: which tree, whether it was the build, and what the tree
    /// held then.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Call {
        tree: String,
        no_run: bool,
        patched: bool,
        probe_there: bool,
    }

    /// What one exec's environment held: its temp and target dirs, and
    /// whether a marker a candidate run leaves in its temp dir was there.
    #[derive(Debug, Clone)]
    struct Seen {
        tree: String,
        tmp: String,
        target: String,
        marker: bool,
    }

    /// Answers each exec by (tree, build?). Each candidate run also acts
    /// like hostile code: it leaves a marker in its temp dir and, given
    /// `outside`, swaps its cargo home for a symlink there.
    struct Fake {
        answer: Answer,
        outside: Option<PathBuf>,
        calls: RefCell<Vec<Call>>,
        seen: RefCell<Vec<Seen>>,
    }

    impl Fake {
        fn new(answer: Answer) -> Self {
            Self {
                answer,
                outside: None,
                calls: RefCell::new(Vec::new()),
                seen: RefCell::new(Vec::new()),
            }
        }

        fn hostile(&self, env: &std::collections::BTreeMap<String, String>) {
            std::fs::write(Path::new(&env["TMPDIR"]).join("marker"), "x").expect("marker");
            let Some(outside) = &self.outside else {
                return;
            };
            let home = Path::new(&env["CARGO_HOME"]);
            let _ = std::fs::remove_dir_all(home);
            symlink(outside, home).expect("symlink");
        }
    }

    impl Executor for Fake {
        type Error = String;
        fn exec(
            &self,
            spec: &SandboxSpec,
            _: &str,
            args: &[String],
        ) -> Result<ExecOutcome, String> {
            let cwd = spec.cwd();
            let tree = cwd
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let lib = std::fs::read_to_string(cwd.join("crates/a/src/lib.rs")).unwrap_or_default();
            let no_run = args.iter().any(|a| a == "--no-run");
            let env: std::collections::BTreeMap<_, _> = spec.env().iter().cloned().collect();
            self.seen.borrow_mut().push(Seen {
                tree: tree.clone(),
                tmp: env["TMPDIR"].clone(),
                target: env["CARGO_TARGET_DIR"].clone(),
                marker: Path::new(&env["TMPDIR"]).join("marker").exists(),
            });
            if tree == "candidate" && !no_run {
                self.hostile(&env);
            }
            self.calls.borrow_mut().push(Call {
                tree: tree.clone(),
                no_run,
                patched: lib.contains("    2\n"),
                probe_there: cwd.join("crates/a/tests/probe_7_1.rs").is_file(),
            });
            Ok(ExecOutcome {
                termination: (self.answer)(&tree, no_run),
                stdout: vec![],
                stderr: vec![],
                wall: Duration::ZERO,
            })
        }
    }

    fn toolchain(root: &Path) -> Toolchain {
        Toolchain {
            bin: root.join("tc/bin"),
            sysroot: root.join("tc"),
            cargo_home: root.join("cargo"),
        }
    }

    fn base(repo: &Repo) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo.root())
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Run one probe on `site` with `diffs`; the calls made, and whether
    /// the probe's scratch dir is gone afterwards.
    fn run(
        tag: &str,
        site: &str,
        diffs: &[&str],
        answer: Answer,
    ) -> (Result<Runs, String>, Vec<Call>, bool) {
        let (root, repo) = repo(tag);
        let exec = Fake::new(answer);
        let (got, gone) = run_with(&root, &repo, &exec, site, diffs);
        let calls = exec.calls.borrow().clone();
        let _ = std::fs::remove_dir_all(&root);
        (got, calls, gone)
    }

    /// [`run`] on a given repo and executor; the result and whether the
    /// probe's scratch dir is gone afterwards.
    fn run_with(
        root: &Path,
        repo: &Repo,
        exec: &Fake,
        site: &str,
        diffs: &[&str],
    ) -> (Result<Runs, String>, bool) {
        let tc = toolchain(root);
        let dirs = Dirs {
            probes: root.join("state/alien/probes/t"),
            target: root.join("state/alien/target"),
        };
        let candidate = Candidate {
            base: base(repo),
            diffs: diffs.iter().map(|d| (*d).to_owned()).collect(),
        };
        let mut probes = SandboxProbes::new(exec, repo, &tc, candidate, dirs.clone());
        let got = probes.run(&Probe {
            file: "probe_7_1.rs".to_owned(),
            site,
            source: PROBE,
        });
        (got, !dirs.probes.join("7-1").exists())
    }

    /// A diff adding `path` as a symlink to `to`.
    #[cfg(unix)]
    fn link_diff(path: &str, to: &Path) -> String {
        let to = to.to_string_lossy();
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 120000\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{to}\n\\ No newline at end of file\n"
        )
    }

    /// Every entry under `dir`, sorted.
    #[cfg(unix)]
    fn listing(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    fn ok(_: &str, _: bool) -> Termination {
        Termination::Exited(0)
    }

    fn call(tree: &str, no_run: bool, patched: bool) -> Call {
        Call {
            tree: tree.to_owned(),
            no_run,
            patched,
            probe_there: true,
        }
    }

    #[test]
    fn a_passing_probe_runs_on_the_patched_candidate_only() {
        let (got, calls, gone) = run("pass", "crates/a/src/lib.rs", &[DIFF], ok);
        let want = Runs {
            candidate: Run::Passed,
            base: None,
        };
        assert_eq!(got, Ok(want));
        assert_eq!(
            calls,
            [
                call("candidate", true, true),
                call("candidate", false, true)
            ]
        );
        assert!(gone, "the trees are removed");
    }

    #[test]
    fn a_failing_probe_runs_again_on_the_unpatched_base() {
        let answer = |tree: &str, no_run: bool| match (tree, no_run) {
            ("candidate", false) => Termination::Exited(101),
            _ => Termination::Exited(0),
        };
        let (got, calls, gone) = run("regress", "my_crate::f", &[DIFF], answer);
        let want = Runs {
            candidate: Run::Failed,
            base: Some(Run::Passed),
        };
        assert_eq!(got, Ok(want));
        assert_eq!(
            calls,
            [
                call("candidate", true, true),
                call("candidate", false, true),
                call("base", true, false),
                call("base", false, false),
            ]
        );
        assert!(gone);
    }

    #[test]
    fn candidate_runs_map_hangs_and_kills_to_build_errors() {
        let cases: [(Answer, Run); 4] = [
            (
                |_, b| {
                    if b {
                        Termination::Exited(101)
                    } else {
                        Termination::Exited(0)
                    }
                },
                Run::BuildError,
            ),
            (
                |_, b| {
                    if b {
                        Termination::TimedOut
                    } else {
                        Termination::Exited(0)
                    }
                },
                Run::BuildError,
            ),
            (
                |_, b| {
                    if b {
                        Termination::Exited(0)
                    } else {
                        Termination::TimedOut
                    }
                },
                Run::BuildError,
            ),
            (
                |_, b| {
                    if b {
                        Termination::Exited(0)
                    } else {
                        Termination::Signaled(9)
                    }
                },
                Run::BuildError,
            ),
        ];
        for (i, (answer, want)) in cases.into_iter().enumerate() {
            let (got, calls, gone) = run(&format!("cand{i}"), "my_crate::f", &[DIFF], answer);
            let want = Runs {
                candidate: want,
                base: None,
            };
            assert_eq!(got, Ok(want), "case {i}");
            assert!(
                calls.iter().all(|c| c.tree == "candidate"),
                "case {i}: base not run"
            );
            assert!(gone, "case {i}");
        }
    }

    #[test]
    fn base_build_failure_is_recorded_but_a_base_hang_is_not_run() {
        let answer = |tree: &str, no_run: bool| match (tree, no_run) {
            ("candidate", true) => Termination::Exited(0),
            ("candidate", false) => Termination::Exited(101),
            _ => Termination::Exited(101),
        };
        let (got, _, _) = run("newcode", "my_crate::f", &[DIFF], answer);
        let want = Runs {
            candidate: Run::Failed,
            base: Some(Run::BuildError),
        };
        assert_eq!(got, Ok(want));

        let hangs: [Answer; 3] = [
            |tree, no_run| match (tree, no_run) {
                ("candidate", false) => Termination::Exited(101),
                ("base", true) => Termination::TimedOut,
                _ => Termination::Exited(0),
            },
            |tree, no_run| match (tree, no_run) {
                ("candidate", false) => Termination::Exited(101),
                ("base", false) => Termination::Signaled(24),
                _ => Termination::Exited(0),
            },
            |tree, no_run| match (tree, no_run) {
                ("candidate", false) => Termination::Exited(101),
                ("base", true) => Termination::Signaled(9),
                _ => Termination::Exited(0),
            },
        ];
        for (i, answer) in hangs.into_iter().enumerate() {
            let (got, _, gone) = run(&format!("hang{i}"), "my_crate::f", &[DIFF], answer);
            assert!(got.is_err(), "case {i}: {got:?}");
            assert!(gone, "case {i}");
        }
    }

    #[test]
    fn a_site_naming_no_crate_does_not_build_and_runs_nothing() {
        for site in ["other::f", "README.md", "../outside.rs", "/abs/lib.rs"] {
            let (got, calls, gone) = run("nocrate", site, &[DIFF], ok);
            let want = Runs {
                candidate: Run::BuildError,
                base: None,
            };
            assert_eq!(got, Ok(want), "{site}");
            assert!(calls.is_empty(), "{site}");
            assert!(gone, "{site}");
        }
    }

    #[test]
    fn a_diff_that_does_not_apply_is_not_run() {
        let bad = DIFF.replace("-    1", "-    9");
        let (got, calls, gone) = run("bad", "my_crate::f", &[&bad], ok);
        let err = got.expect_err("not applied");
        assert!(err.contains("diff 1"), "{err}");
        assert!(calls.is_empty());
        assert!(gone);
    }

    #[test]
    fn probe_file_names_become_their_directories() {
        assert_eq!(probe_dir("probe_7_1.rs"), Ok("7-1".to_owned()));
        assert_eq!(probe_dir("probe_120_3.rs"), Ok("120-3".to_owned()));
        for bad in [
            "probe_7.rs",
            "probe__1.rs",
            "probe_7_x.rs",
            "x_7_1.rs",
            "probe_7_1",
            "probe_../_1.rs",
        ] {
            assert!(probe_dir(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cargo_builds_then_runs_offline_one_test() {
        let probe = Probe {
            file: "probe_7_1.rs".to_owned(),
            site: "s",
            source: "",
        };
        let build = cargo_args("my-crate", &probe, true).join(" ");
        assert_eq!(
            build,
            "test --offline -p my-crate --test probe_7_1 --no-run"
        );
        let test = cargo_args("my-crate", &probe, false).join(" ");
        assert_eq!(test, "test --offline -p my-crate --test probe_7_1");
    }

    #[test]
    fn the_sandbox_writes_only_the_tree_cache_and_temp() {
        let (root, repo) = repo("spec");
        let exec = Fake::new(ok);
        let tc = toolchain(&root);
        let dirs = Dirs {
            probes: root.join("p"),
            target: root.join("target"),
        };
        let candidate = Candidate {
            base: String::new(),
            diffs: vec![],
        };
        let probes = SandboxProbes::new(&exec, &repo, &tc, candidate, dirs);
        let tmp = root.join("tmp");
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let tree = Tree {
            src: root.join("tree"),
            tmp: tmp.clone(),
        };
        let spec = probes.spec(&tree, &root.join("target")).expect("spec");
        let write: Vec<PathBuf> = vec![
            root.join("tree"),
            root.join("target"),
            tmp.clone(),
            PathBuf::from(DEV_NULL),
        ];
        assert_eq!(spec.write_roots(), write.as_slice());
        assert!(spec.read_roots().contains(&root.join("tc")));
        let env: std::collections::BTreeMap<_, _> = spec.env().iter().cloned().collect();
        assert_eq!(env["CARGO_HOME"], tmp.join("cargo-home").to_string_lossy());
        assert!(env["PATH"].starts_with(&*root.join("tc/bin").to_string_lossy()));
        assert_eq!(
            spec.limits(),
            &Limits::new(
                PROBE_CARGO_LIMIT,
                PROBE_CARGO_LIMIT,
                16 << 30,
                4 << 30,
                4096,
                4096,
            )
            .expect("limits")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn base_gets_fresh_temp_cargo_home_and_target_whatever_the_candidate_did() {
        let (root, repo) = repo("isolate");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let mut exec = Fake::new(|tree, no_run| match (tree, no_run) {
            ("candidate", false) => Termination::Exited(101),
            _ => Termination::Exited(0),
        });
        exec.outside = Some(outside.clone());
        let (got, gone) = run_with(&root, &repo, &exec, "my_crate::f", &[DIFF]);
        let want = Runs {
            candidate: Run::Failed,
            base: Some(Run::Passed),
        };
        assert_eq!(got, Ok(want));
        assert!(gone);
        let seen = exec.seen.borrow().clone();
        let (cand, base): (Vec<_>, Vec<_>) = seen.iter().partition(|s| s.tree == "candidate");
        assert_eq!((cand.len(), base.len()), (2, 2), "{seen:?}");
        for b in &base {
            assert!(!b.marker, "base saw the candidate's temp: {b:?}");
            assert_ne!(b.tmp, cand[0].tmp);
            assert_ne!(b.target, cand[0].target);
        }
        assert!(listing(&outside).is_empty(), "nothing was linked outside");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn a_symlinked_tests_dir_is_refused_before_anything_runs() {
        let (root, repo) = repo("linktests");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("mkdir");
        std::fs::write(outside.join("sentinel"), "s").expect("write");
        let exec = Fake::new(ok);
        let diff = link_diff("crates/a/tests", &outside);
        let (got, gone) = run_with(&root, &repo, &exec, "my_crate::f", &[&diff]);
        let err = got.expect_err("refused");
        assert!(err.contains("not a directory"), "{err}");
        assert!(exec.calls.borrow().is_empty());
        assert!(gone);
        assert_eq!(listing(&outside), ["sentinel"]);
        assert_eq!(
            std::fs::read_to_string(outside.join("sentinel")).expect("read"),
            "s"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn a_site_under_a_symlinked_dir_names_no_crate() {
        let (root, repo) = repo("linksite");
        let outside = root.join("outside");
        std::fs::create_dir_all(outside.join("src")).expect("mkdir");
        std::fs::write(
            outside.join("Cargo.toml"),
            "[package]\nname = \"evil\"\nversion = \"0.1.0\"\n",
        )
        .expect("write");
        let exec = Fake::new(ok);
        let diff = link_diff("crates/b", &outside);
        let (got, gone) = run_with(&root, &repo, &exec, "crates/b/src/lib.rs", &[&diff]);
        let want = Runs {
            candidate: Run::BuildError,
            base: None,
        };
        assert_eq!(got, Ok(want));
        assert!(exec.calls.borrow().is_empty());
        assert!(gone);
        assert_eq!(listing(&outside), ["Cargo.toml", "src"]);
        let (got, _) = run_with(&root, &repo, &exec, "evil::f", &[&diff]);
        assert_eq!(got.map(|r| r.candidate), Ok(Run::BuildError));
        assert_eq!(listing(&outside), ["Cargo.toml", "src"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn real_dirs_refuses_symlinks_and_escapes() {
        let (root, _repo) = repo("realdirs");
        let tree = root.join("repo");
        symlink(&tree.join("crates/a"), &tree.join("crates/l")).expect("link");
        assert!(real_dirs(&tree, &tree));
        assert!(real_dirs(&tree, &tree.join("crates/a/src")));
        assert!(!real_dirs(&tree, &tree.join("crates/l")));
        assert!(!real_dirs(&tree, &tree.join("crates/l/src")));
        assert!(!real_dirs(&tree, &tree.join("crates/a/missing")));
        assert!(!real_dirs(&tree, &root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn package_names_come_from_the_package_table_only() {
        let dir = std::env::temp_dir().join(format!("orch_probes_{}_manifest", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("Cargo.toml");
        let cases = [
            ("[package]\nname = \"a-b\"\n", Some("a-b")),
            (
                "[workspace]\nname = \"w\"\n[package]\nnamespace = 1\nname=\"x\"\n",
                Some("x"),
            ),
            ("[workspace]\nmembers = []\n", None),
            ("[dependencies]\nname = \"d\"\n", None),
        ];
        for (text, want) in cases {
            std::fs::write(&file, text).expect("write");
            assert_eq!(package_name(&file), Ok(want.map(str::to_owned)), "{text:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
