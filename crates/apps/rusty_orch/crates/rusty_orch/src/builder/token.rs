//! The builder session's gateway token (#1382 slice 2, D232).
//!
//! At launch orch mints a token with `orch-token` and writes it to
//! `<state>/tokens/<claim>/<n>/token` (0600, in a 0700 directory); the
//! `jail.json` `token` slot names that file and [`VAR`], and `lel-jail` hands
//! the value to the agent (D107). The file sits outside every jail root, which
//! `JailSpec::validate` checks again on the paths that are written. Only the
//! `jti` and `sub` are ever logged.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_jail::spec::FileToken;
use orch_token::{mint, policy_digest, Claims, PolicyDigest, Role, Scope, SigningKey};
use rusty_codec::toml::TomlValue;

/// The variable the agent receives the token in.
pub const VAR: &str = "LEL_TOKEN";

/// A lens policy file as far as the token needs it: its lens and digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    lens: String,
    digest: PolicyDigest,
}

/// Read the lens out of a policy file's bytes. The lens must equal the file
/// name without `.toml`, as the gateway requires, and the role must be
/// `builder`; otherwise the gateway would deny every tool, so launch fails
/// here instead.
pub fn policy(file_name: &str, bytes: &[u8]) -> Result<Policy, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{file_name}: not UTF-8"))?;
    let doc = TomlValue::parse_str(text).map_err(|e| format!("{file_name}: {e}"))?;
    let lens = doc
        .get("lens")
        .and_then(TomlValue::as_str)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| format!("{file_name}: `lens` must be a non-empty string"))?;
    if file_name.strip_suffix(".toml") != Some(lens) {
        return Err(format!(
            "{file_name}: `lens = \"{lens}\"` must match the file name"
        ));
    }
    if doc.get("role").and_then(TomlValue::as_str) != Some(Role::Builder.as_str()) {
        return Err(format!("{file_name}: a builder needs `role = \"builder\"`"));
    }
    Ok(Policy {
        lens: lens.to_owned(),
        digest: policy_digest(bytes),
    })
}

/// What every builder token is signed with and bound to.
#[derive(Debug)]
pub struct Signer {
    key: SigningKey,
    policy: Policy,
    host: String,
}

impl Signer {
    /// From already-read parts. `host` names this machine in `iss`.
    pub fn new(key: SigningKey, policy: Policy, host: &str) -> Self {
        Self {
            key,
            policy,
            host: host.to_owned(),
        }
    }

    /// Read the key and the policy file; the host is this machine's
    /// `uname` nodename.
    pub fn load(key: &Path, kid: &str, policy_path: &Path) -> Result<Self, String> {
        let pem = std::fs::read(key).map_err(|e| format!("{}: {e}", key.display()))?;
        let key = SigningKey::from_pem(&pem, kid).map_err(|e| format!("{}: {e}", key.display()))?;
        let name = policy_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{}: no file name", policy_path.display()))?;
        let bytes =
            std::fs::read(policy_path).map_err(|e| format!("{}: {e}", policy_path.display()))?;
        Ok(Self::new(key, policy(name, &bytes)?, &hostname()?))
    }
}

/// One session's place on the board.
#[derive(Debug, Clone, Copy)]
pub struct Seat<'a> {
    pub board: &'a str,
    pub task: &'a str,
    pub claim: &'a str,
    pub n: u32,
}

/// A minted token and what may be logged about it.
pub struct Minted {
    /// The token itself. Never logged.
    pub token: String,
    pub jti: String,
    pub sub: String,
}

/// Mint the token for one builder session starting at `now` (Unix seconds)
/// with a `wall` budget; `jti` is fresh random.
pub fn mint_for(
    signer: &Signer,
    seat: Seat<'_>,
    jti: [u8; 16],
    now: u64,
    wall: Duration,
) -> Result<Minted, String> {
    let scope = Scope::new(
        seat.board,
        seat.task,
        seat.claim,
        &signer.policy.lens,
        Role::Builder,
        seat.n,
    )
    .map_err(|e| format!("token: {e}"))?;
    let claims = Claims::new(
        &signer.host,
        scope,
        signer.policy.digest.clone(),
        Vec::new(),
        jti,
        now,
        wall,
    )
    .map_err(|e| format!("token: {e}"))?;
    let token = mint(&claims, &signer.key).map_err(|e| format!("token: {e}"))?;
    Ok(Minted {
        token,
        jti: claims.jti(),
        sub: claims.subject(),
    })
}

/// `<state>/tokens/<claim>/<n>/token`: beside, not under, the run dir and
/// the worktree.
pub fn path(state: &Path, claim: &str, n: u32) -> PathBuf {
    state
        .join("tokens")
        .join(claim)
        .join(n.to_string())
        .join("token")
}

/// A session's token directory and file, created by this process and owned
/// by it alone: dropping it removes exactly what [`acquire`] made, and
/// nothing that was there before.
#[derive(Debug)]
pub struct TokenFile {
    dir: PathBuf,
    file: PathBuf,
    /// The handle `create_new` returned: writes go to the inode this guard
    /// created, never to whatever the path names later.
    handle: File,
}

impl TokenFile {
    /// The token file's path.
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Write the token through the handle this guard created.
    pub fn write(&mut self, token: &str) -> Result<(), String> {
        self.handle
            .write_all(token.as_bytes())
            .and_then(|()| self.handle.sync_all())
            .map_err(|e| format!("{}: {e}", self.file.display()))
    }
}

impl TokenFile {
    /// Remove the file only if the path still names the inode this guard
    /// created, then the (then empty) directory.
    fn release(&self) -> Result<(), String> {
        let ours = self
            .handle
            .metadata()
            .map_err(|e| format!("{}: {e}", self.file.display()))?;
        match std::fs::symlink_metadata(&self.file) {
            Ok(now) if (now.dev(), now.ino()) == (ours.dev(), ours.ino()) => {
                std::fs::remove_file(&self.file)
                    .map_err(|e| format!("{}: {e}", self.file.display()))?;
            }
            Ok(_) => {
                return Err(format!(
                    "{} no longer names the file this run created; left in place",
                    self.file.display()
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", self.file.display())),
        }
        std::fs::remove_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))
    }
}

impl Drop for TokenFile {
    fn drop(&mut self) {
        if let Err(e) = self.release() {
            eprintln!("rusty_orch builder: token file not removed: {e}");
        }
    }
}

/// Claim the session's token directory and create an empty 0600 file in it.
///
/// The session directory (`<n>`, 0700) is created exclusively, so two
/// attempts can never share it. One that already exists is never removed or
/// reused: it belongs to an earlier run of the same session number, and only
/// an operator can tell whether that run is still alive, so launch is refused
/// with its path. Nothing is cleaned up unless this call created it.
pub fn acquire(file: &Path) -> Result<TokenFile, String> {
    let dir = file
        .parent()
        .ok_or_else(|| format!("{}: no directory", file.display()))?;
    let claim_dir = dir
        .parent()
        .ok_or_else(|| format!("{}: no claim directory", dir.display()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(claim_dir)
        .map_err(|e| format!("{}: {e}", claim_dir.display()))?;
    DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => format!(
                "{} exists from an earlier run of this session; remove it once no session uses it",
                dir.display()
            ),
            _ => format!("{}: {e}", dir.display()),
        })?;
    let created = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(file);
    let handle = match created {
        Ok(handle) => handle,
        Err(e) => {
            // The directory is ours and empty; leave nothing behind, and say
            // so if that fails too.
            let cleanup = std::fs::remove_dir(dir)
                .err()
                .map(|c| format!("; {} not removed: {c}", dir.display()))
                .unwrap_or_default();
            return Err(format!("{}: {e}{cleanup}", file.display()));
        }
    };
    Ok(TokenFile {
        dir: dir.to_owned(),
        file: file.to_owned(),
        handle,
    })
}

/// The `jail.json` slot for `file`.
pub fn slot(file: &Path) -> FileToken {
    FileToken {
        file: file.display().to_string(),
        var: VAR.to_owned(),
    }
}

/// A fresh 128-bit `jti`.
pub fn jti() -> Result<[u8; 16], String> {
    let mut id = [0u8; 16];
    rusty_rand::fill(&mut id).map_err(|e| format!("jti: {e}"))?;
    Ok(id)
}

fn hostname() -> Result<String, String> {
    let names = rusty_libc::process::uname().map_err(|e| format!("uname: {e:?}"))?;
    names
        .nodename()
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "uname: hostname is not UTF-8".to_owned())
}

#[cfg(test)]
mod tests;
