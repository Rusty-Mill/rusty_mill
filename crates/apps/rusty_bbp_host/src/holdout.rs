//! Holdout suites on the host (#597): the frozen holdout set, its versions,
//! and what a holdout run may show.
//!
//! A holdout set is a profile set whose profiles are all `holdout:` or
//! `golden:` named, plus a host directory of test files and the SHA-256 of
//! each. Sets are stored content-addressed at
//! `<dir>/profiles/<sha256(task)>.holdout/<digest>.json`, so every version a
//! task ever adopted stays readable; the core's `holdout_digest` names the
//! current one. The directory is mounted read-only into holdout runs only,
//! never into a build run, and its files are checked against the manifest
//! before every holdout run, so a changed or planted file fails the run.

use crate::profiles::ProfileSet;
use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// One file of the suite: its path below `root` and its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteFile {
    pub path: String,
    pub sha256: Sha256,
}

/// The frozen holdout set of a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldoutSet {
    /// The profiles, environment and limits of a holdout run.
    pub set: ProfileSet,
    /// Absolute host directory holding the suite's files; outside every
    /// sandbox mount of a build run.
    pub root: String,
    /// Every file under `root`, sorted by path.
    pub files: Vec<SuiteFile>,
}

/// The environment variable a holdout profile finds the suite under.
pub const ROOT_ENV: &str = "BBP_HOLDOUT";

impl HoldoutSet {
    /// A set of `set` over every file now under `root`.
    pub fn scan(set: ProfileSet, root: &Path) -> Result<HoldoutSet, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("holdout root {}: {e}", root.display()))?;
        let files = hash_tree(&root)?
            .into_iter()
            .map(|(path, sha256)| SuiteFile { path, sha256 })
            .collect();
        let root = root.to_str().ok_or("holdout root is not UTF-8")?.to_owned();
        let hs = HoldoutSet { set, root, files };
        hs.validate()?;
        Ok(hs)
    }

    /// The shape rules: holdout profile names, an absolute root, files.
    pub fn validate(&self) -> Result<(), String> {
        if self.set.profiles.is_empty() {
            return Err("holdout set has no profiles".into());
        }
        if let Some(p) = self
            .set
            .profiles
            .iter()
            .find(|p| !Suite::Holdout.owns_profile(&p.name))
        {
            return Err(format!(
                "holdout profile {:?} must start with holdout: or golden:",
                p.name
            ));
        }
        if !Path::new(&self.root).is_absolute() {
            return Err("holdout root must be absolute".into());
        }
        if self.files.is_empty() {
            return Err("holdout root holds no files".into());
        }
        Ok(())
    }

    /// The files under `root` are exactly the manifest's, byte for byte.
    pub fn verify_files(&self) -> Result<(), String> {
        let want: BTreeMap<String, Sha256> = self
            .files
            .iter()
            .map(|f| (f.path.clone(), f.sha256))
            .collect();
        let have = hash_tree(Path::new(&self.root))?;
        if have == want {
            return Ok(());
        }
        let differ: Vec<&String> = want
            .keys()
            .chain(have.keys())
            .filter(|p| want.get(*p) != have.get(*p))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Err(format!(
            "holdout files differ from the frozen set: {}",
            differ
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Does `self` only add to `old`? Every old file is unchanged, every old
    /// profile is kept as it was, and nothing else about the run changed.
    /// Pure. Anything else removes or changes tests and needs the human.
    pub fn extends(&self, old: &HoldoutSet) -> bool {
        let files_kept = old.files.iter().all(|f| self.files.contains(f));
        let profiles_kept = old
            .set
            .profiles
            .iter()
            .all(|p| self.set.profiles.contains(p));
        let same_run = ProfileSet {
            profiles: vec![],
            ..self.set.clone()
        } == ProfileSet {
            profiles: vec![],
            ..old.set.clone()
        };
        files_kept && profiles_kept && same_run && self.root == old.root
    }

    /// The profile set a holdout run executes: the suite's, with its root
    /// readable and named in the environment.
    pub fn run_set(&self) -> ProfileSet {
        let mut set = self.set.clone();
        set.read_roots.push(self.root.clone());
        set.env.retain(|(k, _)| k != ROOT_ENV);
        set.env.push((ROOT_ENV.into(), self.root.clone()));
        set
    }

    /// #597 2.2: no build run can reach the suite. Its root shares no
    /// directory with the build set's read roots or with `dirs` (the
    /// repository, the work root): neither contains the other, compared after
    /// resolving symlinks, so an alias counts as the directory it names.
    pub fn isolated_from(&self, build: &ProfileSet, dirs: &[&Path]) -> Result<(), String> {
        let root = canonical(Path::new(&self.root))?;
        let roots = build.read_roots.iter().map(PathBuf::from);
        for other in roots.chain(dirs.iter().map(|d| d.to_path_buf())) {
            let resolved = canonical(&other)?;
            if resolved.starts_with(&root) || root.starts_with(&resolved) {
                return Err(format!(
                    "holdout root {} overlaps {}, which build runs can read",
                    self.root,
                    other.display()
                ));
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        rusty_serde::json::to_string(self)
            .map(String::into_bytes)
            .map_err(|e| e.to_string())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<HoldoutSet, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        rusty_serde::json::from_str(text).map_err(|e| e.to_string())
    }
}

/// `p` made absolute with its symlinks resolved. A path that does not exist
/// yet resolves through its deepest existing ancestor, so a symlink above it
/// still counts; `..` is refused rather than interpreted.
fn canonical(p: &Path) -> Result<PathBuf, String> {
    let abs = std::path::absolute(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if abs.components().any(|c| c == Component::ParentDir) {
        return Err(format!("{}: a path with .. cannot be checked", p.display()));
    }
    let mut rest = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(found) = cur.canonicalize() {
            return Ok(rest.iter().rev().fold(found, |acc, name| acc.join(name)));
        }
        let (Some(parent), Some(name)) = (cur.parent(), cur.file_name()) else {
            return Err(format!("{}: cannot be resolved", p.display()));
        };
        rest.push(name.to_os_string());
        cur = parent;
    }
}

/// Path and SHA-256 of every regular file under `root`, relative and
/// `/`-separated. A symlink or other special file fails closed: it could
/// point the run at something the manifest never hashed.
fn hash_tree(root: &Path) -> Result<BTreeMap<String, Sha256>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                return Err(format!("{}: not a regular file", path.display()));
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", path.display()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.insert(rel, Sha256::of(&bytes));
        }
    }
    Ok(out)
}

fn dir_of(dir: &Path, task: &TaskId) -> PathBuf {
    dir.join("profiles")
        .join(format!("{}.holdout", Sha256::of(task.0.as_bytes()).hex()))
}

/// Store `hs` as a version of `task`'s holdout suite; its digest.
pub fn stage(dir: &Path, task: &TaskId, hs: &HoldoutSet) -> Result<Sha256, String> {
    hs.validate()?;
    hs.verify_files()?;
    let bytes = hs.to_bytes()?;
    let digest = Sha256::of(&bytes);
    let d = dir_of(dir, task);
    std::fs::create_dir_all(&d).map_err(|e| format!("holdout: {e}"))?;
    rusty_atomic_file::write(&d.join(format!("{}.json", digest.hex())), &bytes)
        .map_err(|e| format!("holdout: {e}"))?;
    Ok(digest)
}

/// Load version `digest` of `task`'s holdout suite, checked against it.
pub fn load(dir: &Path, task: &TaskId, digest: Sha256) -> Result<HoldoutSet, String> {
    let p = dir_of(dir, task).join(format!("{}.json", digest.hex()));
    let bytes = std::fs::read(&p).map_err(|e| format!("holdout {}: {e}", p.display()))?;
    if Sha256::of(&bytes) != digest {
        return Err("holdout set on disk does not match its digest".into());
    }
    HoldoutSet::from_bytes(&bytes)
}

/// Where the full output of holdout run `run` is kept: host-only, never an
/// artifact, so no lens reads it (#597 2.2). Nano and the inbox may.
pub fn log_path(dir: &Path, task: &TaskId, run: RunId) -> PathBuf {
    dir.join("holdout-logs")
        .join(Sha256::of(task.0.as_bytes()).hex())
        .join(format!("run-{}.log", run.0))
}

/// The log artifact of a holdout run: profile, exit code and failure count
/// per profile, and nothing the tests printed. Pure.
pub fn summary(status: RunStatus, profiles: &[ProfileResult]) -> String {
    let mut s = format!("holdout run: {status:?} (full output is host-only)\n");
    for p in profiles {
        s.push_str(&format!(
            "{}: exit {} failed {}\n",
            p.name, p.exit_code, p.failed
        ));
    }
    s
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn suite(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bbp_holdout_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("api")).unwrap();
        std::fs::write(root.join("api/test.sh"), "true\n").unwrap();
        root
    }

    fn set() -> ProfileSet {
        ProfileSet::shell("holdout:api", "sh \"$BBP_HOLDOUT/api/test.sh\"")
    }

    #[test]
    fn a_set_needs_holdout_profiles_and_files() {
        let root = suite("shape");
        let hs = HoldoutSet::scan(set(), &root).unwrap();
        assert_eq!(hs.files.len(), 1);
        assert_eq!(hs.files[0].path, "api/test.sh");
        let err = HoldoutSet::scan(ProfileSet::shell("unit", "true"), &root).unwrap_err();
        assert!(err.contains("holdout: or golden:"), "{err}");
        let empty = suite("empty");
        std::fs::remove_dir_all(empty.join("api")).unwrap();
        assert!(HoldoutSet::scan(set(), &empty).is_err());
        let run = hs.run_set();
        assert!(run.read_roots.contains(&hs.root));
        assert!(run.env.contains(&(ROOT_ENV.to_owned(), hs.root.clone())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn changed_planted_or_removed_files_fail_verification() {
        let root = suite("verify");
        let hs = HoldoutSet::scan(set(), &root).unwrap();
        hs.verify_files().unwrap();
        std::fs::write(root.join("conftest.py"), "skip all").unwrap();
        let err = hs.verify_files().unwrap_err();
        assert!(err.contains("conftest.py"), "{err}");
        std::fs::remove_file(root.join("conftest.py")).unwrap();
        std::fs::write(root.join("api/test.sh"), "false\n").unwrap();
        assert!(hs.verify_files().unwrap_err().contains("api/test.sh"));
        std::fs::remove_file(root.join("api/test.sh")).unwrap();
        assert!(hs.verify_files().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_pure_additions_extend() {
        let root = suite("extend");
        let v1 = HoldoutSet::scan(set(), &root).unwrap();
        std::fs::write(root.join("api/more.sh"), "true\n").unwrap();
        let mut more = set();
        more.profiles.push(crate::profiles::Profile {
            name: "holdout:more".into(),
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "true".into()],
        });
        let v2 = HoldoutSet::scan(more.clone(), &root).unwrap();
        assert!(v2.extends(&v1));
        assert!(!v1.extends(&v2), "removing a file and a profile");
        std::fs::write(root.join("api/test.sh"), "exit 0\n").unwrap();
        let v3 = HoldoutSet::scan(more.clone(), &root).unwrap();
        assert!(!v3.extends(&v2), "a changed file");
        let mut looser = more;
        looser.limits.wall_secs += 1;
        let v4 = HoldoutSet::scan(looser, &root).unwrap();
        assert!(!v4.extends(&v3), "a changed limit");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn versions_are_stored_by_digest() {
        let root = suite("stage");
        let dir = std::env::temp_dir().join(format!("bbp_holdout_dir_{}", std::process::id()));
        let task = TaskId("T".into());
        let hs = HoldoutSet::scan(set(), &root).unwrap();
        let d = stage(&dir, &task, &hs).unwrap();
        assert_eq!(load(&dir, &task, d).unwrap(), hs);
        assert!(load(&dir, &task, Sha256::of(b"other")).is_err());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_root_any_build_run_can_read_is_refused() {
        let root = suite("isolated");
        let hs = HoldoutSet::scan(set(), &root).unwrap();
        let build = ProfileSet::shell("test", "true");
        hs.isolated_from(&build, &[Path::new("/nonexistent/work")])
            .unwrap();
        let parent = root.parent().unwrap().to_path_buf();
        let mut wide = build.clone();
        wide.read_roots.push(parent.to_str().unwrap().into());
        assert!(hs
            .isolated_from(&wide, &[])
            .unwrap_err()
            .contains("overlaps"));
        let inner = root.join("api").join("not-yet");
        assert!(
            hs.isolated_from(&build, &[&inner]).is_err(),
            "inside the root"
        );
        let alias = std::env::temp_dir().join(format!("bbp_holdout_alias_{}", std::process::id()));
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let mut aliased = build.clone();
        aliased
            .read_roots
            .push(alias.join("api").to_str().unwrap().into());
        assert!(hs.isolated_from(&aliased, &[]).is_err(), "a symlink alias");
        let mut dotted = build;
        dotted.read_roots.push("/usr/../tmp".into());
        assert!(hs.isolated_from(&dotted, &[]).is_err(), "..");
        let _ = std::fs::remove_file(&alias);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_summary_hides_test_output() {
        let s = summary(
            RunStatus::Failed,
            &[ProfileResult {
                name: "holdout:api".into(),
                exit_code: 1,
                failed: 1,
            }],
        );
        assert!(s.contains("holdout:api: exit 1 failed 1"), "{s}");
        assert_eq!(s.lines().count(), 2);
    }
}
