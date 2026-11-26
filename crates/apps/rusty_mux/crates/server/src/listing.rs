//! `terminal.list-dir` (C08): one directory on the server's host, read for a
//! client's directory browse.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::process::escape;

/// Entries listed at most; the rest are counted in the `+` line.
const LISTED_MAX: usize = 1000;
/// Entries read at most, so one huge directory cannot hold the server.
const READ_MAX: usize = 20_000;

/// What an entry is, as the one-letter kind a line starts with; `None` when
/// the file system will not say.
///
/// With `follow` a link is told by what it points to; without it a link is
/// `l` and its target is never inspected (the workspace listing, whose
/// targets may lie outside the workspace).
fn kind(path: &Path, entry: &fs::DirEntry, follow: bool) -> Option<char> {
    let link = entry.file_type().ok()?;
    if !link.is_symlink() {
        return Some(match (link.is_dir(), link.is_file()) {
            (true, _) => 'd',
            (_, true) => 'f',
            _ => 'o',
        });
    }
    if !follow {
        return Some('l');
    }
    // A link is told by what it points to; a broken one is `l`.
    Some(match fs::metadata(path) {
        Ok(m) if m.is_dir() => 'd',
        _ => 'l',
    })
}

/// Whether `path` names another machine (a UNC path), which opening would
/// make the server contact.
pub(crate) fn is_network_path(path: &str) -> bool {
    path.starts_with("\\\\") || (cfg!(windows) && path.starts_with("//"))
}

/// What lists a directory: [`list`], or in tests a stand-in that stalls.
pub(crate) type Lister = Arc<
    dyn Fn(Option<&str>, Option<&str>, Option<&Workspace>) -> Result<String, String> + Send + Sync,
>;

/// The real [`Lister`].
pub(crate) fn lister() -> Lister {
    Arc::new(list)
}

/// How long one listing may take before it is refused.
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);
/// Listings that may be running (or hung on a dead mount) at once.
const MAX_RUNNING: usize = 8;

/// Counts listings from the moment they start to the moment their thread
/// ends, which is later than their deadline when one hangs.
#[derive(Clone, Default)]
pub(crate) struct Gate(Arc<AtomicUsize>);

/// One listing's place under [`Gate::take`]; frees it on drop.
pub(crate) struct Slot(Arc<AtomicUsize>);

impl Gate {
    pub(crate) fn take(&self) -> Result<Slot, String> {
        let taken = self.0.fetch_add(1, Ordering::AcqRel);
        if taken >= MAX_RUNNING {
            self.0.fetch_sub(1, Ordering::AcqRel);
            return Err("too many file reads or listings are still running".into());
        }
        Ok(Slot(self.0.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The listing of `path`, taken relative to `base` (the block's working
/// directory) when it is not absolute, and `base` itself when `None`.
/// With a `root` (the workspace, D337) the listing is confined to it: see
/// [`confined`]. A link inside it is shown as `l` and never followed, the
/// first line is the path under the workspace (`.` for itself), and a
/// refusal names nothing outside it.
///
/// The first line is the directory's resolved path. Every other line is
/// `KIND<TAB>NAME`: `d` for a directory (a link to one too), `f` a file,
/// `l` another link, `o` anything else; directories first, then by name.
/// Then, each only when it applies: `+<TAB>N`, exactly N entries read were
/// left out; `><TAB>N`, the directory has more entries than are read, so at
/// least N were left out; `!<TAB>N`, N entries could not be read or have a name that is not UTF-8. Names
/// are [`escape`]d, so none adds a field or a line.
pub(crate) fn list(
    base: Option<&str>,
    path: Option<&str>,
    root: Option<&Workspace>,
) -> Result<String, String> {
    list_with(base, path, root, READ_MAX, LISTED_MAX)
}

fn list_with(
    base: Option<&str>,
    path: Option<&str>,
    root: Option<&Workspace>,
    read_max: usize,
    listed_max: usize,
) -> Result<String, String> {
    if path.is_some_and(is_network_path) {
        return Err("a network path is not listed".into());
    }
    if let Some(root) = root {
        let held = confined(root, path.unwrap_or(""))?;
        return render(&held.read, &held.shown, false, read_max, listed_max);
    }
    let dir = match (path, base) {
        (Some(p), Some(b)) => Path::new(b).join(p),
        (Some(p), None) if Path::new(p).is_absolute() => p.into(),
        (Some(_), None) | (None, None) => {
            return Err("the program's working directory is unknown".into());
        }
        (None, Some(b)) => b.into(),
    };
    let dir = fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    render(&dir, &dir.to_string_lossy(), true, read_max, listed_max)
}

/// A workspace directory held open: `read` is what to list, `shown` the
/// path under the workspace the reply names, `.` for the workspace itself.
struct Held {
    read: PathBuf,
    shown: String,
    _anchor: Anchor,
}

/// The refusal for any path that cannot be listed under the workspace. It
/// names nothing outside the workspace, so it cannot tell a client what is
/// out there (a link out, a missing directory and a file all read alike).
const UNSUPPORTED: &str = "the workspace explorer is not available on this platform yet";

const NOT_LISTED: &str = "that directory cannot be listed in the workspace";

/// The plain names of `rel`; `..`, a root or a drive prefix is outside.
fn plain_names(rel: &str) -> Result<Vec<&std::ffi::OsStr>, String> {
    let mut names = Vec::new();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(n) => names.push(n),
            Component::CurDir => {}
            _ => return Err("the path is outside the workspace".into()),
        }
    }
    Ok(names)
}

/// `rel` under `root` (D337): its components must be plain names, each a real
/// directory, never a link. On Linux every component is opened from its
/// parent's descriptor with `O_NOFOLLOW` and the listing reads that
/// descriptor, so nothing swapped into the tree after the check is followed.
/// Elsewhere the call is refused: no atomic confinement exists there yet.
fn confined(root: &Workspace, rel: &str) -> Result<Held, String> {
    let names = plain_names(rel)?;
    let (read, anchor) = root.anchor(&names).map_err(|e| match e.kind() {
        std::io::ErrorKind::Unsupported => UNSUPPORTED.to_owned(),
        _ => {
            // The reply names nothing; the cause (a missing directory, a
            // link, EMFILE) is for the operator.
            eprintln!(
                "rusty-mux-server: workspace-list /{}: {e}",
                names_joined(&names)
            );
            NOT_LISTED.to_owned()
        }
    })?;
    let shown = if names.is_empty() {
        ".".to_owned()
    } else {
        names_joined(&names)
    };
    Ok(Held {
        read,
        shown,
        _anchor: anchor,
    })
}

/// The directory a block was launched in, held open (D337). The identity
/// of that directory is fixed when the block starts, so a later rename or
/// a link swapped in at its old path changes nothing: every listing walks
/// down from this descriptor, never from the path. Unknown when it could
/// not be opened, after a restart, and off Linux; the call is refused then.
#[derive(Clone, Default)]
pub(crate) struct Workspace(Option<Arc<Root>>);

/// Compares equal to every other: which directory a block holds is not part
/// of its saved record.
impl PartialEq for Workspace {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Workspace {}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Workspace(held)"
        } else {
            "Workspace(unknown)"
        })
    }
}

impl Workspace {
    /// Whether the directory is held.
    pub(crate) fn is_known(&self) -> bool {
        self.0.is_some()
    }
}

fn names_joined(names: &[&std::ffi::OsStr]) -> String {
    let parts: Vec<_> = names.iter().map(|n| n.to_string_lossy()).collect();
    parts.join("/")
}

#[cfg(target_os = "linux")]
struct Root(i32);

#[cfg(target_os = "linux")]
impl Drop for Root {
    fn drop(&mut self) {
        let _ = rusty_libc::fd::close(self.0);
    }
}

#[cfg(target_os = "linux")]
impl Workspace {
    /// Holds `dir` open as it is now. Unknown when it cannot be opened.
    pub(crate) fn open(dir: &Path) -> Self {
        use rusty_libc::fd::{AT_FDCWD, O_CLOEXEC, O_DIRECTORY, O_RDONLY, openat};
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return Self(None);
        };
        let flags = O_RDONLY | O_DIRECTORY | O_CLOEXEC;
        Self(
            openat(AT_FDCWD, &path, flags, 0)
                .ok()
                .map(|fd| Arc::new(Root(fd))),
        )
    }

    /// The held directory as a path only a child of this process resolves
    /// to it: its inherited descriptor. For the spawn's `chdir`.
    pub(crate) fn spawn_cwd(&self) -> Option<PathBuf> {
        let path = PathBuf::from(format!("/proc/self/fd/{}", self.0.as_ref()?.0));
        // Without a usable `/proc` the program starts at its path instead.
        path.is_dir().then_some(path)
    }

    /// `names` walked down from the held directory, each opened from its
    /// parent's descriptor with `O_NOFOLLOW`. The first descriptor is a new
    /// one for the same directory (`.`), so the held one is never closed.
    fn anchor(&self, names: &[&std::ffi::OsStr]) -> std::io::Result<(PathBuf, Anchor)> {
        use rusty_libc::fd::{O_CLOEXEC, O_DIRECTORY, O_NOFOLLOW, O_RDONLY, openat};
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c = |b: &[u8]| {
            CString::new(b).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
        };
        let io_err = |e: rusty_libc::Errno| std::io::Error::other(format!("{e:?}"));
        let root = self.0.as_ref().ok_or(std::io::ErrorKind::NotFound)?;
        let flags = O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC;
        let mut held = Anchor(Vec::new());
        let mut fd = openat(root.0, &c(b".")?, flags, 0).map_err(io_err)?;
        held.0.push(fd);
        for name in names {
            fd = openat(fd, &c(name.as_bytes())?, flags, 0).map_err(io_err)?;
            held.0.push(fd);
        }
        Ok((PathBuf::from(format!("/proc/self/fd/{fd}")), held))
    }
}

/// The descriptors of one walk, closed when the listing is done.
#[cfg(target_os = "linux")]
struct Anchor(Vec<i32>);

#[cfg(target_os = "linux")]
impl Drop for Anchor {
    fn drop(&mut self) {
        for fd in self.0.drain(..) {
            let _ = rusty_libc::fd::close(fd);
        }
    }
}

/// Only Linux has the descriptor walk, so elsewhere the call fails closed
/// rather than offer a confinement a racing process could beat.
#[cfg(not(target_os = "linux"))]
struct Root;

#[cfg(not(target_os = "linux"))]
struct Anchor;

#[cfg(not(target_os = "linux"))]
impl Workspace {
    pub(crate) fn open(_: &Path) -> Self {
        Self(None)
    }

    pub(crate) fn spawn_cwd(&self) -> Option<PathBuf> {
        None
    }

    fn anchor(&self, _: &[&std::ffi::OsStr]) -> std::io::Result<(PathBuf, Anchor)> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

const NOT_A_WORK_TREE: &str = "the workspace is not in a git work tree it belongs to";

const NOT_STATTED: &str = "that entry cannot be read in the workspace";

/// `terminal.workspace-stat`: what one entry under the workspace is, as
/// `KIND<TAB>SIZE<TAB>MTIME`: the kind as in [`list`], the size in bytes
/// and the modification time in seconds since the epoch. The entry is not
/// followed if it is a link (`l`), and every directory above it is walked
/// as [`confined`] does. The workspace itself is `.`.
pub(crate) fn stat(root: &Workspace, rel: &str) -> Result<String, String> {
    let names = plain_names(rel)?;
    let (last, parents) = match names.split_last() {
        Some((last, parents)) => (Some(last), parents),
        None => (None, &names[..]),
    };
    let (dir, _anchor) = root.anchor(parents).map_err(|e| match e.kind() {
        std::io::ErrorKind::Unsupported => UNSUPPORTED.to_owned(),
        _ => NOT_STATTED.to_owned(),
    })?;
    // The anchor is a descriptor's link, which names the directory itself, so
    // the workspace is read through it and an entry below it is not followed.
    let meta = match last {
        Some(l) => fs::symlink_metadata(dir.join(l)),
        None => fs::metadata(&dir),
    }
    .map_err(|_| NOT_STATTED.to_owned())?;
    let kind = match meta.file_type() {
        t if t.is_symlink() => 'l',
        t if t.is_dir() => 'd',
        t if t.is_file() => 'f',
        _ => 'o',
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok(format!("{kind}\t{}\t{mtime}", meta.len()))
}

/// Output of `git status` read at most; more is a refusal, not a cut list.
#[cfg(target_os = "linux")]
const TOUCHED_MAX: usize = 256 * 1024;
/// How long `git` may run before it is killed.
#[cfg(target_os = "linux")]
const TOUCHED_DEADLINE: Duration = Duration::from_secs(4);

/// Runs `git` in the held workspace directory and returns its stdout.
///
/// The repository belongs to whoever works in it, so what `git` would run
/// for it is turned off: no fsmonitor hook, no hooks directory, no pager,
/// no prompt, no optional lock. Killed after [`TOUCHED_DEADLINE`].
#[cfg(target_os = "linux")]
fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(["-c", "core.hooksPath=/dev/null", "--no-pager"])
        .args(args)
        .current_dir(dir)
        // Its own process group, so the deadline ends what git started (a
        // clean filter) as well as git.
        .process_group(0)
        // The repository is the one the directory is in, never one the
        // server's environment names.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "git is not available to the server".to_owned())?;
    let out = child.stdout.take().ok_or("git gave no output")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.take(TOUCHED_MAX as u64 + 1).read_to_end(&mut buf);
        buf
    });
    let deadline = std::time::Instant::now() + TOUCHED_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            // Past the deadline, or the wait itself failed.
            _ => break None,
        }
        // A reader that stops at its limit drops the pipe, and git ends on
        // SIGPIPE, so a flood needs no check here.
    };
    let ok = status.is_some_and(|s| s.success());
    // The group is ended either way: a finished git may leave a filter behind.
    let group = i32::try_from(child.id()).unwrap_or(0);
    if group > 0 {
        let _ = rusty_libc::process::killpg(group, rusty_libc::signal::SIGKILL);
    }
    if status.is_none() {
        let _ = child.wait();
    }
    let buf = reader
        .join()
        .map_err(|_| "git's output was lost".to_owned())?;
    if buf.len() > TOUCHED_MAX {
        return Err("too many changes to list".into());
    }
    if !ok {
        return Err("the workspace is not a git work tree, or git failed".into());
    }
    Ok(buf)
}

#[cfg(not(target_os = "linux"))]
fn git(_: &Path, _: &[&str]) -> Result<Vec<u8>, String> {
    Err(UNSUPPORTED.into())
}

/// Git's two status bytes as text, a space as `.`.
fn code_of(code: &[u8]) -> String {
    code.iter()
        .map(|b| if *b == b' ' { '.' } else { *b as char })
        .collect()
}

/// `terminal.workspace-touched`: what `git status` says changed under the
/// workspace, for the explorer's overlay. The first line is `.`; each other
/// is `XY<TAB>PATH`, `XY` being git's two status letters with a space as `.`
/// (`M.` staged, `.M` worktree, `??` untracked) and `PATH` relative to the
/// workspace, [`escape`]d. Renames are not followed: a rename is a delete and
/// an add. Linux only, as the rest of the workspace calls.
pub(crate) fn touched(root: &Workspace) -> Result<String, String> {
    let (dir, _anchor) = root.anchor(&[]).map_err(|e| match e.kind() {
        std::io::ErrorKind::Unsupported => UNSUPPORTED.to_owned(),
        _ => NOT_LISTED.to_owned(),
    })?;
    // Git's own account of where it is: the work tree it uses must hold this
    // directory, at the prefix it reports. A `core.worktree` that points
    // elsewhere fails that, and nothing of the other tree is listed.
    let top = git(&dir, &["rev-parse", "--show-toplevel", "--show-prefix"])?;
    let top = String::from_utf8_lossy(&top);
    let mut lines = top.lines();
    let (top, prefix) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
    let here = fs::canonicalize(&dir).map_err(|_| NOT_LISTED.to_owned())?;
    let inside = here
        .strip_prefix(top)
        .ok()
        .map(|r| r.to_string_lossy().into_owned());
    if inside.as_deref().map(|r| r.trim_end_matches('/')) != Some(prefix.trim_end_matches('/')) {
        return Err(NOT_A_WORK_TREE.into());
    }
    let prefix = prefix.to_owned();
    let raw = git(
        &dir,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=normal",
            "--",
            ".",
        ],
    )?;
    let mut out = String::from(".");
    for rec in raw.split(|b| *b == 0).filter(|r| r.len() > 3) {
        let (code, path) = (&rec[..2], &rec[3..]);
        let Ok(path) = std::str::from_utf8(path) else {
            continue;
        };
        let Some(path) = path.strip_prefix(prefix.as_str()) else {
            continue;
        };
        // An untracked workspace is one record naming itself, `sub/`.
        let path = if path.is_empty() { "." } else { path };
        out.push_str(&format!("\n{}\t{}", code_of(code), escape(path)));
    }
    Ok(out)
}

/// The listing text of the directory at `read`, headed by `shown`.
fn render(
    read: &Path,
    shown: &str,
    follow: bool,
    read_max: usize,
    listed_max: usize,
) -> Result<String, String> {
    let mut reader = fs::read_dir(read).map_err(|e| {
        // Confined (`!follow`): a refusal must not name where it looked.
        if follow {
            format!("{}: {e}", read.display())
        } else {
            NOT_LISTED.to_owned()
        }
    })?;
    let (mut entries, mut unread) = (Vec::new(), 0_usize);
    for item in reader.by_ref().take(read_max) {
        match item {
            Ok(e) => match (kind(&e.path(), &e, follow), e.file_name().into_string()) {
                (Some(k), Ok(name)) => entries.push((k, name)),
                // A name that is not UTF-8 cannot be sent or typed back.
                _ => unread += 1,
            },
            Err(_) => unread += 1,
        }
    }
    let more = reader.next().is_some();
    entries.sort_by_cached_key(|(k, name)| (*k != 'd', name.to_lowercase(), name.clone()));
    let left = entries.len().saturating_sub(listed_max);
    entries.truncate(listed_max);
    let mut out = escape(shown);
    for (k, name) in &entries {
        out.push_str(&format!("\n{k}\t{}", escape(name)));
    }
    if more {
        out.push_str(&format!("\n>\t{}", left + 1));
    } else if left > 0 {
        out.push_str(&format!("\n+\t{left}"));
    }
    if unread > 0 {
        out.push_str(&format!("\n!\t{unread}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-list-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn directories_come_first_then_names_without_regard_to_case() {
        let dir = scratch("order");
        fs::create_dir(dir.join("zeta")).unwrap();
        fs::create_dir(dir.join("Alpha")).unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::write(dir.join("A.txt"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(lines, ["d\tAlpha", "d\tzeta", "f\tA.txt", "f\tb.txt"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_relative_path_is_taken_from_the_base() {
        let dir = scratch("rel");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("a.txt"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), Some("sub/../sub"), None).unwrap();
        let mut lines = text.lines();
        assert!(lines.next().unwrap().ends_with("sub"));
        assert_eq!(lines.collect::<Vec<_>>(), ["f\ta.txt"]);
        fs::remove_dir_all(dir).unwrap();
    }

    /// A tab cannot be in a Windows file name, so this one is Unix only.
    #[cfg(unix)]
    #[test]
    fn names_are_escaped() {
        let dir = scratch("esc");
        fs::write(dir.join("a\tb"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        assert_eq!(text.lines().last(), Some("f\ta\\tb"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_told_by_what_it_points_to() {
        let dir = scratch("link");
        fs::create_dir(dir.join("real")).unwrap();
        fs::write(dir.join("file"), "").unwrap();
        std::os::unix::fs::symlink("real", dir.join("to-dir")).unwrap();
        std::os::unix::fs::symlink("file", dir.join("to-file")).unwrap();
        std::os::unix::fs::symlink("nowhere", dir.join("broken")).unwrap();
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            ["d\treal", "d\tto-dir", "l\tbroken", "f\tfile", "l\tto-file"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn files(dir: &Path, n: usize) {
        for i in 0..n {
            fs::write(dir.join(format!("f{i:05}")), "").unwrap();
        }
    }

    #[test]
    fn what_is_left_out_is_counted() {
        let dir = scratch("many");
        files(&dir, 10);
        let text = list_with(Some(dir.to_str().unwrap()), None, None, 100, 7).unwrap();
        assert_eq!(text.lines().count(), 1 + 7 + 1);
        assert_eq!(text.lines().last(), Some("+\t3"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_scan_that_stops_early_says_the_count_is_a_lower_bound() {
        let dir = scratch("scan");
        files(&dir, 11);
        let at = |max| list_with(Some(dir.to_str().unwrap()), None, None, max, 5).unwrap();
        // The directory holds exactly what is read: the count is exact.
        assert_eq!(at(11).lines().last(), Some("+\t6"));
        // One more than is read: at least 6 are out (5 read, 1 never read).
        assert_eq!(at(10).lines().last(), Some(">\t6"));
        assert_eq!(at(6).lines().last(), Some(">\t2"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_gate_refuses_past_its_limit_until_a_listing_ends() {
        let gate = Gate::default();
        let held: Vec<_> = (0..MAX_RUNNING).map(|_| gate.take().unwrap()).collect();
        assert!(gate.take().is_err());
        drop(held);
        assert!(gate.take().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_workspace_listing_stays_under_its_root() {
        let dir = scratch("root");
        fs::create_dir_all(dir.join("ws").join("sub")).unwrap();
        fs::write(dir.join("outside.txt"), "").unwrap();
        fs::write(dir.join("ws").join("sub").join("a.txt"), "").unwrap();
        let root = dir.join("ws");
        let root = root.to_str().unwrap();
        let ok = list(
            Some(root),
            Some("sub"),
            Some(&Workspace::open(Path::new(root))),
        )
        .unwrap();
        // The first line is under the workspace, never an absolute path.
        assert_eq!(ok.lines().collect::<Vec<_>>(), ["sub", "f\ta.txt"]);
        let top = list(Some(root), None, Some(&Workspace::open(Path::new(root)))).unwrap();
        assert_eq!(top.lines().next(), Some("."));
        for escape in ["..", "sub/../..", "/", dir.to_str().unwrap()] {
            let reason = list(
                Some(root),
                Some(escape),
                Some(&Workspace::open(Path::new(root))),
            )
            .unwrap_err();
            assert!(
                reason.contains("outside the workspace"),
                "{escape}: {reason}"
            );
            assert!(!reason.contains(dir.to_str().unwrap()), "{reason}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_refusal_names_nothing_outside_the_workspace() {
        let dir = scratch("scope");
        fs::create_dir_all(dir.join("ws")).unwrap();
        fs::write(dir.join("ws").join("file"), "").unwrap();
        let root = dir.join("ws");
        let root = root.to_str().unwrap();
        for missing in ["no/such", "file"] {
            let reason = list(
                Some(root),
                Some(missing),
                Some(&Workspace::open(Path::new(root))),
            )
            .unwrap_err();
            assert_eq!(reason, NOT_LISTED, "{missing}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_is_not_followed_in_a_workspace() {
        let dir = scratch("rootlink");
        fs::create_dir_all(dir.join("ws").join("in")).unwrap();
        fs::create_dir(dir.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("ws").join("out")).unwrap();
        std::os::unix::fs::symlink("in", dir.join("ws").join("local")).unwrap();
        let root = dir.join("ws");
        let root = root.to_str().unwrap();
        // A link, even one that stays inside, is neither entered nor classified
        // by its target: it is `l`.
        let top = list(Some(root), None, Some(&Workspace::open(Path::new(root)))).unwrap();
        assert_eq!(
            top.lines().skip(1).collect::<Vec<_>>(),
            ["d\tin", "l\tlocal", "l\tout"]
        );
        for link in ["out", "local"] {
            let reason = list(
                Some(root),
                Some(link),
                Some(&Workspace::open(Path::new(root))),
            )
            .unwrap_err();
            assert_eq!(reason, NOT_LISTED, "{link}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_launch_directory_is_held_not_its_path() {
        let dir = scratch("held");
        fs::create_dir_all(dir.join("ws")).unwrap();
        fs::write(dir.join("ws").join("mine"), "").unwrap();
        fs::create_dir(dir.join("other")).unwrap();
        fs::write(dir.join("other").join("theirs"), "").unwrap();
        let ws = Workspace::open(&dir.join("ws"));
        // The path is renamed away and a link to somewhere else put in its place.
        fs::rename(dir.join("ws"), dir.join("moved")).unwrap();
        std::os::unix::fs::symlink(dir.join("other"), dir.join("ws")).unwrap();
        let top = list(None, None, Some(&ws)).unwrap();
        assert_eq!(top.lines().collect::<Vec<_>>(), [".", "f\tmine"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stat_tells_an_entry_and_stays_in_the_workspace() {
        let dir = scratch("stat");
        fs::create_dir_all(dir.join("ws").join("sub")).unwrap();
        fs::write(dir.join("ws").join("sub").join("a.txt"), "four").unwrap();
        fs::write(dir.join("outside"), "").unwrap();
        std::os::unix::fs::symlink(dir.join("outside"), dir.join("ws").join("out")).unwrap();
        let ws = Workspace::open(&dir.join("ws"));
        let kind =
            |rel| stat(&ws, rel).map(|t| t.split('\t').take(2).collect::<Vec<_>>().join(" "));
        assert!(kind(".").unwrap().starts_with("d "));
        assert_eq!(kind("sub/a.txt"), Ok("f 4".into()));
        assert!(kind("sub").unwrap().starts_with("d "));
        // A link is `l` and not followed; one in the path is refused.
        assert!(kind("out").unwrap().starts_with("l "));
        assert_eq!(kind("out/x"), Err(NOT_STATTED.into()));
        assert_eq!(kind("missing"), Err(NOT_STATTED.into()));
        for outside in ["..", "sub/../..", "/"] {
            assert_eq!(
                kind(outside),
                Err("the path is outside the workspace".into())
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn touched_lists_changes_under_the_workspace_only() {
        use std::process::Command;
        let dir = scratch("touched");
        let ws_dir = dir.join("sub");
        fs::create_dir_all(&ws_dir).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&dir)
                .status()
                .map(|s| s.success());
            assert_eq!(ok.ok(), Some(true), "git {args:?}");
        };
        git(&["init", "-q"]);
        fs::write(ws_dir.join("tracked"), "1").unwrap();
        fs::write(dir.join("elsewhere"), "1").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "one"]);
        fs::write(ws_dir.join("tracked"), "2").unwrap();
        fs::write(ws_dir.join("new file"), "").unwrap();
        fs::write(dir.join("elsewhere"), "2").unwrap();
        let text = touched(&Workspace::open(&ws_dir)).unwrap();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [".", ".M\ttracked", "??\tnew file"]
        );
        // An untracked workspace is one record for itself.
        let fresh = dir.join("fresh");
        fs::create_dir(&fresh).unwrap();
        fs::write(fresh.join("x"), "").unwrap();
        assert_eq!(
            touched(&Workspace::open(&fresh))
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [".", "??\t."]
        );
        // A `core.worktree` pointing elsewhere is refused, listing nothing of it.
        let elsewhere = scratch("touched-elsewhere");
        fs::write(elsewhere.join("secret"), "").unwrap();
        git(&["config", "core.worktree", elsewhere.to_str().unwrap()]);
        assert_eq!(
            touched(&Workspace::open(&ws_dir)),
            Err(NOT_A_WORK_TREE.into())
        );
        git(&["config", "--unset", "core.worktree"]);
        fs::remove_dir_all(elsewhere).unwrap();
        // A directory that is no work tree is refused, naming nothing.
        let plain = scratch("touched-plain");
        let reason = touched(&Workspace::open(&plain)).unwrap_err();
        assert!(!reason.contains(plain.to_str().unwrap()), "{reason}");
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(plain).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_directory_that_cannot_be_opened_is_not_a_workspace() {
        let dir = scratch("gone");
        assert!(!Workspace::open(&dir.join("missing")).is_known());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_workspace_listing_is_refused_where_it_cannot_be_confined() {
        let dir = scratch("unsupported");
        let root = dir.to_str().unwrap();
        assert_eq!(
            list(Some(root), None, Some(&Workspace::open(Path::new(root)))).unwrap_err(),
            UNSUPPORTED
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_directory_or_base_is_refused() {
        assert!(list(None, None, None).is_err());
        assert!(list(None, Some("rel"), None).is_err());
        assert!(list(Some("/"), Some("no/such/dir"), None).is_err());
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    #[test]
    fn a_unc_path_is_refused() {
        for path in ["\\\\host\\share", "\\\\?\\C:\\", "\\\\.\\pipe"] {
            let reason = list(None, Some(path), None).unwrap_err();
            assert!(reason.contains("network"), "{path}: {reason}");
        }
    }
}
