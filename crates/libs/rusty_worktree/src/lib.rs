//! Git worktrees that share one repository's object store.
//!
//! [`Repo`] adds, inspects, commits and removes worktrees of one repository
//! through the `git` CLI. Every call removes inherited `GIT_*` variables and
//! turns hooks off, and every call that acts on a worktree names its git
//! directory and work tree explicitly: a worktree's `.git` file is written by
//! whoever edits the worktree, so it is never trusted to say where the
//! repository is. A sandboxed process can therefore be given write access to
//! one worktree and read access to [`Repo::git_dir`], with staging and
//! commits left to the caller outside the sandbox.
//!
//! On Windows, `canonicalize` gives verbatim paths (`\\?\C:\…`,
//! `\\?\UNC\server\share\…`), which git does not accept as a `--git-dir`.
//! Every path handed to git is first spelled in the plain form when that
//! form names the same file, and the path is refused otherwise.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One repository whose worktrees share its object store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

/// Who a commit is by.
#[derive(Debug, Clone, Copy)]
pub struct Author<'a> {
    /// `user.name`.
    pub name: &'a str,
    /// `user.email`.
    pub email: &'a str,
}

impl Repo {
    /// The non-bare repository at `path`.
    ///
    /// # Errors
    /// When `path` cannot be resolved or has no `.git` directory.
    pub fn open(path: &Path) -> Result<Repo, String> {
        let root = path
            .canonicalize()
            .map_err(|e| format!("repo {}: {e}", path.display()))?;
        if !root.join(".git").is_dir() {
            return Err(format!("repo {}: no .git directory", root.display()));
        }
        Ok(Repo { root })
    }

    /// The repository's working directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The shared git directory: objects, refs, and each worktree's admin
    /// directory. A worktree's user needs to read it, never to write it.
    pub fn git_dir(&self) -> PathBuf {
        self.root.join(".git")
    }

    /// A worktree at `dir` on `branch`: the existing branch, or a new one
    /// from `base`. An existing worktree at `dir` is kept as it is.
    ///
    /// # Errors
    /// When git refuses.
    pub fn add_branch(&self, dir: &Path, branch: &str, base: &str) -> Result<PathBuf, String> {
        if !dir.exists() {
            let reference = format!("refs/heads/{branch}");
            let verify = [
                os("rev-parse"),
                os("--verify"),
                os("--quiet"),
                os(&reference),
            ];
            let target = git_path(dir)?;
            let tail: Vec<&OsStr> = if self.git(&verify).is_ok() {
                vec![os("worktree"), os("add"), target.as_os_str(), os(branch)]
            } else {
                vec![
                    os("worktree"),
                    os("add"),
                    os("-b"),
                    os(branch),
                    target.as_os_str(),
                    os(base),
                ]
            };
            self.git(&tail)?;
        }
        dir.canonicalize().map_err(|e| e.to_string())
    }

    /// A fresh worktree at `dir`, detached at `rev`. Anything already at
    /// `dir` is removed first.
    ///
    /// # Errors
    /// When the leftover cannot be removed or git refuses.
    pub fn add_detached(&self, dir: &Path, rev: &str) -> Result<PathBuf, String> {
        if dir.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        self.git(&[os("worktree"), os("prune")])?;
        let target = git_path(dir)?;
        self.git(&[
            os("worktree"),
            os("add"),
            os("--detach"),
            target.as_os_str(),
            os(rev),
        ])?;
        dir.canonicalize().map_err(|e| e.to_string())
    }

    /// Remove the worktree at `dir` and its admin directory, discarding
    /// whatever it holds.
    ///
    /// # Errors
    /// When git refuses.
    pub fn remove(&self, dir: &Path) -> Result<(), String> {
        let target = git_path(dir)?;
        self.git(&[
            os("worktree"),
            os("remove"),
            os("--force"),
            target.as_os_str(),
        ])
        .map(drop)
    }

    /// The admin directory git keeps for the worktree at `dir`.
    ///
    /// # Errors
    /// When `dir` is not one of this repository's worktrees.
    pub fn admin_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        let name = dir.file_name().ok_or("worktree has no name")?;
        let admin = self.git_dir().join("worktrees").join(name);
        admin
            .is_dir()
            .then_some(admin)
            .ok_or_else(|| format!("no git directory for worktree {}", dir.display()))
    }

    /// Every repository path with uncommitted changes in `dir`
    /// (`/`-separated, both sides of a rename, untracked files included).
    ///
    /// # Errors
    /// When the state cannot be read.
    pub fn changed(&self, dir: &Path) -> Result<Vec<String>, String> {
        let out = self.in_worktree(
            dir,
            &[
                os("status"),
                os("--porcelain=v1"),
                os("-z"),
                os("--untracked-files=all"),
            ],
        )?;
        Ok(parse_status(&out))
    }

    /// Throw away everything uncommitted in `dir`.
    ///
    /// # Errors
    /// When git refuses.
    pub fn discard(&self, dir: &Path) -> Result<(), String> {
        self.in_worktree(dir, &[os("reset"), os("--hard"), os("-q")])?;
        self.in_worktree(dir, &[os("clean"), os("-fdq")])?;
        Ok(())
    }

    /// Commit everything in `dir` as `author`, unsigned and without hooks.
    /// `Ok(false)` when there was nothing to commit.
    ///
    /// # Errors
    /// When git refuses.
    pub fn commit(&self, dir: &Path, message: &str, author: Author<'_>) -> Result<bool, String> {
        self.in_worktree(dir, &[os("add"), os("-A")])?;
        if self
            .in_worktree(dir, &[os("diff"), os("--cached"), os("--quiet")])
            .is_ok()
        {
            return Ok(false);
        }
        let name = format!("user.name={}", author.name);
        let email = format!("user.email={}", author.email);
        self.in_worktree(
            dir,
            &[
                os("-c"),
                os(&name),
                os("-c"),
                os(&email),
                os("-c"),
                os("commit.gpgsign=false"),
                os("commit"),
                os("--no-verify"),
                os("-q"),
                os("-m"),
                os(message),
            ],
        )?;
        Ok(true)
    }

    /// git in the main repository.
    fn git(&self, tail: &[&OsStr]) -> Result<String, String> {
        let root = git_path(&self.root)?;
        run(&[&[os("-C"), root.as_os_str()], tail].concat())
    }

    /// git on the worktree at `dir`, through its admin directory.
    fn in_worktree(&self, dir: &Path, tail: &[&OsStr]) -> Result<String, String> {
        let admin = git_path(&self.admin_dir(dir)?)?;
        let work = git_path(dir)?;
        let base = [
            os("--git-dir"),
            admin.as_os_str(),
            os("--work-tree"),
            work.as_os_str(),
        ];
        run(&[&base[..], tail].concat())
    }
}

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

/// `path` spelled the way git accepts it. Anything but a Windows verbatim
/// path is returned as it is. A verbatim disk or UNC path becomes its plain
/// form (`C:\…`, `\\server\share\…`) when every component means the same
/// there; any other verbatim path is refused rather than altered.
///
/// # Errors
/// When `path` is verbatim and has no plain form naming the same file.
fn git_path(path: &Path) -> Result<PathBuf, String> {
    plain_form(path).ok_or_else(|| {
        format!(
            "{}: git cannot take this path, and no plain form names the same file",
            path.display()
        )
    })
}

#[cfg(not(windows))]
fn plain_form(path: &Path) -> Option<PathBuf> {
    Some(path.to_path_buf())
}

#[cfg(windows)]
fn plain_form(path: &Path) -> Option<PathBuf> {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Some(path.to_path_buf());
    };
    let mut plain = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut unc = std::ffi::OsString::from(r"\\");
            unc.push(server);
            unc.push(r"\");
            unc.push(share);
            unc.push(r"\");
            PathBuf::from(unc)
        }
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return None,
        Prefix::Disk(_) | Prefix::UNC(..) => return Some(path.to_path_buf()),
    };
    for part in parts {
        match part {
            Component::RootDir => {}
            Component::Normal(name) if name.to_str().is_some_and(plain_name) => plain.push(name),
            _ => return None,
        }
    }
    Some(plain)
}

/// Whether a verbatim path component means the same in a plain Windows
/// path: no character the plain form treats specially, no trailing dot or
/// space (stripped there), and not a DOS device name (which the plain form
/// opens instead of the file).
#[cfg_attr(not(windows), allow(dead_code))]
fn plain_name(name: &str) -> bool {
    const DEVICES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if matches!(name, "" | "." | "..") || name.ends_with(['.', ' ']) {
        return false;
    }
    if name
        .chars()
        .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    let numbered = ["COM", "LPT"].iter().any(|port| {
        stem.strip_prefix(port).is_some_and(|n| {
            matches!(
                n,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    !numbered && !DEVICES.contains(&stem.as_str())
}

/// Run git; stdout on success, stderr on failure. No inherited `GIT_*`
/// variable can redirect it, and no hook runs.
fn run(args: &[&OsStr]) -> Result<String, String> {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    let out = command
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let shown: Vec<_> = args.iter().map(|a| a.to_string_lossy()).collect();
    Err(format!(
        "git {}: {}",
        shown.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// The paths in `git status --porcelain=v1 -z` output: `XY path\0`, and for
/// a rename or copy the original path follows as its own field. Both sides
/// count.
pub fn parse_status(out: &str) -> Vec<String> {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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

    #[test]
    fn plain_names_exclude_what_a_plain_windows_path_rewrites() {
        for ok in [
            "repo", ".git", "a b", "con-fig", "COM10", "nul_", "x.y.z", "é",
        ] {
            assert!(plain_name(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", "dot.", "space ", "a:b", "a/b", "a*", "tab\t", "CON", "nul.txt",
            "Aux .rs", "com1", "LPT9.log", "COM¹", "CONOUT$",
        ] {
            assert!(!plain_name(bad), "{bad}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn paths_pass_through_off_windows() {
        let path = Path::new("/tmp/a b/.git/worktrees/x");
        assert_eq!(git_path(path).unwrap(), path);
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_paths_take_their_plain_form_or_are_refused() {
        let cases = [
            (r"\\?\C:\Users\a\repo\.git", Some(r"C:\Users\a\repo\.git")),
            (r"\\?\UNC\srv\share\r\.git", Some(r"\\srv\share\r\.git")),
            (r"C:\plain\path", Some(r"C:\plain\path")),
            (r"\\srv\share\plain", Some(r"\\srv\share\plain")),
            (r"relative\dir", Some(r"relative\dir")),
            (r"\\?\C:\a\trailing.", None),
            (r"\\?\C:\a\nul\b", None),
            (r"\\?\Volume{0}\x", None),
            (r"\\.\COM1", None),
        ];
        for (input, want) in cases {
            let got = git_path(Path::new(input)).ok();
            assert_eq!(got.as_deref(), want.map(Path::new), "{input}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_canonical_path_reaches_git_in_a_form_it_reads() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let plain = git_path(&dir).unwrap();
        assert!(
            !plain.as_os_str().to_string_lossy().starts_with(r"\\?\"),
            "{plain:?}"
        );
        assert_eq!(plain.canonicalize().unwrap(), dir);
    }
}
