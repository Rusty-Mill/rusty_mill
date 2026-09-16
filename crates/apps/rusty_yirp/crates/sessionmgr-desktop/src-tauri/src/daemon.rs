//! Starting a daemon, without depending on `sessionmgr-daemon`.
//!
//! `sessionmgr-tui` never has to do this: it is reached through the
//! `sessionmgr tui` subcommand, and the daemon binary itself calls
//! `client::ensure_daemon` before handing off to it. This app has no
//! such wrapper -- it is its own executable, launched directly -- so it
//! shells out to the one it does have: the `sessionmgr` binary's own
//! `daemon start` subcommand, which is already idempotent and already
//! blocks until the daemon answers. That keeps this crate from ever
//! constructing a process command line of its own beyond "run this one,
//! already-named external program with these two words" -- the same
//! "cannot name a process type" boundary `paths.rs`'s doc comment
//! describes, just applied to starting the daemon instead of a session.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sessionmgr_protocol::{Request, Response};

use crate::client;

/// Locates the `sessionmgr` binary: first a sibling of this executable
/// (how a packaged install ships both binaries together), then bare
/// `sessionmgr` resolved from `PATH` (how a development build reaches
/// it, both installed via `cargo build --workspace`).
fn sessionmgr_exe() -> PathBuf {
    let name = if cfg!(windows) {
        "sessionmgr.exe"
    } else {
        "sessionmgr"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(name);
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from(name)
}

fn ping(socket: &Path) -> bool {
    matches!(
        client::request(socket, &Request::Ping),
        Ok(Response::Pong { .. })
    )
}

/// Starts a daemon at `root` unless one is already answering.
pub fn ensure_daemon(root: &Path) -> Result<(), String> {
    let socket = crate::paths::daemon_socket(root);
    if ping(&socket) {
        return Ok(());
    }

    let exe = sessionmgr_exe();
    let output = Command::new(&exe)
        .arg("daemon")
        .arg("start")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("running `{} daemon start`: {e}", exe.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`{} daemon start` failed: {}",
            exe.display(),
            stderr.trim()
        ));
    }

    if !ping(&socket) {
        return Err(format!(
            "`{} daemon start` reported success but {} is not answering",
            exe.display(),
            socket.display()
        ));
    }
    Ok(())
}
