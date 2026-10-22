//! The MCP channel of a confined harness.
//!
//! A confined harness cannot read the task directory, so the `bbp mcp` it
//! starts cannot derive its turn's token there. Instead that process, inside
//! the sandbox, is a relay: with `BBP_MCP_RELAY` naming a pipe directory, it
//! copies its stdin into the `mcp.in` named pipe and the `mcp.out` pipe to
//! its stdout. The broker, which the moderator starts outside the sandbox
//! with the turn's binding, serves each connection on those pipes with the
//! ordinary per-turn [`Server`]. The harness is granted the two pipes and
//! nothing else of the directory that holds them, so it can neither replace
//! them nor reach the broker any other way.

use crate::mcp::{Binding, Server};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Names the pipe directory; set, `bbp mcp` relays instead of serving.
pub const RELAY_ENV: &str = "BBP_MCP_RELAY";
const REQUESTS: &str = "mcp.in";
const REPLIES: &str = "mcp.out";

/// The request and reply pipes in `dir`: what a confined harness may open.
pub fn pipes(dir: &Path) -> (PathBuf, PathBuf) {
    (dir.join(REQUESTS), dir.join(REPLIES))
}

/// Make `dir` afresh, owner-only, holding the two pipes.
#[cfg(target_os = "linux")]
pub fn make_pipes(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", dir.display());
    if dir.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(dir).map_err(|e| fail(&e))?;
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .map_err(|e| fail(&e))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| fail(&e))?;
    let (requests, replies) = pipes(dir);
    for pipe in [requests, replies] {
        let name =
            std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).map_err(|e| fail(&e))?;
        rusty_libc::fs::mkfifo(&name, 0o600).map_err(|e| fail(&format_args!("{e:?}")))?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn make_pipes(_: &Path) -> Result<(), String> {
    Err("harness confinement requires Linux".into())
}

/// `bbp mcp` inside the sandbox: stdin to the broker, the broker to stdout,
/// until both directions close.
pub fn relay(dir: &Path) -> Result<(), String> {
    let (requests, replies) = pipes(dir);
    // Same opening order as the broker, so neither waits on the other.
    let mut up = OpenOptions::new()
        .write(true)
        .open(&requests)
        .map_err(|e| format!("{}: {e}", requests.display()))?;
    let mut down = File::open(&replies).map_err(|e| format!("{}: {e}", replies.display()))?;
    let forward = std::thread::spawn(move || io::copy(&mut io::stdin().lock(), &mut up));
    io::copy(&mut down, &mut io::stdout().lock()).map_err(|e| format!("relay: {e}"))?;
    match forward.join() {
        Ok(copied) => copied.map(drop).map_err(|e| format!("relay: {e}")),
        Err(_) => Err("relay: request copy panicked".into()),
    }
}

/// Serve each relay connection on the pipes in `dir` as `binding`'s turn,
/// until `harness` has exited.
pub fn broker(dir: &Path, binding: Binding, harness: u32) -> Result<(), String> {
    watch(harness);
    let (requests, replies) = pipes(dir);
    loop {
        let input = File::open(&requests).map_err(|e| format!("{}: {e}", requests.display()))?;
        let output = OpenOptions::new()
            .write(true)
            .open(&replies)
            .map_err(|e| format!("{}: {e}", replies.display()))?;
        for (file, path) in [(&input, &requests), (&output, &replies)] {
            if !is_fifo(file) {
                return Err(format!("{} is not a named pipe", path.display()));
            }
        }
        // A relay that goes away mid-reply is a closed connection, not an
        // error of the broker's.
        if let Err(e) = Server::new(binding.clone())?.serve_on(BufReader::new(input), output) {
            eprintln!("bbp mcp-broker: connection ended: {e}");
        }
    }
}

#[cfg(unix)]
fn is_fifo(file: &File) -> bool {
    use std::os::unix::fs::FileTypeExt;
    file.metadata().is_ok_and(|m| m.file_type().is_fifo())
}

#[cfg(not(unix))]
fn is_fifo(_: &File) -> bool {
    false
}

/// Exit this process once `harness` is gone. The moderator kills the broker
/// with its harness; this covers a moderator that died first, which would
/// otherwise leave the broker waiting on a pipe no one will open.
fn watch(harness: u32) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if !alive(harness) {
            std::process::exit(0);
        }
    });
}

#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    rusty_libc::process::kill(pid, 0) != Err(rusty_libc::Errno::ESRCH)
}

#[cfg(not(target_os = "linux"))]
fn alive(_: u32) -> bool {
    false
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn pipes_are_fresh_owner_only_fifos() -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("bbp_pipes_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("stale")).map_err(|e| e.to_string())?;
        make_pipes(&dir)?;
        assert!(!dir.join("stale").exists(), "a leftover is replaced");
        for pipe in [pipes(&dir).0, pipes(&dir).1] {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&pipe)
                .map_err(|e| e.to_string())?;
            assert!(is_fifo(&file));
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = dir
            .metadata()
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    }
}
