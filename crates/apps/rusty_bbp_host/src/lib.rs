//! Host for the Blackboard Protocol over a `rusty_bbp` file store.
//!
//! - [`mcp`]: the per-turn MCP server an agent harness talks to over stdio.
//!   One process serves one turn; when that turn ends every call is refused.
//! - [`human`]: the human channel as CLI actions.
//! - [`moderator`]: the loop that launches one agent harness per granted
//!   turn and the runner per selected run, until the task closes.
//! - [`runner`]: the test supervisor. Applies a candidate in a fresh checkout,
//!   runs the frozen profile set under a sandbox, stores log and report.
//! - [`profiles`]: the profile set a task freezes at open.
//! - [`admin`]: open a task and assign roles.
//! - [`args`]: the small argument parser the `bbp` binary uses.

#![forbid(unsafe_code)]

pub mod admin;
pub mod args;
pub mod human;
pub mod mcp;
pub mod moderator;
pub mod profiles;
pub mod runner;

use rusty_bbp::*;
use std::path::Path;

/// Wall-clock milliseconds since the Unix epoch, as the core's `Time`.
pub fn now() -> Time {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Time(ms)
}

/// Open the file store at `dir` and rebuild `task`'s state under the
/// directory's master secret.
pub fn open_driver(dir: &Path, task: &TaskId) -> Result<Driver<FsStore>, String> {
    let master = master::load(dir)?;
    let store = FsStore::open(dir).map_err(|e| e.to_string())?;
    let mut d = Driver::new(store, task.clone()).with_master(master);
    d.reload().map_err(|e| format!("{e:?}"))?;
    Ok(d)
}

/// The per-directory master secret that keys every execution token and run
/// secret. `<dir>/master`, 64 hex characters, owner-readable only. It is the
/// one thing a reader of the log must not have.
pub mod master {
    use rusty_bbp::Sha256;
    use std::path::Path;

    fn path(dir: &Path) -> std::path::PathBuf {
        dir.join("master")
    }

    /// Create the master if the directory has none, from `rusty_rand`.
    ///
    /// Single winner: the candidate is written whole to a private temporary
    /// file and published with a no-clobber `hard_link`, so two first opens
    /// racing each other both come back with the key on disk and no later
    /// process ever overwrites a key an earlier one is already using.
    pub fn ensure(dir: &Path) -> Result<[u8; 32], String> {
        if path(dir).exists() {
            return load(dir);
        }
        let bytes = rusty_rand::bytes(32).map_err(|e| format!("rusty_rand: {e}"))?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut candidate = [0u8; 32];
        candidate.copy_from_slice(&bytes);
        let tmp = dir.join(format!("master.{}", Sha256::of(&bytes).hex()));
        rusty_atomic_file::write_private(&tmp, Sha256(candidate).hex().as_bytes())
            .map_err(|e| format!("master: {e}"))?;
        let published = std::fs::hard_link(&tmp, path(dir));
        let _ = std::fs::remove_file(&tmp);
        match published {
            Ok(()) => Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load(dir),
            Err(e) => Err(format!("master: {e}")),
        }
    }

    /// Read the master; a directory without one was never opened.
    pub fn load(dir: &Path) -> Result<[u8; 32], String> {
        let text = std::fs::read_to_string(path(dir))
            .map_err(|e| format!("{}: {e} (open the task first)", path(dir).display()))?;
        Sha256::from_hex(text.trim())
            .map(|s| s.0)
            .ok_or_else(|| "master file is not 64 hex characters".to_owned())
    }
}

/// A fresh operation id for a call the agent did not name.
pub fn fresh_op() -> OpId {
    let bytes = rusty_rand::bytes(8).unwrap_or_else(|_| now().0.to_le_bytes().to_vec());
    OpId(format!(
        "auto-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// Render a response as JSON text.
pub fn response_json(r: &Response) -> String {
    rusty_serde::json::to_string(r).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
