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
//! on base, either is an `Err`, so the probe gets no row. Failures to set
//! up a tree or the sandbox are `Err` too.

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
    /// `<state>/alien/target`: the build cache, kept between probes.
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
            apply(self.repo, &cand, diff).map_err(|e| format!("diff {}: {e}", i + 1))?;
        }
        let Some(pkg) = place(&cand, probe)? else {
            return Ok(Runs {
                candidate: Run::BuildError,
                base: None,
            });
        };
        let candidate = match self.cargo(&cand, &pkg, probe, &trees.tmp)? {
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
        let base = match place(&base, probe)? {
            None => Run::BuildError,
            Some(pkg) => match self.cargo(&base, &pkg, probe, &trees.tmp)? {
                Ran::Unbuilt(Termination::TimedOut) => {
                    return Err("timed out building on base".to_owned());
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
    fn cargo(&self, tree: &Path, pkg: &str, probe: &Probe<'_>, tmp: &Path) -> Result<Ran, String> {
        let spec = self.spec(tree, tmp)?;
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

    /// Read the toolchain and the tree; write the tree, the build cache,
    /// the private cargo home and temp dir, and `/dev/null`.
    fn spec(&self, tree: &Path, tmp: &Path) -> Result<SandboxSpec, String> {
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
        std::fs::create_dir_all(&self.dirs.target)
            .map_err(|e| format!("{}: {e}", self.dirs.target.display()))?;
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let env = vec![
            ("PATH".to_owned(), format!("{}:/usr/bin:/bin", path(&t.bin))),
            ("CARGO_HOME".to_owned(), path(&home)),
            ("CARGO_TARGET_DIR".to_owned(), path(&self.dirs.target)),
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
            tree.to_path_buf(),
            self.dirs.target.clone(),
            tmp.to_path_buf(),
            PathBuf::from(DEV_NULL),
        ];
        SandboxSpec::new(read, write, tree.to_path_buf(), env, limits).map_err(|e| e.to_string())
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

/// One probe's scratch trees, removed when dropped, whatever happened.
struct Trees<'a> {
    repo: &'a Repo,
    root: PathBuf,
    tmp: PathBuf,
    added: Vec<PathBuf>,
}

impl<'a> Trees<'a> {
    fn new(repo: &'a Repo, root: PathBuf) -> Result<Self, String> {
        if root.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        }
        let tmp = root.join("tmp");
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let tmp = tmp
            .canonicalize()
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        Ok(Self {
            repo,
            root,
            tmp,
            added: Vec::new(),
        })
    }

    /// A fresh worktree `name` detached at `rev`.
    fn add(&mut self, name: &str, rev: &str) -> Result<PathBuf, String> {
        let dir = self.root.join(name);
        let tree = self.repo.add_detached(&dir, rev)?;
        self.added.push(dir);
        Ok(tree)
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
/// is a probe that cannot build there.
fn place(tree: &Path, probe: &Probe<'_>) -> Result<Option<String>, String> {
    let Some((dir, pkg)) = crate_of(tree, probe.site)? else {
        return Ok(None);
    };
    let tests = dir.join("tests");
    std::fs::create_dir_all(&tests).map_err(|e| format!("{}: {e}", tests.display()))?;
    let file = tests.join(&probe.file);
    if file.symlink_metadata().is_ok() {
        return Err(format!("{} already exists", file.display()));
    }
    std::fs::write(&file, probe.source).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(Some(pkg))
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

/// A private cargo home at `home`: the user's registry and git caches
/// linked in (read-only through the sandbox), so cargo's lock and any
/// state it writes stay in the probe's temp dir.
fn cargo_home(home: &Path, user: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    for cache in CARGO_CACHES {
        let (from, to) = (user.join(cache), home.join(cache));
        if from.exists() && to.symlink_metadata().is_err() {
            symlink(&from, &to)?;
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

    /// Answers each exec by (tree, build?).
    struct Fake {
        answer: Answer,
        calls: RefCell<Vec<Call>>,
    }

    impl Fake {
        fn new(answer: Answer) -> Self {
            Self {
                answer,
                calls: RefCell::new(Vec::new()),
            }
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
        let tc = toolchain(&root);
        let dirs = Dirs {
            probes: root.join("state/alien/probes/t"),
            target: root.join("state/alien/target"),
        };
        let candidate = Candidate {
            base: base(&repo),
            diffs: diffs.iter().map(|d| (*d).to_owned()).collect(),
        };
        let mut probes = SandboxProbes::new(&exec, &repo, &tc, candidate, dirs.clone());
        let got = probes.run(&Probe {
            file: "probe_7_1.rs".to_owned(),
            site,
            source: PROBE,
        });
        let gone = !dirs.probes.join("7-1").exists();
        let calls = exec.calls.borrow().clone();
        let _ = std::fs::remove_dir_all(&root);
        (got, calls, gone)
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

        let hangs: [Answer; 2] = [
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
        let spec = probes.spec(&root.join("tree"), &tmp).expect("spec");
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
