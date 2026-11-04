//! Sessions that survive a server restart (C45): the store directory holds
//! the model as `rusty_multimodal_db_engine` records (`sessions.mmap` and
//! its companions; rusty_term D73) and one RTCP v6 screen per block
//! (`<block>.rtcp`). Screens are files, not records: the engine keeps every
//! record in memory and logs each change, and a screen is megabytes that
//! the server already holds as a grid (D76).
//!
//! A save writes screens first, each to a `.tmp` beside it renamed over the
//! old one, then the changed session records, so a run that dies midway
//! leaves records with screens at least as new. The directory is locked
//! while a server holds it. A store that cannot be read is moved aside and
//! the server starts empty: a bad store never stops a start.

mod record;

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusty_multimodal_db_engine::dir_lock::DirLock;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::{
    DeleteError, GenericMmapStore, InsertError, ReplaceError,
};
use rusty_mux_core::BlockId;
use rusty_term_core::{AnsiParser, Grid};

use record::{ByGenerated, ByOrder, SessionRecord};
pub(crate) use record::{Manifest, Started};

const SESSIONS: &str = "sessions.mmap";
const LOCK: &str = "server.lock";
const SCREEN: &str = "rtcp";
/// Saves between compactions of the engine's insert log.
const COMPACT_EVERY: u32 = 64;

type Sessions = GenericMmapStore<SessionRecord, ByGenerated, ByOrder>;

/// A store directory, made owner-only and locked. Clones share it.
#[derive(Clone)]
pub(crate) struct Store {
    dir: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// `None` until [`Store::load`] opens it.
    sessions: Option<Sessions>,
    /// The newest save written, so saves land one at a time and an older
    /// one finishing late never overwrites a newer one.
    written: u64,
    saves: u32,
    /// `None` only while [`Store::set_aside`] moves the directory.
    lock: Option<DirLock>,
}

/// What a store held: the manifest and each block's screen it could read.
pub(crate) struct Loaded {
    pub(crate) manifest: Manifest,
    pub(crate) screens: Vec<(BlockId, Grid, AnsiParser)>,
}

impl Store {
    /// Opens `dir`, making it (and its parents) if missing, and locks it;
    /// refused while another server holds it.
    pub(crate) fn open(dir: PathBuf) -> io::Result<Self> {
        make_private_dir(&dir)?;
        let lock = DirLock::acquire(&dir, LOCK).map_err(io::Error::other)?;
        let inner = Inner {
            sessions: None,
            written: 0,
            saves: 0,
            lock: Some(lock),
        };
        Ok(Self {
            dir,
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    fn inner(&self) -> io::Result<MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("an earlier save panicked"))
    }

    fn screen(&self, block: BlockId) -> PathBuf {
        self.dir.join(format!("{block}.{SCREEN}"))
    }

    /// Opens the session records and returns what they hold, `Ok(None)`
    /// when they hold no session. Records that cannot be read are an error; a screen
    /// that cannot be read is skipped with a warning, so its block
    /// restarts blank.
    pub(crate) fn load(&self) -> Result<Option<Loaded>, String> {
        let mut inner = self.inner().map_err(|e| e.to_string())?;
        let path = self.dir.join(SESSIONS);
        if !path.exists() {
            let empty = Sessions::create(Vec::new(), &path)
                .map_err(|e| format!("creating {SESSIONS}: {e}"))?;
            inner.sessions = Some(empty);
            return Ok(None);
        }
        let sessions =
            Sessions::open_portable(&path).map_err(|e| format!("reading {SESSIONS}: {e}"))?;
        let records: Vec<_> = sessions
            .all_ids()
            .into_iter()
            .filter_map(|id| sessions.get(id))
            .collect();
        inner.sessions = Some(sessions);
        if records.is_empty() {
            return Ok(None);
        }
        let manifest = record::manifest(records)?;
        let screens = manifest
            .started
            .keys()
            .filter_map(|block| self.load_screen(*block))
            .collect();
        Ok(Some(Loaded { manifest, screens }))
    }

    fn load_screen(&self, block: BlockId) -> Option<(BlockId, Grid, AnsiParser)> {
        let data = match fs::read(self.screen(block)) {
            Ok(data) => data,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => return warn(block, &e.to_string()),
        };
        match crate::checkpoint::restore(&data) {
            Ok((grid, parser)) => Some((block, grid, parser)),
            Err(e) => warn(block, &e),
        }
    }

    /// Writes the screens given, then the session records that changed,
    /// then removes screens of blocks the manifest no longer holds;
    /// skipped, and `Ok`, when a save numbered `generation` or later has
    /// been written already.
    pub(crate) fn save(
        &self,
        generation: u64,
        manifest: &Manifest,
        screens: &[(BlockId, Vec<u8>)],
    ) -> io::Result<()> {
        let mut inner = self.inner()?;
        if generation <= inner.written {
            return Ok(());
        }
        for (block, data) in screens {
            replace(&self.screen(*block), data)?;
        }
        let sessions = inner
            .sessions
            .as_mut()
            .ok_or_else(|| io::Error::other("the store was never loaded"))?;
        sync(sessions, record::records(manifest))?;
        inner.saves += 1;
        if inner.saves % COMPACT_EVERY == 0
            && let Some(sessions) = inner.sessions.as_mut()
        {
            sessions.compact().map_err(io::Error::other)?;
        }
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if is_stale(&path, manifest) {
                fs::remove_file(&path)?;
            }
        }
        inner.written = generation;
        Ok(())
    }

    /// Moves an unreadable store to `<dir>.bad-<seconds>` and starts a
    /// new empty one in its place, locked again.
    pub(crate) fn set_aside(&self) -> io::Result<PathBuf> {
        let mut inner = self.inner()?;
        inner.sessions = None;
        // Windows renames no directory with a file open in it.
        inner.lock = None;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut aside = self.dir.clone().into_os_string();
        aside.push(format!(".bad-{secs}"));
        let aside = PathBuf::from(aside);
        fs::rename(&self.dir, &aside)?;
        make_private_dir(&self.dir)?;
        inner.lock = Some(DirLock::acquire(&self.dir, LOCK).map_err(io::Error::other)?);
        let path = self.dir.join(SESSIONS);
        inner.sessions = Some(Sessions::create(Vec::new(), &path).map_err(io::Error::other)?);
        Ok(aside)
    }
}

/// Makes the stored sessions `records`: inserts the new, replaces the
/// changed, deletes the gone; an unchanged session writes nothing.
fn sync(sessions: &mut Sessions, records: Vec<SessionRecord>) -> io::Result<()> {
    let keep: BTreeSet<_> = records.iter().map(|r| r.id).collect();
    for record in records {
        match sessions.get(record.id) {
            Some(old) if old == record => {}
            Some(_) => sessions.replace(record).map_err(|e| match e {
                ReplaceError::NotFound(id) => io::Error::other(format!("session {id} vanished")),
                ReplaceError::Durability(e) => io::Error::other(e),
            })?,
            None => sessions.insert(record).map_err(|e| match e {
                InsertError::Duplicate(id) => io::Error::other(format!("session {id} twice")),
                InsertError::Durability(e) => io::Error::other(e),
            })?,
        }
    }
    let gone: Vec<_> = sessions
        .all_ids()
        .into_iter()
        .filter(|id| !keep.contains(id))
        .collect();
    for id in gone {
        sessions.delete(id).map_err(|e| match e {
            DeleteError::NotFound(id) => io::Error::other(format!("session {id} vanished")),
            DeleteError::Durability(e) => io::Error::other(e),
        })?;
    }
    Ok(())
}

fn warn<T>(block: BlockId, why: &str) -> Option<T> {
    eprintln!(
        "rusty-mux-server: block {block}'s saved screen is unreadable, restarting it blank: {why}"
    );
    None
}

/// A screen file whose block is gone, or a leftover `.tmp`.
fn is_stale(path: &Path, manifest: &Manifest) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.ends_with(".tmp") {
        return true;
    }
    let Some(id) = name.strip_suffix(&format!(".{SCREEN}")) else {
        return false;
    };
    id.parse()
        .map(BlockId::from_uuid)
        .is_ok_and(|block| !manifest.started.contains_key(&block))
}

/// Writes `data` to `path` through a `.tmp` beside it, synced before the
/// rename so a crash leaves the old file or the new one, never half.
fn replace(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = fs::File::create(&tmp)?;
        io::Write::write_all(&mut file, data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(unix)]
fn make_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // An existing directory keeps its mode through `create`; make it ours alone.
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn make_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    platform_windows::fs::restrict_to_owner(dir)
        .map_err(|e| io::Error::other(format!("restricting {}: {e:?}", dir.display())))
}

#[cfg(not(any(unix, windows)))]
fn make_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(test)]
mod tests;
