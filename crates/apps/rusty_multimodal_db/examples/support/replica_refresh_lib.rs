//! Replica refresh orchestration for `ADR-0118` (`RRF-FR-001`–`004`):
//! the operator recipe `docs/design/SERVER-REPLICATION-DESIGN.md` named
//! and did not ship, as running code. Included via `#[path]` by the CLI
//! and its integration test, matching the restore tool's placement.
//! Uses only the public client, the public portable constructors and
//! `std::fs`; no new library API or on-disk format.

use rusty_multimodal_db::durability::DurabilityError;
use rusty_multimodal_db::generic::entity::open_entity_production_stack_portable;
use rusty_multimodal_db::generic::memory::open_memory_production_stack_portable;
use rusty_multimodal_db::generic::query::AllIds;
use rusty_multimodal_db::generic::relation::open_relation_production_stack_portable;
use rusty_multimodal_db::server::client::{ClientError, ConnectOptions, SchemaDrivenClient};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The table's domain: decides the stem the files are verified under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Memory,
    Entity,
    Relation,
}

impl Domain {
    /// The stem every file of the table shares (`BAK-FR-006`).
    pub fn stem(self) -> &'static str {
        match self {
            Self::Memory => "memories.mmap",
            Self::Entity => "entities.mmap",
            Self::Relation => "relations.mmap",
        }
    }
}

impl FromStr for Domain {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "memory" => Ok(Self::Memory),
            "entity" => Ok(Self::Entity),
            "relation" => Ok(Self::Relation),
            _ => Err("domain must be exactly memory, entity, or relation"),
        }
    }
}

/// One completed refresh (`RRF-FR-002`/`003`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    /// The fresh directory holding the table; point `SERVER_DATA_DIR` here.
    pub directory: PathBuf,
    /// Files the snapshot carried and the directory now holds.
    pub files: u64,
    /// Bytes across those files.
    pub bytes: u64,
    /// Records the verification reopen counted.
    pub records: usize,
}

/// Everything that can stop a refresh, each leaving no directory the
/// operator could mistake for a complete snapshot.
#[derive(Debug)]
pub enum RefreshError {
    /// Connecting or authenticating failed; nothing was written.
    Connect(ClientError),
    /// `FetchSnapshot` was refused or failed; nothing was written.
    Fetch(ClientError),
    /// A file under the staging directory could not be written or synced;
    /// the staging directory is removed.
    Staging { path: PathBuf, source: io::Error },
    /// The staged directory could not be renamed into place; it is removed.
    Install { path: PathBuf, source: io::Error },
    /// The installed directory does not reopen through the domain's own
    /// portable constructor; it is left in place for inspection under a
    /// `.failed-` prefix, never under a name a refresh loop would pick.
    Verification {
        path: PathBuf,
        source: DurabilityError,
    },
}

impl std::fmt::Display for RefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "connecting: {e}"),
            Self::Fetch(e) => write!(f, "fetching the snapshot: {e}"),
            Self::Staging { path, source } => write!(f, "staging {}: {source}", path.display()),
            Self::Install { path, source } => {
                write!(f, "installing {}: {source}", path.display())
            }
            Self::Verification { path, source } => write!(
                f,
                "verification reopen of {} failed: {source}; {}",
                path.display(),
                if path.exists() {
                    "kept for inspection"
                } else {
                    "removed (it could not be renamed aside)"
                }
            ),
        }
    }
}

impl std::error::Error for RefreshError {}

/// What a refresh connects with: the address and the replication token
/// (`RPL-FR-002`), plus everything else `ConnectOptions` carries (TLS).
pub struct Target {
    pub addr: String,
    pub options: ConnectOptions,
}

impl Target {
    /// Plaintext, token only — loopback or a trusted network; for
    /// anything else set `options.tls` as `SchemaDrivenClient::connect_with`
    /// documents.
    pub fn new(addr: impl Into<String>, token: &str) -> Self {
        Self {
            addr: addr.into(),
            options: ConnectOptions::new().token(token),
        }
    }
}

/// `RRF-FR-001`–`003`: fetch one snapshot of `domain`'s table from
/// `target` and install it as a fresh directory under `root`, named by
/// the second it was taken and a sequence number, crash-safely: every
/// file is written and synced under a staging directory, the staging
/// directory is synced and renamed into place in one step, `root` is
/// synced, and the result is verified by a real reopen through the
/// domain's own portable constructor. A crash at any point leaves
/// either no new directory or a complete, verified one; a failed
/// verification keeps the directory under a `.failed-` name.
pub fn refresh(
    target: &Target,
    root: &Path,
    domain: Domain,
) -> Result<RefreshReport, RefreshError> {
    let mut client = SchemaDrivenClient::connect_with(&target.addr, target.options.clone())
        .map_err(RefreshError::Connect)?;
    let files = client.fetch_snapshot().map_err(RefreshError::Fetch)?;
    drop(client);

    std::fs::create_dir_all(root).map_err(|e| staging(root, e))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    // `RGM-FR-006` (ADR-0121): a fixed prefix and a plausible epoch, so
    // `snapshots`/`prune` can never mistake a directory the operator
    // keeps here (an ISO date is three numbers too) for one of ours.
    let name = format!("{SNAPSHOT_PREFIX}{stamp}-{}-{seq}", std::process::id());
    let staging_dir = root.join(format!(".refresh-tmp-{name}"));
    let final_dir = root.join(&name);

    // `RGM-FR-008` (ADR-0121): the server names the files; only a single
    // normal path component may be joined under the staging directory.
    for (file_name, _) in &files {
        if !is_plain_file_name(file_name) {
            return Err(RefreshError::Staging {
                path: staging_dir,
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "the snapshot names a file that is not a plain file name: {file_name:?}"
                    ),
                ),
            });
        }
    }

    std::fs::create_dir(&staging_dir).map_err(|e| staging(&staging_dir, e))?;
    let mut bytes = 0u64;
    let written = (|| -> io::Result<()> {
        for (file_name, contents) in &files {
            let path = staging_dir.join(file_name);
            let mut file = std::fs::File::create(&path)?;
            file.write_all(contents)?;
            file.sync_all()?;
            bytes += contents.len() as u64;
        }
        std::fs::File::open(&staging_dir)?.sync_all()
    })();
    if let Err(source) = written {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(RefreshError::Staging {
            path: staging_dir,
            source,
        });
    }
    if let Err(source) = std::fs::rename(&staging_dir, &final_dir) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(RefreshError::Install {
            path: final_dir,
            source,
        });
    }
    if let Err(source) = std::fs::File::open(root).and_then(|d| d.sync_all()) {
        return Err(RefreshError::Install {
            path: root.to_path_buf(),
            source,
        });
    }

    let stem = final_dir.join(domain.stem());
    let records = match domain {
        Domain::Memory => open_memory_production_stack_portable(&stem).map(|s| s.all_ids().len()),
        Domain::Entity => open_entity_production_stack_portable(&stem).map(|s| s.all_ids().len()),
        Domain::Relation => {
            open_relation_production_stack_portable(&stem).map(|s| s.all_ids().len())
        }
    };
    let records = match records {
        Ok(n) => n,
        Err(source) => {
            // `RGM-FR-007` (ADR-0121): a directory that does not reopen
            // must not stay under a name a refresh loop would pick. Kept
            // under `.failed-` for inspection when the rename works;
            // removed when it does not, and the error says which.
            let failed = root.join(format!(".failed-{name}"));
            let path = match std::fs::rename(&final_dir, &failed) {
                Ok(()) => failed,
                Err(_) => {
                    let _ = std::fs::remove_dir_all(&final_dir);
                    final_dir
                }
            };
            return Err(RefreshError::Verification { path, source });
        }
    };
    Ok(RefreshReport {
        directory: final_dir,
        files: files.len() as u64,
        bytes,
        records,
    })
}

/// The prefix every snapshot directory name carries (`RGM-FR-006`).
pub const SNAPSHOT_PREFIX: &str = "refresh-";

/// The earliest `<secs>` a snapshot name can carry: 2001-09-09, so a
/// small number that happens to parse (an ISO date's year) never does.
const EARLIEST_STAMP: u64 = 1_000_000_000;

/// `RGM-FR-008`: exactly one normal path component — no separator, no
/// `..`, no root, nothing empty.
pub fn is_plain_file_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) && !name.contains(['/', '\\'])
}

/// `RRF-FR-004`: the snapshot directories under `root`, oldest first —
/// only names a refresh made (`refresh-<secs>-<pid>-<seq>` with a
/// plausible epoch), never a staging or failed one, never anything else
/// the operator keeps there.
pub fn snapshots(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<(u64, u64, u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(name) = name.strip_prefix(SNAPSHOT_PREFIX) else {
            continue;
        };
        let mut parts = name.splitn(3, '-');
        let key = (
            parts.next().and_then(|p| p.parse::<u64>().ok()),
            parts.next().and_then(|p| p.parse::<u64>().ok()),
            parts.next().and_then(|p| p.parse::<u64>().ok()),
        );
        if let (Some(secs), Some(pid), Some(seq)) = key {
            if secs >= EARLIEST_STAMP {
                dirs.push((secs, pid, seq, entry.path()));
            }
        }
    }
    dirs.sort();
    Ok(dirs.into_iter().map(|(_, _, _, path)| path).collect())
}

/// How old a leftover staging directory must be before `prune` removes
/// it (`RGL-FR-003`): well past any refresh still in flight.
const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(3600);

/// `RRF-FR-004`: remove every snapshot directory but the newest `keep`;
/// returns what was removed. `keep == 0` is refused as a no-op: a loop
/// must never delete the directory it just made. Also removes
/// `.refresh-tmp-*` directories older than an hour — a refresh that
/// crashed between creating one and renaming it (`RGL-FR-003`). A
/// `.failed-*` directory is never removed here: it is kept for the
/// operator to inspect and delete. Ordering is by the name's stamp, so
/// a clock stepped backwards can make the newest sort first; `keep`
/// should leave room for that.
pub fn prune(root: &Path, keep: usize) -> io::Result<Vec<PathBuf>> {
    if keep == 0 {
        return Ok(Vec::new());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let is_stale_staging = entry
            .file_name()
            .to_string_lossy()
            .starts_with(".refresh-tmp-")
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() > STALE_STAGING)
                .unwrap_or(false);
        if is_stale_staging {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    let all = snapshots(root)?;
    let excess = all.len().saturating_sub(keep);
    let mut removed = Vec::with_capacity(excess);
    for dir in all.into_iter().take(excess) {
        std::fs::remove_dir_all(&dir)?;
        removed.push(dir);
    }
    Ok(removed)
}

fn staging(path: &Path, source: io::Error) -> RefreshError {
    RefreshError::Staging {
        path: path.to_path_buf(),
        source,
    }
}
