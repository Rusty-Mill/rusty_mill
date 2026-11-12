//! Reaching a server on another machine through one OpenSSH
//! forward of a local Unix socket to the server's. The client then connects
//! to the local socket as it would to a local server, once per connection,
//! while SSH logs in once. The SSH login is the identity: the server sees
//! the forward's connections as coming from the user `ssh` logged in as.
//!
//! The local socket lives in a fresh directory only its owner can enter,
//! which is removed, with the forward stopped, when the `Forward` drops.
//! The forward is Unix only; the options and `ssh -G` lookup are not, so
//! `rusty-mux-ctl ssh check` uses them everywhere (term pack D194).
//! Windows, whose OpenSSH cannot forward a local Unix socket, reaches the
//! server through [`Bridge`] instead: one `ssh` running
//! `rusty-mux-ctl bridge` per connection.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(any(windows, test))]
pub use bridge::Bridge;
#[cfg(unix)]
pub use forward::{Forward, args, private_dir};

/// The options that keep the forward owned by the `ssh` this client starts
/// and reaps. A user's config may share a master connection or fork after
/// login, either of which would leave the listener with a process `Drop`
/// never stops. Options given with `-o` win over the config, whose host,
/// user, key and jump settings still apply. `ForkAfterAuthentication`
/// needs OpenSSH 8.7 or later. `rusty-mux-ctl ssh check` passes the same.
pub const OWNED: [&str; 6] = [
    "ControlMaster=no",
    "ControlPath=none",
    "ControlPersist=no",
    "ForkAfterAuthentication=no",
    "ExitOnForwardFailure=yes",
    "StreamLocalBindUnlink=yes",
];

/// The option [`Login::Batch`] adds.
pub const BATCH: &str = "BatchMode=yes";

/// How `ssh` may log in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    /// It may ask on the terminal, for a password or a host key.
    Interactive,
    /// It must not ask (`BatchMode=yes`): a reconnect, while the client
    /// reads the terminal, logs in with a key or an agent or fails.
    Batch,
}

/// The name `ssh` would connect to for `host`, after the user's config
/// (`HostName`, aliases), from `ssh -G`, which resolves the options and
/// connects to nothing. A `ProxyJump` is not followed: UDP goes straight
/// to this name, and the client falls back to SSH if nothing answers.
pub fn hostname(program: &OsStr, host: &str) -> Result<String, String> {
    let out = Command::new(program)
        .args(["-G", "--", host])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("starting {} -G: {e}", Path::new(program).display()))?;
    if !out.status.success() {
        return Err(format!("ssh -G {host} failed ({})", out.status));
    }
    parse_hostname(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| format!("ssh -G {host} printed no hostname"))
}

/// The value of the `hostname` line in `ssh -G` output.
fn parse_hostname(config: &str) -> Option<String> {
    config.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == "hostname" && !value.is_empty()).then(|| value.to_owned())
    })
}

#[cfg(any(windows, test))]
pub mod bridge;
#[cfg(unix)]
mod forward;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hostname_comes_from_ssh_g() {
        let config = "user nano\nhostname box.example.net\nport 22\n";
        assert_eq!(parse_hostname(config).as_deref(), Some("box.example.net"));
        assert_eq!(parse_hostname("user nano\nhostnames x\n"), None);
    }
}
