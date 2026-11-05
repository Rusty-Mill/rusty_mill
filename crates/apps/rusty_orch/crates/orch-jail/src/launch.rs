//! What `lel-jail` does between the mux's fork and the agent's exec.
//!
//! The order is the v0 contract (LEL D85): validate the spec, require the
//! slot lock (fd 3) and the ACP socket (fd 4), join the session's cgroup,
//! close every other descriptor, pin every root, the cwd and the token with
//! an `O_PATH` descriptor and check the token is out of reach on what those
//! descriptors are, read the token through its descriptor, then hand off
//! to rusty_sandbox's helper with `/proc/self/fd/N` for each path. The
//! helper's Landlock rules then land on the pinned objects, which no later
//! rename or symlink, by a confined peer or anyone else, can redirect. It
//! sets rlimits, Landlock and seccomp and execs. Any failure stops before
//! the exec: nothing runs unconfined.

use std::os::fd::IntoRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::spec::{JailSpec, Resolved};

/// `O_PATH` (Linux, the same value on every architecture this builds for):
/// a handle for the object itself, usable as a path through procfs.
const O_PATH: i32 = 0o10_000_000;

/// The slot lock the cgroup holder shares (design §2 step 4).
pub const SLOT_FD: i32 = 3;
/// orch's end of the ACP socketpair.
pub const ACP_FD: i32 = 4;

/// Confines this process per the spec at `path` and execs the agent.
///
/// Returns only on failure, with the reason. A failure after the spec is
/// parsed is also written to its status file (the helper writes its own).
#[must_use]
pub fn launch(path: &Path) -> String {
    launch_with(path, || {})
}

/// [`launch`], calling `before_handoff` once every path is pinned and
/// checked and just before the helper installs its rules: the seam the
/// swap tests pause in.
#[doc(hidden)]
#[must_use]
pub fn launch_with(path: &Path, before_handoff: impl FnOnce()) -> String {
    let spec = match read_spec(path) {
        Ok(spec) => spec,
        Err(error) => return error,
    };
    match prepare(&spec) {
        Ok((spec, token)) => {
            before_handoff();
            rusty_sandbox::run_helper(&spec.helper_request(token).encode()).to_string()
        }
        Err(error) => {
            if let Err(write) = std::fs::write(spec.status(), &error) {
                return format!("{error} (and the status file: {write})");
            }
            error
        }
    }
}

fn read_spec(path: &Path) -> Result<JailSpec, String> {
    let json =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    JailSpec::parse(&json)
}

/// Every step before the helper; returns the resolved spec, which is what
/// the helper is given, and the token's value.
fn prepare(spec: &JailSpec) -> Result<(JailSpec, Option<String>), String> {
    keep_inherited(SLOT_FD, "slot lock")?;
    keep_inherited(ACP_FD, "ACP socket")?;
    join_cgroup(spec.cgroup())?;
    close_others()?;
    let spec = spec.resolve(pin)?;
    let token = spec.token().map(|t| read_token(&t.file)).transpose()?;
    Ok((spec, token))
}

/// Opens `path` once, as an `O_PATH` descriptor kept until the agent's exec
/// (it is close-on-exec), and judges it by what that descriptor is: its
/// link in `/proc/self/fd` names the object it holds, which no later rename
/// or symlink changes. The handle given back is that link, so the helper
/// opens the same object. Refused unless no one but root and this user owns
/// a part of its path, and no directory above it is writable by anyone else
/// unless sticky.
fn pin(path: &Path) -> Result<Resolved, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_PATH)
        .open(path)
        .map_err(|e| format!("opening {}: {e}", path.display()))?;
    let handle = PathBuf::from(format!("/proc/self/fd/{}", file.into_raw_fd()));
    let real = std::fs::read_link(&handle)
        .map_err(|e| format!("resolving {} ({}): {e}", path.display(), handle.display()))?;
    if !real.is_absolute() {
        return Err(format!("{} resolves to {}", path.display(), real.display()));
    }
    owned_path(path, &real)?;
    Ok(Resolved { real, handle })
}

fn owned_path(path: &Path, real: &Path) -> Result<(), String> {
    let me = rusty_libc::process::geteuid();
    for part in real.ancestors() {
        let meta = std::fs::symlink_metadata(part)
            .map_err(|e| format!("checking {}: {e}", part.display()))?;
        let swappable = part != real && meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0;
        if meta.file_type().is_symlink() || ![0, me].contains(&meta.uid()) || swappable {
            return Err(format!(
                "{} resolves through {}, which another user could replace",
                path.display(),
                part.display()
            ));
        }
    }
    Ok(())
}

/// Fails closed when `fd` is missing, and clears close-on-exec so the agent
/// inherits it through the helper's exec.
fn keep_inherited(fd: i32, what: &str) -> Result<(), String> {
    use rusty_libc::fd::{fcntl, F_GETFD, F_SETFD};
    fcntl(fd, F_GETFD, 0).map_err(|e| format!("fd {fd} ({what}) is not open: {e}"))?;
    fcntl(fd, F_SETFD, 0).map_err(|e| format!("fd {fd} ({what}): clearing close-on-exec: {e}"))?;
    Ok(())
}

/// Moves this process, and so everything the agent starts, into the
/// session's cgroup, which orch kills and the holder waits on (design §3).
fn join_cgroup(cgroup: &Path) -> Result<(), String> {
    let procs = cgroup.join("cgroup.procs");
    std::fs::write(&procs, std::process::id().to_string())
        .map_err(|e| format!("joining the cgroup {}: {e}", cgroup.display()))
}

/// Closes everything above fd 4 (design §4: the agent keeps 0-4 only).
fn close_others() -> Result<(), String> {
    rusty_libc::fd::close_range(5, u32::MAX, 0)
        .map_err(|e| format!("closing descriptors above {ACP_FD}: {e}"))
}

fn read_token(file: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(file)
        .map_err(|e| format!("checking the token {}: {e}", file.display()))?;
    // A second name could put the same file inside a root.
    if meta.nlink() != 1 {
        return Err(format!("the token {} has other links", file.display()));
    }
    let raw = std::fs::read_to_string(file)
        .map_err(|e| format!("reading the token {}: {e}", file.display()))?;
    let token = raw.trim_end_matches(['\r', '\n']);
    if token.is_empty() {
        return Err(format!("the token {} is empty", file.display()));
    }
    Ok(token.to_owned())
}
