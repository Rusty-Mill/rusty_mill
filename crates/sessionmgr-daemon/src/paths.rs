//! Where everything lives on disk, and the socket-path budget that
//! constrains it.
//!
//! # The `AF_UNIX` path budget
//!
//! `sun_path` allows a hard **107 usable bytes for an entire socket
//! path**, on Windows as much as on Unix. That is small, and it is spent
//! before this module gets a say: a Windows state root under
//! `C:\Users\<user>\AppData\Local\sessionmgr` is already ~45 bytes, and a
//! test's state root under `std::env::temp_dir()` can be worse.
//! `rusty_prime_agent` hit exactly this limit and had to shorten its own
//! test directory names to recover.
//!
//! So sockets deliberately do **not** live beside the session data they
//! belong to. `sessions/<id>/worker.sock` would spend 12 (`sessions/`) +
//! 12 (id) + 1 + 11 (`worker.sock`) = 36 bytes; the flat `s/<id>.sock`
//! layout here spends 19. That is the whole reason for the odd-looking
//! split between a session's directory and its socket.

use std::path::{Path, PathBuf};

use sessionmgr_core::SessionId;

use crate::error::{Error, Result};

/// Environment variable overriding the state root. Set by every
/// black-box test so runs are isolated from each other and from the
/// developer's real sessions.
pub const HOME_ENV: &str = "SESSIONMGR_HOME";

/// Resolves the state root.
///
/// Order: `$SESSIONMGR_HOME`, then the platform's conventional per-user
/// state location. Explicit config beats magic globals, and the env var
/// exists so a test never has to touch a real user's directory.
pub fn state_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(HOME_ENV) {
        let dir = PathBuf::from(dir);
        if dir.as_os_str().is_empty() {
            return Err(Error::usage(format!("{HOME_ENV} is set but empty")));
        }
        return Ok(dir);
    }
    #[cfg(windows)]
    {
        // `LOCALAPPDATA`, not `APPDATA`: this is machine-local state that
        // must never follow a roaming profile between machines -- it
        // records pids, which mean nothing anywhere else.
        let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            Error::usage(format!(
                "neither {HOME_ENV} nor LOCALAPPDATA is set; cannot locate a state directory"
            ))
        })?;
        Ok(PathBuf::from(base).join("sessionmgr"))
    }
    #[cfg(unix)]
    {
        if let Some(base) = std::env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(base).join("sessionmgr"));
        }
        let home = std::env::var_os("HOME").ok_or_else(|| {
            Error::usage(format!(
                "neither {HOME_ENV}, XDG_STATE_HOME, nor HOME is set; cannot locate a state directory"
            ))
        })?;
        Ok(PathBuf::from(home).join(".local/state/sessionmgr"))
    }
}

pub fn ensure_dir(context: &'static str, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(context, dir.to_path_buf(), e))
}

/// The flat socket directory. See the module docs for why sockets are not
/// stored beside their session data.
pub fn socket_dir(root: &Path) -> PathBuf {
    root.join("s")
}

/// The daemon's public socket. One character of basename, for the same
/// budget reason.
pub fn daemon_socket(root: &Path) -> PathBuf {
    socket_dir(root).join("d.sock")
}

/// A worker's private socket.
pub fn worker_socket(root: &Path, id: &SessionId) -> PathBuf {
    socket_dir(root).join(format!("{id}.sock"))
}

/// The daemon's pointer file: pid and start fingerprint of the running
/// supervisor.
pub fn daemon_state(root: &Path) -> PathBuf {
    root.join("daemon.json")
}

/// The daemon's stderr when started detached. Without it, a supervisor
/// that dies before binding its socket fails completely silently and the
/// only symptom is a client timeout.
pub fn daemon_log(root: &Path) -> PathBuf {
    root.join("daemon.log")
}

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

pub fn session_dir(root: &Path, id: &SessionId) -> PathBuf {
    sessions_dir(root).join(id.as_str())
}

/// The session's pointer file. Small, rewritten in full, and the only
/// thing a restarting supervisor reads to rebuild its registry.
pub fn session_state(root: &Path, id: &SessionId) -> PathBuf {
    session_dir(root, id).join("state.json")
}

/// The session's append-only output log -- the source of truth a
/// reattaching client replays, as opposed to `state.json`'s pointer role.
pub fn session_transcript(root: &Path, id: &SessionId) -> PathBuf {
    session_dir(root, id).join("transcript.jsonl")
}

/// A worker's stderr, for the same reason [`daemon_log`] exists.
pub fn worker_log(root: &Path, id: &SessionId) -> PathBuf {
    session_dir(root, id).join("worker.log")
}

/// Removes a stale socket file so a bind can succeed.
///
/// Binding fails if the path already exists, and a socket file always
/// outlives an uncleanly-killed process -- which, for a tool whose whole
/// premise is surviving unclean exits, is the normal case rather than the
/// exception. A missing file is success, not an error.
pub fn clear_socket(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io("removing a stale socket", path.to_path_buf(), e)),
    }
}

/// Warns on stderr if a socket path is close to or past the `sun_path`
/// budget, rather than letting the bind fail with an OS error that says
/// nothing about *why* the path was too long.
///
/// A warning and not a hard failure: the limit is on the OS side, so if a
/// bind at 104 bytes works on some platform, refusing it here would be
/// this tool inventing a restriction. The bind failing right afterwards
/// is the real enforcement; this just makes the cause legible.
pub fn warn_if_socket_path_is_long(path: &Path) {
    const BUDGET: usize = 107;
    let len = path.as_os_str().len();
    if len > BUDGET * 3 / 4 {
        eprintln!(
            "sessionmgr: warning: socket path is {len} bytes, and the AF_UNIX limit is {BUDGET} \
             ({}). Consider setting {HOME_ENV} to a shorter directory.",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_stay_within_the_af_unix_budget_for_a_realistic_root() {
        // A realistic worst-case Windows state root, spelled out so the
        // budget is asserted against something concrete rather than
        // against a short test path that would pass trivially.
        let root = PathBuf::from(r"C:\Users\a-fairly-long-username\AppData\Local\sessionmgr");
        let id = SessionId::new(1_700_000_000_000, 7);
        let worker = worker_socket(&root, &id);
        assert!(
            worker.as_os_str().len() <= 107,
            "worker socket path is {} bytes, over the sun_path budget: {}",
            worker.as_os_str().len(),
            worker.display()
        );
        assert!(daemon_socket(&root).as_os_str().len() <= 107);
    }

    #[test]
    fn the_flat_socket_layout_is_actually_shorter_than_the_obvious_one() {
        // Guards the reason this module splits sockets away from session
        // directories at all -- if a later refactor "tidies" sockets back
        // under `sessions/<id>/`, this fails and says why.
        let root = PathBuf::from("/r");
        let id = SessionId::new(1_700_000_000_000, 7);
        let flat = worker_socket(&root, &id).as_os_str().len();
        let obvious = session_dir(&root, &id).join("worker.sock").as_os_str().len();
        assert!(
            flat < obvious,
            "the flat socket layout ({flat}) must be shorter than the co-located one ({obvious})"
        );
    }

    #[test]
    fn clearing_a_missing_socket_is_not_an_error() {
        let missing = std::env::temp_dir().join("sessionmgr-definitely-not-here.sock");
        assert!(clear_socket(&missing).is_ok());
    }

    #[test]
    fn an_empty_home_env_is_rejected_rather_than_silently_using_the_cwd() {
        // Setting an env var to "" is a common scripting slip; treating
        // it as "the current directory" would scatter state into whatever
        // repo the user happened to be standing in.
        //
        // `std::env::set_var` mutates process-global state shared with
        // every other test in this binary, so this test is the only one
        // here that touches `SESSIONMGR_HOME`.
        let previous = std::env::var_os(HOME_ENV);
        std::env::set_var(HOME_ENV, "");
        let result = state_root();
        match previous {
            Some(value) => std::env::set_var(HOME_ENV, value),
            None => std::env::remove_var(HOME_ENV),
        }
        assert!(result.is_err());
    }
}
