//! A [`MemoryStore`] saved to one JSON file, so a restarted hub keeps its
//! nodes and pre-auth keys. The file holds pre-auth secrets, so it is
//! created `0600`. Each change is written to a temporary file and renamed
//! over the old one: a crash leaves the old table or the new one, never a
//! torn file.

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ts_types::tailcfg::MapResponse;
use ts_types::{DiscoPublic, MachinePublic, NodePublic};

use crate::store::{KeyPolicy, MemoryStore, NodeStore, RegisterError, Revocation};

/// Why a node table file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("node table I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("corrupt node table {path}: {reason}")]
    Corrupt { path: PathBuf, reason: String },
}

/// A node table that survives restarts.
pub struct FileStore {
    inner: MemoryStore,
    path: PathBuf,
}

impl FileStore {
    /// Opens the table at `path`, or starts an empty one if the file does
    /// not exist yet. `node_ttl` is how long a registration lasts, in
    /// seconds; `None` never expires.
    pub fn open(path: impl Into<PathBuf>, node_ttl: Option<u64>) -> Result<Self, StoreError> {
        let path = path.into();
        let inner = match fs::read(&path) {
            Ok(bytes) => {
                MemoryStore::from_json(&bytes, node_ttl).map_err(|e| StoreError::Corrupt {
                    path: path.clone(),
                    reason: e.to_string(),
                })?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let empty = MemoryStore::default();
                match node_ttl {
                    Some(secs) => empty.with_node_ttl(secs),
                    None => empty,
                }
            }
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        Ok(Self { inner, path })
    }

    fn flush(&self) -> Result<(), RegisterError> {
        let storage = |e: &dyn std::fmt::Display| RegisterError::Storage(e.to_string());
        let json = self.inner.to_json().map_err(|e| storage(&e))?;
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, &json).map_err(|e| storage(&e))?;
        fs::rename(&tmp, &self.path).map_err(|e| storage(&e))?;
        sync_parent(&self.path).map_err(|e| storage(&e))
    }

    /// Runs `change`; if the new table cannot be saved, puts the old one
    /// back, so a refused write never leaves a node admitted in memory only.
    fn saved<T>(
        &mut self,
        change: impl FnOnce(&mut MemoryStore) -> Result<T, RegisterError>,
    ) -> Result<T, RegisterError> {
        let before = self.inner.clone();
        let out = change(&mut self.inner)?;
        if let Err(e) = self.flush() {
            self.inner = before;
            return Err(e);
        }
        Ok(out)
    }
}

/// Writes `bytes` to a new file at `path` (`0600`), replacing a stale one so
/// its old mode cannot carry over.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Makes the rename durable.
#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    fs::File::open(dir.unwrap_or(Path::new(".")))?.sync_all()
}

/// Windows cannot open a directory to sync it.
#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

impl NodeStore for FileStore {
    fn add_preauth(
        &mut self,
        key: &str,
        login: &str,
        policy: KeyPolicy,
        now: u64,
    ) -> Result<(), RegisterError> {
        self.saved(|s| s.add_preauth(key, login, policy, now))
    }

    fn register(
        &mut self,
        machine: MachinePublic,
        node_key: NodePublic,
        hostname: &str,
        auth_key: &str,
        now: u64,
    ) -> Result<(), RegisterError> {
        self.saved(|s| s.register(machine, node_key, hostname, auth_key, now))
    }

    fn revoke(&mut self, machine: &MachinePublic) -> Revocation {
        // Never rolled back: a revoke that could not be saved still holds
        // for this run, and the caller is told it is not durable.
        let mut out = self.inner.revoke(machine);
        out.saved = self.flush();
        out
    }

    fn has_node_key(&self, key: &NodePublic, now: u64) -> bool {
        self.inner.has_node_key(key, now)
    }

    fn update(
        &mut self,
        machine: &MachinePublic,
        disco: DiscoPublic,
        endpoints: Vec<SocketAddr>,
    ) -> bool {
        self.inner.update(machine, disco, endpoints)
    }

    fn netmap(&self, machine: &MachinePublic, omit_peers: bool, now: u64) -> Option<MapResponse> {
        self.inner.netmap(machine, omit_peers, now)
    }
}
