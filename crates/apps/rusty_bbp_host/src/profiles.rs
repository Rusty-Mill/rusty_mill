//! The profile set a task freezes when it opens: which commands the runner
//! executes, where they may read, their environment and limits. Stored at
//! `<dir>/profiles/<task>.json`; its SHA-256 is the task's `profile_digest`,
//! so a report whose digest differs is refused by the core.

use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitSpec {
    pub cpu_secs: u64,
    pub wall_secs: u64,
    pub memory_bytes: u64,
    pub file_bytes: u64,
    pub open_files: u64,
    pub processes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSet {
    pub profiles: Vec<Profile>,
    /// Absolute directories the workload may read and execute from, beyond
    /// the checkout (toolchains, system libraries).
    pub read_roots: Vec<String>,
    /// The workload's complete environment.
    pub env: Vec<(String, String)>,
    pub limits: LimitSpec,
    /// How `Report.tree` is computed; informative.
    pub tree: String,
    /// Claims mode: globs naming every path the runners read tests, fixtures
    /// or test configuration from. A candidate run lists each matching file
    /// its diffs add, modify or delete in `Report.tests_changed`. Required to
    /// open a claims task; never written when empty, so stage-mode profile
    /// digests are unchanged.
    #[rusty_serde(default, skip_serializing_if = "no_paths")]
    pub test_paths: Vec<String>,
    /// Claims mode (#597): globs no candidate diff may add, modify or delete,
    /// except through an accepted `tests` claim. Covered by the digest; never
    /// written when empty.
    #[rusty_serde(default, skip_serializing_if = "no_paths")]
    pub protected: Vec<String>,
    /// Claims mode with `protected`: the full commit id every candidate
    /// builds on. The diffs are all a candidate changes, so pinning the base
    /// keeps an older revision with weaker tests out (#597 G2). Required
    /// with `protected`; never written when absent.
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    /// Claims mode (#704, D41, D82): `"off"` records that the task opts out
    /// of the erosion gate. A claims set needs this or one profile named
    /// [`EROSION_PROFILE`], not both. Covered by the digest; never written
    /// when absent, so existing digests are unchanged.
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    pub erosion: Option<String>,
}

/// The profile name the claims open looks for (D82): a name, not a program
/// path, so a wrapper such as `sh -c` still counts.
pub const EROSION_PROFILE: &str = "erosion";

impl ProfileSet {
    /// A profile set that runs one shell command, for tests and first runs.
    pub fn shell(name: &str, command: &str) -> ProfileSet {
        ProfileSet {
            profiles: vec![Profile {
                name: name.into(),
                program: "/bin/sh".into(),
                args: vec!["-c".into(), command.into()],
            }],
            read_roots: vec!["/bin".into(), "/usr".into(), "/lib".into(), "/lib64".into()],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            // RLIMIT_NPROC counts every process of the uid, not just the
            // workload's: CI's runner user cannot fork at 64 (rsi learned the
            // same). A fork-bomb brake, not a quota; the wall-clock kill of the
            // process group is what bounds a run.
            limits: LimitSpec {
                cpu_secs: 60,
                wall_secs: 120,
                memory_bytes: 1 << 30,
                file_bytes: 1 << 28,
                open_files: 256,
                processes: 4096,
            },
            tree: "sha256(git write-tree id)".into(),
            test_paths: vec![],
            protected: vec![],
            baseline: None,
            erosion: None,
        }
    }

    /// Linux Rust test profile for a toolchain using the LLD linker.
    /// Limits linker parallelism explicitly before the task freezes this profile.
    /// `sysroot` is `rustc --print sysroot`, never a credential-bearing home root.
    pub fn rust_lld_tests(sysroot: &Path) -> Result<ProfileSet, String> {
        let sysroot = sysroot.canonicalize().map_err(|e| e.to_string())?;
        let compiler = sysroot.join("bin/rustc");
        if !compiler.is_file() {
            return Err("Rust sysroot must contain bin/rustc".into());
        }
        let compiler = compiler.to_str().ok_or("non-UTF8 Rust compiler path")?;
        let mut set = Self::shell("unused", "unused");
        set.profiles = vec![
            Profile {
                name: "compile-tests-lld".into(),
                program: compiler.into(),
                args: [
                    "--test",
                    "src/lib.rs",
                    "-C",
                    "link-arg=-Wl,--threads=1",
                    "-o",
                    ".bbp-tests",
                ]
                .map(str::to_owned)
                .to_vec(),
            },
            Profile {
                name: "test".into(),
                program: "./.bbp-tests".into(),
                args: vec![],
            },
        ];
        set.read_roots
            .push(sysroot.to_str().ok_or("non-UTF8 Rust sysroot")?.into());
        set.env = vec![(
            "PATH".into(),
            format!("{}:/usr/bin:/bin", sysroot.join("bin").display()),
        )];
        set.limits.cpu_secs = 10;
        set.limits.wall_secs = 30;
        Ok(set)
    }

    /// The protected-path configuration is enforceable: every glob in the
    /// grammar [`path_matches`] implements, and a full-commit `baseline`
    /// whenever any is set. A pattern it would read literally (`[ab]`, a
    /// leading `/` or `./`) would silently protect nothing, so it fails here.
    pub fn check_protected(&self) -> Result<(), String> {
        if self.protected.is_empty() {
            return Ok(());
        }
        for g in &self.protected {
            check_glob(g).map_err(|e| format!("protected glob {g:?}: {e}"))?;
        }
        match self.baseline.as_deref() {
            Some(b) if is_commit_id(b) => Ok(()),
            Some(b) => Err(format!("baseline {b:?} is not a full commit id")),
            None => Err("protected paths need a baseline commit".into()),
        }
    }

    /// A claims set has exactly one of an `erosion` profile or the recorded
    /// `erosion: "off"` opt-out (D41). The profile's `--base` is a full
    /// commit id, and the pinned `baseline` when one is set (D82).
    pub fn check_erosion(&self) -> Result<(), String> {
        let named: Vec<&Profile> = self
            .profiles
            .iter()
            .filter(|p| p.name == EROSION_PROFILE)
            .collect();
        match (named.as_slice(), self.erosion.as_deref()) {
            ([], Some("off")) => Ok(()),
            (_, Some("off")) => Err("both an erosion profile and erosion \"off\"".into()),
            (_, Some(v)) => Err(format!("{v:?}: the only opt-out is \"off\"")),
            ([], None) => Err("a claims task needs an erosion profile or erosion \"off\"".into()),
            ([p], None) => self.check_erosion_base(p),
            (_, None) => Err("more than one erosion profile".into()),
        }
    }

    /// The `--base` value in the profile's arguments, read as words so a
    /// `sh -c` command line counts too.
    fn check_erosion_base(&self, p: &Profile) -> Result<(), String> {
        let words: Vec<&str> = p.args.iter().flat_map(|a| a.split_whitespace()).collect();
        let bases: Vec<&str> = words
            .windows(2)
            .filter(|w| w[0] == "--base")
            .map(|w| w[1])
            .collect();
        // One base only: `erosion` refuses a repeated flag, and the open must
        // validate the base the run will use.
        let base = match bases[..] {
            [base] => base,
            [] => return Err("the erosion profile names no --base commit".into()),
            _ => return Err("the erosion profile names --base more than once".into()),
        };
        if !is_commit_id(base) {
            return Err(format!("--base {base:?} is not a full commit id"));
        }
        match self.baseline.as_deref() {
            Some(b) if b != base => Err(format!("--base {base} is not the pinned baseline {b}")),
            _ => Ok(()),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        rusty_serde::json::to_string(self)
            .map(String::into_bytes)
            .map_err(|e| e.to_string())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<ProfileSet, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        rusty_serde::json::from_str(text).map_err(|e| e.to_string())
    }
}

/// `<dir>/profiles/<sha256(task id)>.json`: injective, so no two tasks share a file.
pub fn path(dir: &Path, task: &TaskId) -> PathBuf {
    dir.join("profiles")
        .join(format!("{}.json", Sha256::of(task.0.as_bytes()).hex()))
}

/// Read a profile set from a JSON file.
pub fn read_file(file: &Path) -> Result<ProfileSet, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    ProfileSet::from_bytes(&bytes)
}

/// Write the task's profile set and return its digest.
pub fn freeze(dir: &Path, task: &TaskId, set: &ProfileSet) -> Result<Sha256, String> {
    if set.profiles.is_empty() {
        return Err("profile set has no profiles".into());
    }
    let bytes = set.to_bytes()?;
    let p = path(dir, task);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    rusty_atomic_file::write(&p, &bytes).map_err(|e| format!("profiles: {e}"))?;
    Ok(Sha256::of(&bytes))
}

/// Load the task's profile set and check it against the digest the task froze.
pub fn load(dir: &Path, task: &TaskId, expected: Sha256) -> Result<ProfileSet, String> {
    let bytes = std::fs::read(path(dir, task)).map_err(|e| format!("profiles: {e}"))?;
    if Sha256::of(&bytes) != expected {
        return Err("profile set on disk does not match the task's frozen digest".into());
    }
    ProfileSet::from_bytes(&bytes)
}

/// Whether `path` (relative, `/`-separated) matches `glob`. `**` as a whole
/// segment matches any number of segments, `*` any run of characters within
/// one segment, `?` one character; everything else is literal.
pub fn path_matches(glob: &str, path: &str) -> bool {
    let pat: Vec<&str> = glob.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    segments_match(&pat, &segs)
}

fn segments_match(pat: &[&str], segs: &[&str]) -> bool {
    match pat.split_first() {
        None => segs.is_empty(),
        Some((&"**", rest)) => (0..=segs.len()).any(|i| segments_match(rest, &segs[i..])),
        Some((p, rest)) => segs.split_first().is_some_and(|(s, more)| {
            let p: Vec<char> = p.chars().collect();
            let s: Vec<char> = s.chars().collect();
            segment_matches(&p, &s) && segments_match(rest, more)
        }),
    }
}

/// One segment, by character: `?` is one character, never one byte.
fn segment_matches(p: &[char], s: &[char]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some(('*', rest)) => (0..=s.len()).any(|i| segment_matches(rest, &s[i..])),
        Some(('?', rest)) => !s.is_empty() && segment_matches(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && segment_matches(rest, &s[1..]),
    }
}

/// `glob` is in the grammar of [`path_matches`]: relative `/`-separated
/// segments, none empty, `.` or `..`, with `**` only as a whole segment and
/// no syntax it does not implement (`[`, `]`, `{`, `}`, `\`, `!`). A
/// control character or a Windows drive prefix (`C:`) is refused too: such
/// a pattern looks absolute or unprintable and never matches a repository
/// path, so it would protect nothing. `!` is refused, not read literally.
pub fn check_glob(glob: &str) -> Result<(), String> {
    if glob.is_empty() {
        return Err("empty".into());
    }
    if glob.chars().any(char::is_control) {
        return Err("a control character".into());
    }
    if let [d, b':', ..] = glob.as_bytes() {
        if d.is_ascii_alphabetic() {
            return Err("a drive prefix makes it absolute".into());
        }
    }
    if let Some(c) = glob.chars().find(|c| "[]{}\\!".contains(*c)) {
        return Err(format!("{c:?} is not supported; only *, ** and ? are"));
    }
    for seg in glob.split('/') {
        match seg {
            "" => return Err("absolute, or an empty segment".into()),
            "." | ".." => return Err(format!("{seg:?} segment")),
            "**" => {}
            s if s.contains("**") => return Err("** must be a whole segment".into()),
            _ => {}
        }
    }
    Ok(())
}

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
pub fn is_commit_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `path` matches any of `globs`.
pub fn is_test_path(globs: &[String], path: &str) -> bool {
    globs.iter().any(|g| path_matches(g, path))
}

#[allow(clippy::ptr_arg)]
fn no_paths(v: &Vec<String>) -> bool {
    v.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_whole_paths_by_segment() {
        assert!(path_matches("tests/**", "tests/core.rs"));
        assert!(path_matches("tests/**", "tests/a/b/conftest.py"));
        assert!(!path_matches("tests/**", "src/tests.rs"));
        assert!(path_matches("**/conftest.py", "conftest.py"));
        assert!(path_matches("**/conftest.py", "pkg/conftest.py"));
        assert!(path_matches("src/*_test.go", "src/a_test.go"));
        assert!(!path_matches("src/*_test.go", "src/x/a_test.go"));
        assert!(path_matches("pytest.ini", "pytest.ini"));
        assert!(!path_matches("pytest.ini", "a/pytest.ini"));
        assert!(path_matches("t?st/*", "test/x"));
    }

    #[test]
    fn protected_globs_outside_the_grammar_are_refused() {
        for ok in [
            "tests/**",
            "**/conftest.py",
            "src/*_test.go",
            "t?st/*",
            "pytest.ini",
        ] {
            assert_eq!(check_glob(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "/tests/**",
            "./tests/**",
            "tests/../src",
            "tests//a",
            "tests/",
            "tests/[ab].rs",
            "tests/{a,b}.rs",
            "!tests/a.rs",
            "tests\\a.rs",
            "tests/a**",
            "C:/tests/**",
            "c:tests/**",
            "tests/\u{7f}a.rs",
            "tests/a\n.rs",
            "tests/\ta.rs",
        ] {
            assert!(check_glob(bad).is_err(), "{bad:?}");
        }
        let mut set = ProfileSet::shell("test", "true");
        assert_eq!(set.check_protected(), Ok(()));
        set.protected = vec!["tests/**".into()];
        assert!(set.check_protected().is_err_and(|e| e.contains("baseline")));
        set.baseline = Some("main".into());
        assert!(set.check_protected().is_err());
        set.baseline = Some("a".repeat(40));
        assert_eq!(set.check_protected(), Ok(()));
        set.protected.push("tests/[ab].rs".into());
        assert!(set.check_protected().is_err_and(|e| e.contains("[ab]")));
    }

    #[test]
    fn a_question_mark_is_one_character_not_one_byte() {
        assert!(path_matches("tests/?.rs", "tests/a.rs"));
        assert!(path_matches("tests/?.rs", "tests/é.rs"));
        assert!(path_matches("tests/?.rs", "tests/测.rs"));
        assert!(!path_matches("tests/?.rs", "tests/éé.rs"));
        assert!(path_matches("tests/*é.rs", "tests/abé.rs"));
    }

    #[test]
    fn an_empty_test_path_list_leaves_the_stage_digest_alone() {
        let set = ProfileSet::shell("test", "true");
        let text = String::from_utf8(set.to_bytes().unwrap_or_default()).unwrap_or_default();
        assert!(!text.contains("test_paths"), "{text}");
        let mut claims = set.clone();
        claims.test_paths = vec!["tests/**".into()];
        let back = ProfileSet::from_bytes(&claims.to_bytes().unwrap_or_default());
        assert_eq!(back, Ok(claims));
    }

    fn erosion_set(args: &str) -> ProfileSet {
        let mut set = ProfileSet::shell("test", "true");
        set.profiles.push(Profile {
            name: EROSION_PROFILE.into(),
            program: "/usr/local/bin/erosion".into(),
            args: args.split_whitespace().map(String::from).collect(),
        });
        set
    }

    #[test]
    fn a_claims_set_needs_exactly_one_erosion_profile_or_the_opt_out() {
        let c = "0123456789abcdef0123456789abcdef01234567";
        let ok = erosion_set(&format!("check --base {c} --report-only"));
        assert_eq!(ok.check_erosion(), Ok(()));
        let mut off = ProfileSet::shell("test", "true");
        off.erosion = Some("off".into());
        assert_eq!(off.check_erosion(), Ok(()));
        let mut shell = ProfileSet::shell(
            EROSION_PROFILE,
            &format!("erosion check --base {c} --report-only"),
        );
        assert_eq!(shell.check_erosion(), Ok(()));
        shell.baseline = Some(c.into());
        assert_eq!(shell.check_erosion(), Ok(()));

        let mut both = ok.clone();
        both.erosion = Some("off".into());
        let mut other = off.clone();
        other.erosion = Some("no".into());
        let mut twice = ok.clone();
        twice.profiles.push(twice.profiles[1].clone());
        let mut pinned = ok.clone();
        pinned.baseline = Some("f".repeat(40));
        for (bad, why) in [
            (ProfileSet::shell("test", "true"), "needs"),
            (both, "both"),
            (other, "only opt-out"),
            (twice, "more than one"),
            (erosion_set("check --report-only"), "no --base"),
            (
                erosion_set("check --base HEAD --report-only"),
                "full commit",
            ),
            (pinned, "pinned baseline"),
            (
                erosion_set(&format!(
                    "check --base {c} --base {} --report-only",
                    "f".repeat(40)
                )),
                "more than once",
            ),
        ] {
            assert!(bad.check_erosion().is_err_and(|e| e.contains(why)), "{why}");
        }
    }

    #[test]
    fn an_absent_opt_out_leaves_the_digest_alone() {
        let set = ProfileSet::shell("test", "true");
        let text = String::from_utf8(set.to_bytes().unwrap_or_default()).unwrap_or_default();
        assert!(!text.contains("erosion"), "{text}");
        let mut off = set.clone();
        off.erosion = Some("off".into());
        let back = ProfileSet::from_bytes(&off.to_bytes().unwrap_or_default());
        assert_eq!(back, Ok(off));
    }
}
