//! The filtered view (#598 gap 2): what a cloud harness sees of the tree.
//!
//! A launcher's sandbox may name `hide` globs (the factory's
//! `sensitive_paths`). Its turn workspace is then not a worktree of the
//! repository, whose shared object store would hold every hidden blob, but
//! a plain copy of the commit with every hidden path left out, names
//! included, committed once into a fresh repository of its own, so the
//! harness can still diff its edits and its `.git` holds only what it shows.
//! A path is hidden when it or any directory above it matches a glob.
//!
//! The same globs bound what the harness may post: a diff that touches a
//! hidden path is refused before it is stored ([`refused`]). The copy's
//! commit exists only in the copy, so a candidate based on it is pinned
//! back to the source commit it was made from ([`View`]).

use crate::profiles::path_matches;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Whether `path` or a directory above it matches one of `hide`. Pure.
pub fn hidden(hide: &[String], path: &str) -> bool {
    let segs: Vec<&str> = path.split('/').collect();
    (1..=segs.len()).any(|n| {
        let prefix = segs[..n].join("/");
        hide.iter().any(|g| path_matches(g, &prefix))
    })
}

/// The hidden paths among `paths`, sorted and deduplicated. Pure.
pub fn refused(hide: &[String], paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = paths.iter().filter(|p| hidden(hide, p)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// One file of the commit to copy.
struct Entry {
    mode: String,
    sha: String,
    path: String,
}

/// A filtered copy's two commits: the source commit it was made from, and
/// the copy's own, which exists nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The full id of the source commit.
    pub base: String,
    /// The full id of the copy's commit.
    pub head: String,
}

impl View {
    /// The source commit a candidate on `base` stands on: the copy's commit
    /// is pinned to [`View::base`], and the source commit names itself.
    /// Any other base is none the copy could have made. Pure.
    pub fn pin(&self, base: &str) -> Option<&str> {
        (base == self.head || base == self.base).then_some(self.base.as_str())
    }
}

/// Copy commit `rev` of `repo` into the empty or absent `dest`, leaving out
/// every path [`hidden`] by `hide`, and commit the copy into a fresh
/// repository at `dest`. Submodules are left out; a path git would refuse
/// to check out fails closed.
pub fn copy(repo: &Path, rev: &str, dest: &Path, hide: &[String]) -> Result<View, String> {
    let base = commit_id(repo, &format!("{rev}^{{commit}}"))?;
    let entries: Vec<Entry> = tree(repo, &base)?
        .into_iter()
        .filter(|e| e.mode != "160000" && !hidden(hide, &e.path))
        .collect();
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let blobs = cat_blobs(repo, &entries)?;
    for (e, bytes) in entries.iter().zip(blobs) {
        write_entry(dest, e, &bytes)?;
    }
    commit(dest)?;
    let head = commit_id(dest, "HEAD")?;
    Ok(View { base, head })
}

/// The full commit id `rev` names in `repo`.
fn commit_id(repo: &Path, rev: &str) -> Result<String, String> {
    let out = git(repo)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options", rev])
        .output()
        .map_err(|e| format!("git rev-parse: {e}"))?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || id.is_empty() {
        return Err(format!("git rev-parse {rev}: no such commit"));
    }
    Ok(id)
}

/// Every blob of `rev`'s tree, from `git ls-tree -r -z`.
fn tree(repo: &Path, rev: &str) -> Result<Vec<Entry>, String> {
    let out = git(repo)
        .args([
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            "--end-of-options",
            rev,
        ])
        .output()
        .map_err(|e| format!("git ls-tree: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-tree {rev}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    out.stdout
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(|raw| {
            let line = std::str::from_utf8(raw).map_err(|_| "a path is not UTF-8".to_owned())?;
            let (meta, path) = line.split_once('\t').ok_or("ls-tree: no path")?;
            let mut parts = meta.split(' ');
            let (Some(mode), Some(_kind), Some(sha)) = (parts.next(), parts.next(), parts.next())
            else {
                return Err(format!("ls-tree: bad entry {meta:?}"));
            };
            safe(path)?;
            Ok(Entry {
                mode: mode.into(),
                sha: sha.into(),
                path: path.into(),
            })
        })
        .collect()
}

/// Refuse a path that could leave `dest` or write a repository's metadata.
fn safe(path: &str) -> Result<(), String> {
    let bad = path.starts_with('/')
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == ".." || s.eq_ignore_ascii_case(".git"));
    if bad {
        return Err(format!("{path:?}: not a path to copy"));
    }
    Ok(())
}

/// The contents of each entry's blob, in order, from one `git cat-file
/// --batch`.
fn cat_blobs(repo: &Path, entries: &[Entry]) -> Result<Vec<Vec<u8>>, String> {
    let mut child = git(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git cat-file: {e}"))?;
    let mut input = child.stdin.take().ok_or("git cat-file: no stdin")?;
    let shas: String = entries.iter().map(|e| format!("{}\n", e.sha)).collect();
    let feed = std::thread::spawn(move || input.write_all(shas.as_bytes()));
    let mut out = BufReader::new(child.stdout.take().ok_or("git cat-file: no stdout")?);
    let mut blobs = Vec::with_capacity(entries.len());
    for e in entries {
        let mut header = String::new();
        out.read_line(&mut header)
            .map_err(|err| format!("git cat-file: {err}"))?;
        let size = match header.trim_end().split(' ').collect::<Vec<_>>()[..] {
            [sha, "blob", size] if sha == e.sha => size
                .parse::<usize>()
                .map_err(|_| format!("git cat-file: bad size for {}", e.path))?,
            _ => return Err(format!("git cat-file: {}: {}", e.path, header.trim())),
        };
        let mut bytes = vec![0; size + 1];
        out.read_exact(&mut bytes)
            .map_err(|err| format!("git cat-file: {err}"))?;
        bytes.pop();
        blobs.push(bytes);
    }
    drop(out);
    feed.join()
        .map_err(|_| "git cat-file: feeder panicked".to_owned())?
        .map_err(|e| format!("git cat-file: {e}"))?;
    let status = child.wait().map_err(|e| format!("git cat-file: {e}"))?;
    if !status.success() {
        return Err(format!("git cat-file: {status}"));
    }
    Ok(blobs)
}

fn write_entry(dest: &Path, e: &Entry, bytes: &[u8]) -> Result<(), String> {
    let target = dest.join(&e.path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    #[cfg(unix)]
    if e.mode == "120000" {
        use std::os::unix::ffi::OsStrExt;
        let link = std::ffi::OsStr::from_bytes(bytes);
        return std::os::unix::fs::symlink(link, &target)
            .map_err(|err| format!("{}: {err}", target.display()));
    }
    std::fs::write(&target, bytes).map_err(|err| format!("{}: {err}", target.display()))?;
    #[cfg(unix)]
    if e.mode == "100755" {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("{}: {err}", target.display()))?;
    }
    Ok(())
}

/// `git init` at `dest` and commit everything there, ignored files too.
fn commit(dest: &Path) -> Result<(), String> {
    let steps: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "-A", "-f"],
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "--no-verify",
            "-m",
            "filtered view",
        ],
    ];
    for args in steps {
        let out = git(dest)
            .args(args)
            .env("GIT_AUTHOR_NAME", "bbp")
            .env("GIT_AUTHOR_EMAIL", "bbp@localhost")
            .env("GIT_COMMITTER_NAME", "bbp")
            .env("GIT_COMMITTER_EMAIL", "bbp@localhost")
            .output()
            .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
        if !out.status.success() {
            return Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// `git -C dir` with no global or system config, so the copy is the same on
/// every host.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    c
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> std::process::Output {
        let out = git(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        out
    }

    #[test]
    fn a_path_is_hidden_by_its_own_glob_or_a_directory_above_it() {
        let hide = vec!["secrets".to_owned(), "**/*.key".to_owned()];
        assert!(hidden(&hide, "secrets"));
        assert!(hidden(&hide, "secrets/a/b.txt"));
        assert!(hidden(&hide, "src/deep/id.key"));
        assert!(!hidden(&hide, "src/secrets.rs"));
        assert_eq!(
            refused(&hide, &["b.key".into(), "src/x.rs".into(), "b.key".into()]),
            ["b.key"]
        );
    }

    #[test]
    fn the_copy_leaves_hidden_paths_out_of_its_files_and_its_git() {
        let root = std::env::temp_dir().join(format!("bbp_filtered_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (repo, view) = (root.join("repo"), root.join("view"));
        std::fs::create_dir_all(repo.join("secrets")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("secrets/token.txt"), "hunter2\n").unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        std::fs::write(repo.join("run.sh"), "#!/bin/sh\n").unwrap();
        run(&repo, &["init", "-q"]);
        run(&repo, &["add", "-A"]);
        run(&repo, &["update-index", "--chmod=+x", "run.sh"]);
        run(&repo, &["commit", "-q", "-m", "base"]);
        let secret =
            String::from_utf8(run(&repo, &["rev-parse", "HEAD:secrets/token.txt"]).stdout).unwrap();

        let pinned = copy(&repo, "HEAD", &view, &["secrets".into()]).unwrap();
        let head = |d: &Path| String::from_utf8(run(d, &["rev-parse", "HEAD"]).stdout).unwrap();
        assert_eq!(pinned.base, head(&repo).trim());
        assert_eq!(pinned.head, head(&view).trim());
        assert_ne!(pinned.base, pinned.head);
        assert_eq!(pinned.pin(&pinned.head), Some(pinned.base.as_str()));
        assert_eq!(pinned.pin(&pinned.base), Some(pinned.base.as_str()));
        assert_eq!(pinned.pin("0123abcd"), None, "no other base is the copy's");

        assert!(
            !view.join("secrets").exists(),
            "the directory name is hidden too"
        );
        assert_eq!(
            std::fs::read_to_string(view.join("src/lib.rs")).unwrap(),
            "pub fn f() {}\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(view.join("run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        let leak = git(&view)
            .args(["cat-file", "-e", secret.trim()])
            .output()
            .unwrap();
        assert!(
            !leak.status.success(),
            "the view's git must not hold the hidden blob"
        );
        let files = String::from_utf8(run(&view, &["ls-files"]).stdout).unwrap();
        assert_eq!(files, "run.sh\nsrc/lib.rs\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_that_could_escape_is_refused() {
        for p in ["../x", "a//b", ".git/config", "a/.GIT/x", "/etc/passwd"] {
            assert!(safe(p).is_err(), "{p}");
        }
        assert!(safe("src/.gitignore").is_ok());
    }
}
