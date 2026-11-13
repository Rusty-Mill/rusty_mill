//! Ai Env Manager filesystem adapter (pack D4, D6, D11). Implements the two
//! ports the pure core leaves open: [`observe`] reads a target path into an
//! [`Observed`]; [`apply`] carries out a [`Plan`] the core produced.
//!
//! Safety rules: a plan with a refusal is never applied, and an `Unlink`
//! removes only a symlink/junction or a folder holding our marker.

mod marker;

use ai_env_core::{Action, Entry, LinkMethod, Observed, Os, Plan, Tool};
use marker::{copy_marked, read_marker, tree_hash, MARKER};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Why applying a plan failed.
#[derive(Debug)]
pub enum Error {
    /// The plan holds a refusal; nothing was touched.
    Refused(PathBuf),
    /// An `Unlink` target is a plain file or a folder without our marker.
    NotManaged(PathBuf),
    /// Every link method in the fallback chain failed; the last error.
    Io(PathBuf, io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Refused(p) => write!(f, "plan refused for {}; nothing was changed", p.display()),
            Error::NotManaged(p) => write!(f, "{} is not managed; not removing it", p.display()),
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

/// The host family this binary runs on.
pub const HOST: Os = if cfg!(windows) { Os::Windows } else { Os::Unix };

/// What is at `entry`'s target in `tool`. A symlink to the entry's source is
/// current; any other symlink is not ours to touch. A folder is ours only with
/// a marker, and the stored hash says whether it was edited since.
pub fn observe(entry: &Entry, tool: &Tool) -> io::Result<Observed> {
    let to = tool.target(entry);
    let meta = match fs::symlink_metadata(&to) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Observed::Absent),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        return Ok(match fs::read_link(&to) {
            Ok(t) if t == entry.source => Observed::ManagedCurrent,
            _ => Observed::Unmanaged,
        });
    }
    let Some(mark) = meta.is_dir().then(|| read_marker(&to)).flatten() else {
        return Ok(Observed::Unmanaged);
    };
    if tree_hash(&to)? != mark.sha256 {
        return Ok(Observed::ManagedModified);
    }
    let same_source = mark.source == entry.source.to_string_lossy();
    // A missing source makes the copy stale, not an error: disable still works.
    let up_to_date = same_source && tree_hash(&entry.source).is_ok_and(|h| h == mark.sha256);
    Ok(if up_to_date {
        Observed::ManagedCurrent
    } else {
        Observed::ManagedStale
    })
}

/// Applies `plan` in order. A plan with a refusal is rejected whole, before
/// any change. A link falls back along [`LinkMethod::fallback_chain`] from its
/// planned method.
pub fn apply(plan: &Plan) -> Result<(), Error> {
    if let Some(Action::Refuse { path, .. }) = plan
        .actions()
        .iter()
        .find(|a| matches!(a, Action::Refuse { .. }))
    {
        return Err(Error::Refused(path.clone()));
    }
    for action in plan.actions() {
        match action {
            Action::Link { from, to, method } => link(from, to, *method)?,
            Action::Unlink { path } => unlink(path)?,
            Action::Refuse { .. } => {}
        }
    }
    Ok(())
}

fn link(from: &Path, to: &Path, planned: LinkMethod) -> Result<(), Error> {
    let io_err = |e| Error::Io(to.to_owned(), e);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(io_err)?;
    }
    let chain = LinkMethod::fallback_chain(HOST);
    let start = chain.iter().position(|m| *m == planned).unwrap_or(0);
    let mut last = io::Error::other("no link method available");
    for method in &chain[start..] {
        match make_link(from, to, *method) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(io_err(last))
}

fn make_link(from: &Path, to: &Path, method: LinkMethod) -> io::Result<()> {
    match method {
        LinkMethod::Symlink => symlink_dir(from, to),
        LinkMethod::Junction => junction(from, to),
        LinkMethod::Copy => copy_marked(from, to),
    }
}

#[cfg(unix)]
fn symlink_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(windows)]
fn symlink_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(from, to)
}

/// Windows directory junction through `mklink /J` (no admin rights needed).
/// Not verified in this repo's CI yet (pack Q5).
#[cfg(windows)]
fn junction(from: &Path, to: &Path) -> io::Result<()> {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(to)
        .arg(from)
        .output()?;
    if status.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&status.stdout).into_owned(),
        ))
    }
}

#[cfg(not(windows))]
fn junction(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "junctions are Windows only",
    ))
}

fn unlink(path: &Path) -> Result<(), Error> {
    let io_err = |e| Error::Io(path.to_owned(), e);
    let meta = fs::symlink_metadata(path).map_err(io_err)?;
    if meta.file_type().is_symlink() {
        // Removes the link itself, never what it points at. A directory
        // symlink or junction on Windows needs `remove_dir`.
        return fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(io_err);
    }
    if meta.is_dir() && path.join(MARKER).is_file() {
        return fs::remove_dir_all(path).map_err(io_err);
    }
    Err(Error::NotManaged(path.to_owned()))
}

#[cfg(test)]
mod tests;
