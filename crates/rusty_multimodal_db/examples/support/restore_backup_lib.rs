//! Offline restore orchestration for `ADR-0070` (`RST-FR-001`–`005`).
//! Included via `#[path]` by the CLI and its integration tests, matching
//! the migration tool's placement. Uses only existing public portable
//! constructors and `std::fs`; no new library API or on-disk format.

use rusty_multimodal_db::durability::DurabilityError;
use rusty_multimodal_db::generic::entity::open_entity_production_stack_portable;
use rusty_multimodal_db::generic::memory::open_memory_production_stack_portable;
use rusty_multimodal_db::generic::query::AllIds;
use rusty_multimodal_db::generic::relation::open_relation_production_stack_portable;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

/// The production constructor used for verification (`RST-FR-001`/`005`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// A `memories.mmap` stack, including its `mentions` companions.
    Memory,
    /// An `entities.mmap` stack, including its relation companions.
    Entity,
    /// A `relations.mmap` record stack.
    Relation,
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

/// The files moved and records actually reopened (`RST-FR-002`/`005`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreReport {
    /// Number of backup files restored under their original names.
    pub files: u64,
    /// Number of records enumerated by the verified stack's `AllIds`.
    pub records: usize,
}

/// A refusal, filesystem failure, or real production reopen failure.
#[derive(Debug)]
pub enum RestoreError {
    /// A target-prefix entry already exists; no copy began (`RST-FR-004`).
    TargetExists {
        /// The conflicting entry.
        path: PathBuf,
    },
    /// Listing or staging failed before any final name changed (`RST-FR-003`).
    StagingIo {
        /// The source or destination involved in the failed operation.
        path: PathBuf,
        /// The filesystem's original error.
        source: io::Error,
    },
    /// A final rename or directory operation failed (`RST-FR-003`).
    /// Some final files may already exist; inspect them before retrying.
    InstallIo {
        /// The final destination or temporary directory involved.
        path: PathBuf,
        /// The filesystem's original error.
        source: io::Error,
    },
    /// Copied files failed the domain's real reopen and remain in place
    /// for inspection (`RST-FR-005`).
    Verification(DurabilityError),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetExists { path } => {
                write!(
                    f,
                    "target already exists: {}; refusing to overwrite",
                    path.display()
                )
            }
            Self::StagingIo { path, source } => {
                write!(f, "staging {}: {source}", path.display())
            }
            Self::InstallIo { path, source } => {
                write!(
                    f,
                    "installing {}: {source}; inspect any partial restore before retrying",
                    path.display()
                )
            }
            Self::Verification(source) => {
                write!(
                    f,
                    "verification reopen failed: {source}; restored files left in place"
                )
            }
        }
    }
}

impl std::error::Error for RestoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TargetExists { .. } => None,
            Self::StagingIo { source, .. } | Self::InstallIo { source, .. } => Some(source),
            Self::Verification(source) => Some(source),
        }
    }
}

fn staging_io(path: &Path, source: io::Error) -> RestoreError {
    RestoreError::StagingIo {
        path: path.to_path_buf(),
        source,
    }
}

fn install_io(path: &Path, source: io::Error) -> RestoreError {
    RestoreError::InstallIo {
        path: path.to_path_buf(),
        source,
    }
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn stage(backup_dir: &Path, tmp: &Path) -> Result<Vec<OsString>, RestoreError> {
    let entries = std::fs::read_dir(backup_dir).map_err(|e| staging_io(backup_dir, e))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| staging_io(backup_dir, e))?;
        let name = entry.file_name();
        // A directory entry deliberately fails copy: backups are flat,
        // and a partial staging set must never reach the real names.
        std::fs::copy(entry.path(), tmp.join(&name)).map_err(|e| staging_io(&entry.path(), e))?;
        names.push(name);
    }
    Ok(names)
}

/// Restore one flat, server-produced backup (`RST-FR-002`–`005`).
///
/// Refuses any existing entry matching `target_stem`'s filename prefix
/// before touching the backup or writing anything. All backup files stage
/// in a fresh sibling temporary directory, preserving their names; only
/// after every copy succeeds are they renamed into the target parent.
/// The filename in `target_stem` must match the backup's original stem.
/// Run offline, without a server or another restore writing this target.
///
/// Each final rename is atomic, **not the whole set** (`RST-FR-003`).
/// A hard kill between renames can leave a partial restore, which the
/// prefix check refuses on retry. Remove that partial set before retrying.
/// Verification uses the domain's existing portable constructor and
/// counts its records through `AllIds`, leaving copied files in place
/// even when verification fails. The operator then points `SERVER_DATA_DIR`
/// at the target parent and (re)starts the binary (`RST-FR-006`).
///
/// # Errors
///
/// [`RestoreError::TargetExists`] names a conflicting target entry.
/// [`RestoreError::StagingIo`] reports invalid paths, listing, or copy
/// failures, with no real target names touched. The temporary directory
/// is removed on staging failure (best effort if the filesystem also
/// refuses cleanup). [`RestoreError::InstallIo`] can leave a partial
/// final set. [`RestoreError::Verification`] carries the production
/// constructor's original [`DurabilityError`]; copied files are retained.
pub fn restore(
    backup_dir: &Path,
    target_stem: &Path,
    domain: Domain,
) -> Result<RestoreReport, RestoreError> {
    let parent = parent_dir(target_stem);
    let stem = target_stem.file_name().ok_or_else(|| {
        staging_io(
            target_stem,
            io::Error::new(io::ErrorKind::InvalidInput, "target_stem has no file name"),
        )
    })?;
    let stem = stem.to_string_lossy();
    match std::fs::read_dir(parent) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| staging_io(parent, e))?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(stem.as_ref())
                {
                    return Err(RestoreError::TargetExists { path: entry.path() });
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(staging_io(parent, e)),
    }

    // PID + invocation counter prevents concurrent restores in this
    // process from sharing staging files. create_dir never reuses a stale
    // directory, including one left by a previous process with this PID.
    // Resolve the parent so a bare relative stem stages beside the
    // current directory, just as an absolute target does.
    let absolute_parent = std::path::absolute(parent).map_err(|e| staging_io(parent, e))?;
    let staging_parent = parent_dir(&absolute_parent);
    // Create the containing directory needed by the sibling staging
    // directory, but leave the target parent itself until staging ends.
    std::fs::create_dir_all(staging_parent).map_err(|e| staging_io(staging_parent, e))?;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let tmp = loop {
        let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let candidate = staging_parent.join(format!(".restore-tmp-{}-{n}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(staging_io(&candidate, e)),
        }
    };
    let result = (|| {
        let names = stage(backup_dir, &tmp)?;
        std::fs::create_dir_all(parent).map_err(|e| install_io(parent, e))?;
        for name in &names {
            let destination = parent.join(name);
            std::fs::rename(tmp.join(name), &destination)
                .map_err(|e| install_io(&destination, e))?;
        }
        Ok(names.len() as u64)
    })();
    let files = match result {
        Ok(files) => files,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(error);
        }
    };
    std::fs::remove_dir(&tmp).map_err(|e| install_io(&tmp, e))?;
    let records = match domain {
        Domain::Memory => {
            open_memory_production_stack_portable(target_stem).map(|s| s.all_ids().len())
        }
        Domain::Entity => {
            open_entity_production_stack_portable(target_stem).map(|s| s.all_ids().len())
        }
        Domain::Relation => {
            open_relation_production_stack_portable(target_stem).map(|s| s.all_ids().len())
        }
    }
    .map_err(RestoreError::Verification)?;
    Ok(RestoreReport { files, records })
}
